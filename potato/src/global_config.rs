use crate::utils::string::StringUtil;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

static SERVER_JWT_SECRET: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new(StringUtil::rand(32)));
static SERVER_WS_PING_DURATION: LazyLock<RwLock<Duration>> =
    LazyLock::new(|| RwLock::new(Duration::from_secs(60)));
static SERVER_MAX_HEADER_COUNT: AtomicUsize = AtomicUsize::new(48);
static SERVER_MAX_HEADER_BYTES: AtomicUsize = AtomicUsize::new(16 * 1024);

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: u64,
}

pub struct ServerConfig;
impl ServerConfig {
    pub async fn set_jwt_secret(secret: impl Into<String>) {
        *SERVER_JWT_SECRET.write().await = secret.into();
    }

    pub async fn get_jwt_secret() -> String {
        SERVER_JWT_SECRET.read().await.clone()
    }

    pub async fn set_ws_ping_duration(dur: Duration) {
        *SERVER_WS_PING_DURATION.write().await = dur;
    }

    pub async fn get_ws_ping_duration() -> Duration {
        SERVER_WS_PING_DURATION.read().await.clone()
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
}

pub struct ServerAuth;
impl ServerAuth {
    pub async fn jwt_issue(payload: String, expire: Duration) -> anyhow::Result<String> {
        let secret = &(*SERVER_JWT_SECRET.read().await)[..];
        let claims = Claims {
            sub: payload,
            exp: (SystemTime::now() + expire)
                .duration_since(UNIX_EPOCH)?
                .as_micros() as u64,
        };
        let header = jsonwebtoken::Header::default();
        let key = jsonwebtoken::EncodingKey::from_secret(secret.as_bytes());
        Ok(jsonwebtoken::encode(&header, &claims, &key)?)
    }

    pub async fn jwt_check(token: &str) -> anyhow::Result<String> {
        let secret = &(*SERVER_JWT_SECRET.read().await)[..];
        let decoding_key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
        let validation = jsonwebtoken::Validation::default();
        let claims = jsonwebtoken::decode::<Claims>(token, &decoding_key, &validation)?.claims;
        let expired = SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(claims.exp);
        match SystemTime::now() <= expired {
            true => Ok(claims.sub),
            false => Err(anyhow::Error::msg("token expired")),
        }
    }
}
