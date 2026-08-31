use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

static SERVER_JWT_SECRET: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));
static SERVER_WS_PING_DURATION: LazyLock<RwLock<Duration>> =
    LazyLock::new(|| RwLock::new(Duration::from_secs(60)));
static SERVER_MAX_HEADER_COUNT: AtomicUsize = AtomicUsize::new(48);
static SERVER_MAX_HEADER_BYTES: AtomicUsize = AtomicUsize::new(16 * 1024);
static SERVER_MAX_BODY_BYTES: AtomicUsize = AtomicUsize::new(100 * 1024 * 1024); // 100MB default

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: u64,
}

pub struct ServerConfig;
impl ServerConfig {
    pub async fn set_jwt_secret(secret: impl Into<String>) {
        let secret = secret.into();
        assert!(!secret.is_empty(), "JWT secret must not be empty");
        *SERVER_JWT_SECRET.write().await = Some(secret);
    }

    pub(crate) async fn jwt_secret() -> anyhow::Result<String> {
        SERVER_JWT_SECRET
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("JWT secret is not initialized"))
    }

    pub async fn set_ws_ping_duration(dur: Duration) {
        *SERVER_WS_PING_DURATION.write().await = dur;
    }

    pub async fn get_ws_ping_duration() -> Duration {
        *SERVER_WS_PING_DURATION.read().await
    }

    pub fn set_max_header_count(limit: usize) {
        SERVER_MAX_HEADER_COUNT.store(limit.max(1), Ordering::Relaxed);
    }

    pub fn get_max_header_count() -> usize {
        SERVER_MAX_HEADER_COUNT.load(Ordering::Relaxed)
    }

    pub fn set_max_header_bytes(limit: usize) {
        SERVER_MAX_HEADER_BYTES.store(limit.max(1), Ordering::Relaxed);
    }

    pub fn get_max_header_bytes() -> usize {
        SERVER_MAX_HEADER_BYTES.load(Ordering::Relaxed)
    }

    pub fn set_max_body_bytes(limit: usize) {
        SERVER_MAX_BODY_BYTES.store(limit.max(1), Ordering::Relaxed);
    }

    pub fn get_max_body_bytes() -> usize {
        SERVER_MAX_BODY_BYTES.load(Ordering::Relaxed)
    }
}

pub struct ServerAuth;
impl ServerAuth {
    pub async fn jwt_issue(payload: String, expire: Duration) -> anyhow::Result<String> {
        let secret = ServerConfig::jwt_secret().await?;
        let secret = &secret[..];
        let claims = Claims {
            sub: payload,
            exp: (SystemTime::now() + expire)
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
        };
        let header = jsonwebtoken::Header::default();
        let key = jsonwebtoken::EncodingKey::from_secret(secret.as_bytes());
        Ok(jsonwebtoken::encode(&header, &claims, &key)?)
    }

    pub async fn jwt_check(token: &str) -> anyhow::Result<String> {
        let secret = ServerConfig::jwt_secret().await?;
        let secret = &secret[..];
        let decoding_key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
        let validation = jsonwebtoken::Validation::default();
        let claims = jsonwebtoken::decode::<Claims>(token, &decoding_key, &validation)?.claims;
        let expired = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(claims.exp);
        match SystemTime::now() <= expired {
            true => Ok(claims.sub),
            false => Err(anyhow::Error::msg("token expired")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    #[tokio::test]
    async fn jwt_exp_uses_rfc7519_seconds() -> anyhow::Result<()> {
        ServerConfig::set_jwt_secret("jwt-numericdate-test").await;
        let token =
            ServerAuth::jwt_issue("test-user".to_string(), Duration::from_secs(3600)).await?;

        let mut validation = Validation {
            validate_exp: false,
            ..Default::default()
        };
        validation.required_spec_claims.clear();
        let claims = decode::<Claims>(
            &token,
            &DecodingKey::from_secret("jwt-numericdate-test".as_bytes()),
            &validation,
        )?
        .claims;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let exp_delta = i128::from(claims.exp) - i128::from(now);
        assert!(
            (3_590..=3_610).contains(&exp_delta),
            "JWT exp should use NumericDate seconds, got delta {exp_delta}"
        );
        assert_eq!(ServerAuth::jwt_check(&token).await?, "test-user");
        Ok(())
    }
}
