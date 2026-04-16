//! ACME自动证书管理模块
//!
//! 提供Let's Encrypt等ACME协议的自动证书申请、续期和热重载功能

#![cfg(feature = "acme")]

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_rustls::rustls;
use tokio_rustls::TlsAcceptor;
use x509_parser::prelude::*;

/// ACME配置选项
pub struct AcmeOptions {
    /// 域名列表
    pub domains: Vec<String>,
    /// 联系邮箱
    pub email: String,
    /// ACME目录URL（默认使用Let's Encrypt生产环境）
    pub acme_directory: Option<String>,
    /// 证书缓存目录（默认 "./acme_certs"）
    pub cert_dir: Option<String>,
}

impl AcmeOptions {
    pub fn new(domain: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            domains: vec![domain.into()],
            email: email.into(),
            acme_directory: None,
            cert_dir: None,
        }
    }
}

/// TLS证书状态
#[allow(dead_code)]
struct TlsCertState {
    cert_pem: String,
    key_pem: String,
    acceptor: TlsAcceptor,
}

/// 动态TLS接受器，支持热重载
#[derive(Clone)]
pub struct DynamicTlsAcceptor {
    state: Arc<RwLock<Arc<TlsCertState>>>,
}

impl DynamicTlsAcceptor {
    pub fn new(cert_pem: &str, key_pem: &str) -> anyhow::Result<Self> {
        let acceptor = Self::create_acceptor(cert_pem, key_pem)?;
        let state = Arc::new(TlsCertState {
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
            acceptor,
        });
        Ok(Self {
            state: Arc::new(RwLock::new(state)),
        })
    }

    fn create_acceptor(cert_pem: &str, key_pem: &str) -> anyhow::Result<TlsAcceptor> {
        // 初始化 rustls CryptoProvider（如果尚未初始化）
        {
            use rustls::crypto::ring::default_provider;
            use rustls::crypto::CryptoProvider;
            let _ = CryptoProvider::install_default(default_provider());
        }

        let certs =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())?;

        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        // 支持HTTP/2和HTTP/1.1
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    pub async fn reload(&self, cert_pem: &str, key_pem: &str) -> anyhow::Result<()> {
        let new_acceptor = Self::create_acceptor(cert_pem, key_pem)?;
        let new_state = Arc::new(TlsCertState {
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
            acceptor: new_acceptor,
        });
        *self.state.write().await = new_state;
        Ok(())
    }

    pub async fn get_acceptor(&self) -> TlsAcceptor {
        self.state.read().await.acceptor.clone()
    }
}

/// ACME挑战信息
#[derive(Clone)]
pub struct AcmeChallenge {
    pub token: String,
    pub key_authorization: String,
}

/// ACME管理器
#[derive(Clone)]
pub struct AcmeManager {
    account: Account,
    domains: Vec<String>,
    cert_dir: String,
    challenges: Arc<RwLock<Vec<AcmeChallenge>>>,
}

impl AcmeManager {
    /// 创建或加载ACME账户
    pub async fn new(opts: AcmeOptions) -> anyhow::Result<(Self, DynamicTlsAcceptor)> {
        let cert_dir = opts
            .cert_dir
            .clone()
            .unwrap_or_else(|| "./acme_certs".to_string());
        let email = opts.email.clone();
        let domains = opts.domains.clone();
        let acme_directory = opts.acme_directory.clone();

        std::fs::create_dir_all(&cert_dir)?;

        let account_path = format!("{cert_dir}/account.json");

        // 尝试加载已有账户
        let account = if let Ok(creds_str) = std::fs::read_to_string(&account_path) {
            if let Ok(creds) = serde_json::from_str::<AccountCredentials>(&creds_str) {
                Account::builder()?.from_credentials(creds).await?
            } else {
                Self::create_account(&email, &acme_directory, &account_path).await?
            }
        } else {
            Self::create_account(&email, &acme_directory, &account_path).await?
        };

        let manager = Self {
            account,
            domains,
            cert_dir: cert_dir.clone(),
            challenges: Arc::new(RwLock::new(Vec::new())),
        };

        // 尝试加载已有证书
        let cert_dir_clone = cert_dir.clone();
        let cert_path = format!("{cert_dir_clone}/cert.pem");
        let key_path = format!("{cert_dir_clone}/key.pem");

        let acceptor = if std::path::Path::new(&cert_path).exists()
            && std::path::Path::new(&key_path).exists()
        {
            let cert_pem = std::fs::read_to_string(&cert_path)?;
            let key_pem = std::fs::read_to_string(&key_path)?;
            DynamicTlsAcceptor::new(&cert_pem, &key_pem)?
        } else {
            // 申请新证书
            let (cert_pem, key_pem) = manager.obtain_certificate().await?;
            DynamicTlsAcceptor::new(&cert_pem, &key_pem)?
        };

        Ok((manager, acceptor))
    }

    async fn create_account(
        email: &str,
        acme_directory: &Option<String>,
        account_path: &str,
    ) -> anyhow::Result<Account> {
        let dir_url = acme_directory
            .clone()
            .unwrap_or_else(|| LetsEncrypt::Production.url().to_string());

        let (account, credentials) = Account::builder()?
            .create(
                &NewAccount {
                    contact: &[&format!("mailto:{email}")],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                dir_url,
                None,
            )
            .await?;

        // 保存账户凭据
        let creds_str = serde_json::to_string_pretty(&credentials)?;
        std::fs::write(account_path, creds_str)?;

        Ok(account)
    }

    /// 获取ACME挑战列表（供HTTP服务器使用）
    pub async fn get_challenges(&self) -> Vec<AcmeChallenge> {
        self.challenges.read().await.clone()
    }

    /// 申请证书（HTTP-01验证）
    async fn obtain_certificate(&self) -> anyhow::Result<(String, String)> {
        println!(
            "[ACME] Starting certificate obtainment for domains: {:?}",
            self.domains
        );

        let identifiers: Vec<Identifier> = self
            .domains
            .iter()
            .map(|d| Identifier::Dns(d.clone()))
            .collect();

        let mut order = self
            .account
            .new_order(&NewOrder::new(identifiers.as_slice()))
            .await?;

        println!("[ACME] Order created, processing authorizations...");

        // 处理授权挑战
        let mut authorizations = order.authorizations();
        let mut challenges = Vec::new();

        while let Some(result) = authorizations.next().await {
            let mut authz = result?;
            println!(
                "[ACME] Authorization status: {:?} for {:?}",
                authz.status,
                authz.identifier()
            );

            if matches!(authz.status, AuthorizationStatus::Valid) {
                println!("[ACME] Authorization already valid, skipping challenge");
                continue;
            }

            let challenge = authz
                .challenge(ChallengeType::Http01)
                .ok_or_else(|| anyhow::anyhow!("no http-01 challenge found"))?;

            let key_auth = challenge.key_authorization();
            println!("[ACME] Challenge token: {}", challenge.token);

            challenges.push(AcmeChallenge {
                token: challenge.token.clone(),
                key_authorization: key_auth.as_str().to_string(),
            });
        }

        // 保存挑战信息供HTTP服务器使用
        // 注意：这里会更新challenges，HTTP服务器会看到新的挑战
        println!(
            "[ACME] Saving {} challenges for HTTP server",
            challenges.len()
        );
        *self.challenges.write().await = challenges.clone();

        // 标记挑战就绪（通知ACME服务器开始验证）
        // 必须在保存challenges之后立即执行，以便HTTP服务器可以响应ACME验证请求
        for challenge in &challenges {
            println!("[ACME] Setting challenge ready: {}", challenge.token);
            let mut authorizations = order.authorizations();
            while let Some(result) = authorizations.next().await {
                let mut authz = result?;
                if let Some(mut ch) = authz.challenge(ChallengeType::Http01) {
                    if ch.token == challenge.token {
                        ch.set_ready().await?;
                        println!("[ACME] Challenge set to ready");
                    }
                }
            }
        }

        // 等待ACME服务器验证完成
        // 注意：这里不再需要额外的sleep，因为set_ready后ACME服务器会立即开始验证
        // HTTP服务器已经在上一步保存challenges时可以响应验证请求了

        // 等待订单就绪
        println!("[ACME] Waiting for order to be ready...");
        let status = order.poll_ready(&RetryPolicy::default()).await?;
        if status != OrderStatus::Ready {
            return Err(anyhow::anyhow!("unexpected order status: {:?}", status));
        }

        // 完成订单并获取证书
        println!("[ACME] Order ready, finalizing...");
        let private_key_pem = order.finalize().await?;
        let cert_chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;

        // 保存证书到文件
        let cert_path = format!("{}/cert.pem", self.cert_dir);
        let key_path = format!("{}/key.pem", self.cert_dir);
        std::fs::write(&cert_path, &cert_chain_pem)?;
        std::fs::write(&key_path, &private_key_pem)?;

        println!("[ACME] Certificate obtained and saved successfully");
        Ok((cert_chain_pem, private_key_pem))
    }

    /// 启动后台续期循环
    /// 注意：由于AcmeManager使用Arc<RwLock>存储challenges，
    /// 续签时更新的challenges会被HTTP服务器看到
    pub async fn start_renewal_loop(self, acceptor: DynamicTlsAcceptor) -> anyhow::Result<()> {
        loop {
            // 每6小时检查一次
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;

            // 检查证书是否即将过期（30天内）
            let cert_path = format!("{}/cert.pem", self.cert_dir);
            if let Ok(_cert_pem) = std::fs::read_to_string(&cert_path) {
                if Self::should_renew(&cert_path) {
                    println!("[ACME] Certificate expiring soon, renewing...");
                    match self.obtain_certificate().await {
                        Ok((cert_pem, key_pem)) => {
                            if let Err(e) = acceptor.reload(&cert_pem, &key_pem).await {
                                eprintln!("[ACME] Failed to reload certificate: {e}");
                            } else {
                                println!("[ACME] Certificate renewed successfully");
                            }
                        }
                        Err(e) => {
                            eprintln!("[ACME] Failed to renew certificate: {e}");
                        }
                    }
                } else {
                    println!("[ACME] Certificate still valid, next check in 6 hours");
                }
            } else {
                // 证书文件不存在，尝试申请
                println!("[ACME] Certificate file not found, applying for new certificate...");
                match self.obtain_certificate().await {
                    Ok((cert_pem, key_pem)) => {
                        if let Err(e) = acceptor.reload(&cert_pem, &key_pem).await {
                            eprintln!("[ACME] Failed to reload certificate: {e}");
                        } else {
                            println!("[ACME] Certificate obtained and loaded successfully");
                        }
                    }
                    Err(e) => {
                        eprintln!("[ACME] Failed to obtain certificate: {e}");
                    }
                }
            }
        }
    }

    /// 检查证书是否应该续期（基于证书实际过期时间）
    fn should_renew(cert_path: &str) -> bool {
        if let Ok(cert_pem) = std::fs::read_to_string(cert_path) {
            // 解析证书获取过期时间
            if let Ok(certs) =
                CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()
            {
                if let Some(cert) = certs.first() {
                    // 使用ASN1解析证书的not_after字段
                    // CertificateDer内部是DER编码的X.509证书
                    // 我们需要解析ASN1结构来获取过期时间
                    return Self::parse_cert_expiry(cert).map_or(false, |expiry| {
                        let now = std::time::SystemTime::now();
                        let thirty_days = Duration::from_secs(30 * 24 * 3600);

                        // 如果证书将在30天内过期，或者已经过期，则需要续期
                        match expiry.duration_since(now) {
                            Ok(duration) => duration < thirty_days,
                            Err(_) => true, // 已经过期
                        }
                    });
                }
            }
        }

        // 后备方案：基于文件修改时间
        if let Ok(metadata) = std::fs::metadata(cert_path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    // 如果证书创建超过60天（Let's Encrypt证书90天有效期，提前30天续期）
                    return elapsed > Duration::from_secs(60 * 24 * 3600);
                }
            }
        }
        false
    }

    /// 解析证书的过期时间
    fn parse_cert_expiry(cert: &CertificateDer<'_>) -> Option<std::time::SystemTime> {
        // 使用x509-parser解析证书
        match X509Certificate::from_der(cert.as_ref()) {
            Ok((_, parsed_cert)) => {
                // 获取validity字段的not_after
                let validity = parsed_cert.validity();
                let not_after = validity.not_after;

                // 将ASN1时间转换为SystemTime
                let timestamp = not_after.to_datetime();
                let secs = timestamp.unix_timestamp();
                let nanos = timestamp.nanosecond();

                Some(
                    std::time::SystemTime::UNIX_EPOCH
                        + Duration::from_secs(secs as u64)
                        + Duration::from_nanos(nanos as u64),
                )
            }
            Err(_) => None,
        }
    }
}
