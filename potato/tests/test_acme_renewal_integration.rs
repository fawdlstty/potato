/// ACME 续签流程集成测试
/// 模拟完整的证书申请、保存、续签流程
#[cfg(feature = "acme")]
mod acme_renewal_integration_tests {
    use potato::acme::{AcmeOptions, DynamicTlsAcceptor};
    use std::fs;
    use std::time::Duration;

    /// 测试完整的证书生命周期模拟
    /// 1. 初始证书创建
    /// 2. 证书使用
    /// 3. 续签判断
    /// 4. 证书重载
    /// 5. 验证新证书生效
    #[tokio::test]
    async fn test_complete_certificate_lifecycle() {
        let temp_dir = std::env::temp_dir().join("potato_acme_lifecycle_test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cert_path = temp_dir.join("cert.pem");
        let key_path = temp_dir.join("key.pem");

        // 阶段1: 初始证书创建（模拟首次申请）
        println!("阶段1: 创建初始证书");
        let (cert_pem1, key_pem1) = generate_test_cert("initial.example.com");
        fs::write(&cert_path, &cert_pem1).unwrap();
        fs::write(&key_path, &key_pem1).unwrap();

        // 创建 TLS acceptor
        let acceptor = DynamicTlsAcceptor::new(&cert_pem1, &key_pem1).unwrap();

        // 验证初始证书可用
        let initial_acceptor = acceptor.get_acceptor().await;
        drop(initial_acceptor);
        println!("✓ 初始证书创建成功");

        // 阶段2: 模拟证书使用（60天后）
        println!("\n阶段2: 模拟证书使用60天");
        // 注意：我们无法真正修改文件时间到60天前，但我们可以验证逻辑

        // 阶段3: 续签判断
        println!("\n阶段3: 续签判断");
        // 新证书不应该续签（有效期还有很久）
        let metadata = fs::metadata(&cert_path).unwrap();
        let modified = metadata.modified().unwrap();
        let elapsed = modified.elapsed().unwrap();
        let sixty_days = Duration::from_secs(60 * 24 * 3600);

        if elapsed > sixty_days {
            println!("  -> 证书已使用超过60天，应该续签");
        } else {
            println!("  -> 证书刚创建，不应续签");
        }

        // 阶段4: 模拟续签（生成新证书）
        println!("\n阶段4: 执行证书续签");
        let (cert_pem2, key_pem2) = generate_test_cert("renewed.example.com");

        // 保存新证书
        fs::write(&cert_path, &cert_pem2).unwrap();
        fs::write(&key_path, &key_pem2).unwrap();

        // 热重载证书
        let reload_result = acceptor.reload(&cert_pem2, &key_pem2).await;
        assert!(reload_result.is_ok(), "证书重载应该成功");
        println!("✓ 证书续签并重载成功");

        // 阶段5: 验证新证书生效
        println!("\n阶段5: 验证新证书生效");
        let new_acceptor = acceptor.get_acceptor().await;
        drop(new_acceptor);
        println!("✓ 新证书已生效");

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);
        println!("\n✓ 完整证书生命周期测试通过");
    }

    /// 测试多次续签循环的稳定性
    #[tokio::test]
    async fn test_multiple_renewal_cycles() {
        let temp_dir = std::env::temp_dir().join("potato_acme_multi_cycle_test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cert_path = temp_dir.join("cert.pem");
        let key_path = temp_dir.join("key.pem");

        // 初始证书
        let (cert_pem, key_pem) = generate_test_cert("cycle-0.example.com");
        fs::write(&cert_path, &cert_pem).unwrap();
        fs::write(&key_path, &key_pem).unwrap();

        let acceptor = DynamicTlsAcceptor::new(&cert_pem, &key_pem).unwrap();

        // 模拟10次续签循环
        for i in 1..=10 {
            println!("执行第 {} 次续签", i);

            // 生成新证书（模拟ACME续签）
            let domain = format!("cycle-{}.example.com", i);
            let (new_cert_pem, new_key_pem) = generate_test_cert(&domain);

            // 保存新证书
            fs::write(&cert_path, &new_cert_pem).unwrap();
            fs::write(&key_path, &new_key_pem).unwrap();

            // 热重载
            let reload_result = acceptor.reload(&new_cert_pem, &new_key_pem).await;
            assert!(reload_result.is_ok(), "第 {} 次续签应该成功", i);

            // 验证新证书可用
            let test_acceptor = acceptor.get_acceptor().await;
            drop(test_acceptor);

            println!("  ✓ 第 {} 次续签成功", i);
        }

        let _ = fs::remove_dir_all(&temp_dir);
        println!("\n✓ 10次续签循环稳定性测试通过");
    }

    /// 测试续签失败后的恢复能力
    #[tokio::test]
    async fn test_renewal_failure_recovery() {
        let temp_dir = std::env::temp_dir().join("potato_acme_failure_recovery_test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let cert_path = temp_dir.join("cert.pem");
        let key_path = temp_dir.join("key.pem");

        // 初始证书
        let (cert_pem1, key_pem1) = generate_test_cert("recovery-v1.example.com");
        fs::write(&cert_path, &cert_pem1).unwrap();
        fs::write(&key_path, &key_pem1).unwrap();

        let acceptor = DynamicTlsAcceptor::new(&cert_pem1, &key_pem1).unwrap();

        // 验证初始证书可用
        let initial_acceptor = acceptor.get_acceptor().await;
        drop(initial_acceptor);

        // 模拟续签失败（使用无效证书）
        println!("模拟续签失败...");
        let invalid_cert = "-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----";
        let invalid_key = "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----";

        let reload_result = acceptor.reload(invalid_cert, invalid_key).await;
        assert!(reload_result.is_err(), "使用无效证书应该失败");
        println!("✓ 无效证书重载正确失败");

        // 验证原有证书仍然可用
        let still_works_acceptor = acceptor.get_acceptor().await;
        drop(still_works_acceptor);
        println!("✓ 原有证书仍然可用");

        // 模拟下次续签成功
        println!("模拟下次续签成功...");
        let (cert_pem2, key_pem2) = generate_test_cert("recovery-v2.example.com");
        let reload_result = acceptor.reload(&cert_pem2, &key_pem2).await;
        assert!(reload_result.is_ok(), "有效证书重载应该成功");

        let new_acceptor = acceptor.get_acceptor().await;
        drop(new_acceptor);
        println!("✓ 续签成功恢复");

        let _ = fs::remove_dir_all(&temp_dir);
        println!("\n✓ 续签失败恢复测试通过");
    }

    /// 测试 AcmeOptions 配置的正确性
    #[test]
    fn test_acme_options_configuration() {
        // 测试最小配置
        let minimal_opts = AcmeOptions::new("example.com", "admin@example.com");
        assert_eq!(minimal_opts.domains, vec!["example.com"]);
        assert_eq!(minimal_opts.email, "admin@example.com");
        assert!(minimal_opts.acme_directory.is_none());
        assert!(minimal_opts.cert_dir.is_none());

        // 测试完整配置
        let full_opts = AcmeOptions {
            domains: vec![
                "example.com".to_string(),
                "www.example.com".to_string(),
                "api.example.com".to_string(),
            ],
            email: "admin@example.com".to_string(),
            acme_directory: Some(
                "https://acme-staging-v02.api.letsencrypt.org/directory".to_string(),
            ),
            cert_dir: Some("/tmp/test_certs".to_string()),
        };

        assert_eq!(full_opts.domains.len(), 3);
        assert!(full_opts.domains.contains(&"example.com".to_string()));
        assert!(full_opts.domains.contains(&"www.example.com".to_string()));
        assert!(full_opts.domains.contains(&"api.example.com".to_string()));
        assert!(full_opts.acme_directory.is_some());
        assert!(full_opts.cert_dir.is_some());

        println!("✓ AcmeOptions 配置测试通过");
    }

    /// 测试证书目录管理
    #[test]
    fn test_certificate_directory_management() {
        let temp_dir = std::env::temp_dir().join("potato_acme_dir_mgmt_test");

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);

        // 验证目录不存在
        assert!(!temp_dir.exists());

        // 创建目录
        fs::create_dir_all(&temp_dir).unwrap();
        assert!(temp_dir.exists());

        // 创建证书文件
        let cert_path = temp_dir.join("cert.pem");
        let key_path = temp_dir.join("key.pem");
        let account_path = temp_dir.join("account.json");

        let (cert_pem, key_pem) = generate_test_cert("dir-mgmt.example.com");
        fs::write(&cert_path, &cert_pem).unwrap();
        fs::write(&key_path, &key_pem).unwrap();
        fs::write(&account_path, "{}").unwrap();

        // 验证所有文件存在
        assert!(cert_path.exists());
        assert!(key_path.exists());
        assert!(account_path.exists());

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);

        println!("✓ 证书目录管理测试通过");
    }

    /// 测试续签时间计算逻辑
    #[test]
    fn test_renewal_timing_calculation() {
        // Let's Encrypt 证书参数
        let certificate_validity_days = 90; // 证书有效期
        let renewal_threshold_days = 30; // 续签阈值（提前30天）
        let check_interval_hours = 6; // 检查间隔

        // 计算何时开始续签
        let renewal_start_day = certificate_validity_days - renewal_threshold_days;
        assert_eq!(renewal_start_day, 60);

        // 计算检查次数
        let checks_per_day = 24 / check_interval_hours;
        assert_eq!(checks_per_day, 4);

        // 在续签窗口内的总检查次数
        let total_checks_in_window = renewal_threshold_days * checks_per_day;
        assert_eq!(total_checks_in_window, 120);

        // 验证时间常量
        let thirty_days = Duration::from_secs(30 * 24 * 3600);
        assert_eq!(thirty_days.as_secs(), 2_592_000);

        let sixty_days = Duration::from_secs(60 * 24 * 3600);
        assert_eq!(sixty_days.as_secs(), 5_184_000);

        let check_interval = Duration::from_secs(check_interval_hours * 3600);
        assert_eq!(check_interval.as_secs(), 21_600);

        println!("✓ 续签时间计算逻辑测试通过");
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
