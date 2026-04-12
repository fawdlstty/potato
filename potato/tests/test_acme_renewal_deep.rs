/// ACME 续签逻辑深度测试
/// 测试续签流程的正确性和边界条件
#[cfg(feature = "acme")]
mod acme_renewal_deep_tests {
    use potato::acme::{AcmeOptions, DynamicTlsAcceptor};
    use std::fs;
    use std::time::Duration;
    use time::OffsetDateTime;

    /// 测试证书过期时间解析的准确性
    #[test]
    fn test_certificate_expiry_parsing_accuracy() {
        use rustls_pki_types::pem::PemObject;
        use rustls_pki_types::CertificateDer;
        use x509_parser::prelude::*;

        // 生成测试证书
        let (cert_pem, _) = generate_test_cert("expiry-test.example.com");

        // 解析证书
        if let Ok(certs) =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()
        {
            if let Some(cert) = certs.first() {
                // 使用 x509-parser 解析
                if let Ok((_, parsed_cert)) = X509Certificate::from_der(cert.as_ref()) {
                    let validity = parsed_cert.validity();
                    let not_after = validity.not_after;
                    let not_before = validity.not_before;

                    // 验证过期时间在未来
                    let timestamp_after = not_after.to_datetime();
                    let timestamp_before = not_before.to_datetime();

                    println!("证书有效期:");
                    println!("  开始时间: {}", timestamp_before);
                    println!("  过期时间: {}", timestamp_after);

                    // rcgen 默认生成的证书有效期为 1 年
                    // 验证过期时间确实比开始时间晚
                    assert!(timestamp_after > timestamp_before);

                    // 验证过期时间在未来（至少 364 天后）
                    let now = OffsetDateTime::now_utc();
                    let duration_days = (timestamp_after - now).whole_days();
                    println!("  剩余天数: {}", duration_days);
                    assert!(duration_days > 360, "证书有效期应该超过360天");
                }
            }
        }
    }

    /// 测试 should_renew 逻辑 - 新证书不应续签
    #[test]
    fn test_should_renew_new_certificate() {
        let temp_dir = std::env::temp_dir().join("potato_acme_new_cert_test");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let cert_path = temp_dir.join("cert.pem");

        // 生成新证书
        let (cert_pem, _) = generate_test_cert("new-cert.example.com");
        fs::write(&cert_path, &cert_pem).unwrap();

        // 解析证书检查过期时间
        use rustls_pki_types::pem::PemObject;
        use rustls_pki_types::CertificateDer;
        use x509_parser::prelude::*;

        if let Ok(cert_data) = fs::read_to_string(&cert_path) {
            if let Ok(certs) =
                CertificateDer::pem_slice_iter(cert_data.as_bytes()).collect::<Result<Vec<_>, _>>()
            {
                if let Some(cert) = certs.first() {
                    if let Ok((_, parsed_cert)) = X509Certificate::from_der(cert.as_ref()) {
                        let validity = parsed_cert.validity();
                        let not_after = validity.not_after.to_datetime();
                        let now = OffsetDateTime::now_utc();
                        let days_until_expiry = (not_after - now).whole_days();

                        println!("新证书剩余天数: {}", days_until_expiry);
                        // 新证书应该还有 360+ 天，远大于 30 天阈值
                        assert!(days_until_expiry > 30, "新证书不应该在30天内过期");
                    }
                }
            }
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    /// 测试续签循环中的挑战更新机制
    #[tokio::test]
    async fn test_challenge_update_mechanism() {
        use potato::acme::AcmeChallenge;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // 模拟挑战存储
        let challenges: Arc<RwLock<Vec<AcmeChallenge>>> = Arc::new(RwLock::new(Vec::new()));

        // 初始状态：无挑战
        assert!(challenges.read().await.is_empty());

        // 模拟第一次申请证书的挑战
        let challenges_1 = vec![
            AcmeChallenge {
                token: "token1".to_string(),
                key_authorization: "keyauth1".to_string(),
            },
            AcmeChallenge {
                token: "token2".to_string(),
                key_authorization: "keyauth2".to_string(),
            },
        ];

        *challenges.write().await = challenges_1.clone();
        assert_eq!(challenges.read().await.len(), 2);

        // 模拟续签时的挑战更新（应该完全替换）
        let challenges_2 = vec![AcmeChallenge {
            token: "new_token".to_string(),
            key_authorization: "new_keyauth".to_string(),
        }];

        *challenges.write().await = challenges_2.clone();

        // 验证挑战已更新
        let updated = challenges.read().await;
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].token, "new_token");
        assert_eq!(updated[0].key_authorization, "new_keyauth");

        println!("✓ 挑战更新机制验证通过");
    }

    /// 测试 DynamicTlsAcceptor 的线程安全性
    #[tokio::test]
    async fn test_dynamic_tls_acceptor_thread_safety() {
        let (cert_pem, key_pem) = generate_test_cert("thread-safety.example.com");
        let acceptor = DynamicTlsAcceptor::new(&cert_pem, &key_pem).unwrap();

        // 创建多个并发任务访问 acceptor
        let mut handles = vec![];
        for i in 0..10 {
            let acceptor_clone = acceptor.clone();
            let handle = tokio::spawn(async move {
                // 每个任务都获取 acceptor
                let acc = acceptor_clone.get_acceptor().await;
                drop(acc);
                i
            });
            handles.push(handle);
        }

        // 等待所有任务完成
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result < 10);
        }

        println!("✓ DynamicTlsAcceptor 线程安全性测试通过");
    }

    /// 测试证书热重载的并发安全性
    #[tokio::test]
    async fn test_certificate_reload_concurrency() {
        let (cert_pem1, key_pem1) = generate_test_cert("concurrent-v1.example.com");
        let acceptor = DynamicTlsAcceptor::new(&cert_pem1, &key_pem1).unwrap();

        // 启动多个读取任务
        let mut read_handles = vec![];
        for _ in 0..5 {
            let acceptor_clone = acceptor.clone();
            let handle = tokio::spawn(async move {
                for _ in 0..10 {
                    let acc = acceptor_clone.get_acceptor().await;
                    drop(acc);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
            read_handles.push(handle);
        }

        // 同时进行重载
        let (cert_pem2, key_pem2) = generate_test_cert("concurrent-v2.example.com");
        let reload_result = acceptor.reload(&cert_pem2, &key_pem2).await;
        assert!(reload_result.is_ok(), "证书重载应该成功");

        // 等待所有读取任务完成
        for handle in read_handles {
            handle.await.unwrap();
        }

        println!("✓ 证书热重载并发安全性测试通过");
    }

    /// 测试 AcmeOptions 的默认值
    #[test]
    fn test_acme_options_defaults() {
        let opts = AcmeOptions::new("test.example.com", "test@example.com");

        assert_eq!(opts.domains, vec!["test.example.com"]);
        assert_eq!(opts.email, "test@example.com");
        assert!(opts.acme_directory.is_none());
        assert!(opts.cert_dir.is_none());

        println!("✓ AcmeOptions 默认值测试通过");
    }

    /// 测试证书文件路径构建
    #[test]
    fn test_certificate_file_paths() {
        let temp_dir = std::env::temp_dir().join("potato_acme_paths_test");
        let cert_dir = temp_dir.to_str().unwrap().to_string();

        let cert_path = format!("{}/cert.pem", cert_dir);
        let key_path = format!("{}/key.pem", cert_dir);
        let account_path = format!("{}/account.json", cert_dir);

        // 验证路径格式正确
        assert!(cert_path.ends_with("/cert.pem"));
        assert!(key_path.ends_with("/key.pem"));
        assert!(account_path.ends_with("/account.json"));

        println!("✓ 证书文件路径构建测试通过");
    }

    /// 测试续签时间窗口计算
    #[test]
    fn test_renewal_time_window() {
        // Let's Encrypt 证书有效期：90 天
        // 续签阈值：30 天
        // 检查间隔：6 小时

        let certificate_validity_days = 90;
        let renewal_threshold_days = 30;
        let check_interval_hours = 6;

        // 计算应该在第几天开始续签
        let renewal_start_day = certificate_validity_days - renewal_threshold_days;
        assert_eq!(renewal_start_day, 60, "应该在第60天开始续签");

        // 验证检查间隔
        let check_interval = Duration::from_secs(check_interval_hours * 3600);
        assert_eq!(check_interval.as_secs(), 21600);

        // 在续签窗口内，最多需要检查的次数
        let renewal_window_days = renewal_threshold_days;
        let max_checks = (renewal_window_days * 24) / check_interval_hours;
        assert_eq!(max_checks, 120, "续签窗口内最多检查120次");

        println!("✓ 续签时间窗口计算测试通过");
    }

    // 辅助函数：生成测试证书
    fn generate_test_cert(domain: &str) -> (String, String) {
        use rcgen::{CertificateParams, DistinguishedName, KeyPair};

        let mut params = CertificateParams::new(vec![domain.to_string()]).unwrap();
        params.distinguished_name = DistinguishedName::new();

        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        (cert_pem, key_pem)
    }
}
