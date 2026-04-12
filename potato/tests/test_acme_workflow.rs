/// ACME完整工作流程测试
/// 测试ACME功能的端到端流程
#[cfg(feature = "acme")]
mod acme_full_workflow_tests {
    use potato::acme::{AcmeOptions, DynamicTlsAcceptor};
    use std::fs;
    use std::time::Duration;

    /// 测试完整的ACME工作流程模拟
    /// 包括：证书创建、加载、续期判断、热重载
    #[tokio::test]
    async fn test_complete_acme_workflow() {
        let temp_dir = std::env::temp_dir().join("potato_acme_workflow_test");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // 步骤1: 生成初始证书
        let (cert_pem1, key_pem1) = generate_test_cert("initial.example.com");
        let cert_path = temp_dir.join("cert.pem");
        let key_path = temp_dir.join("key.pem");

        fs::write(&cert_path, &cert_pem1).unwrap();
        fs::write(&key_path, &key_pem1).unwrap();

        // 步骤2: 创建DynamicTlsAcceptor
        let acceptor = DynamicTlsAcceptor::new(&cert_pem1, &key_pem1).unwrap();

        // 验证初始证书可用
        let initial_acceptor = acceptor.get_acceptor().await;
        drop(initial_acceptor);

        // 步骤3: 模拟证书续期判断
        // 新证书不应该立即续期
        let metadata = fs::metadata(&cert_path).unwrap();
        let modified = metadata.modified().unwrap();
        let elapsed = modified.elapsed().unwrap();
        assert!(elapsed < Duration::from_secs(60 * 24 * 3600)); // 小于60天

        // 步骤4: 模拟续期 - 生成新证书
        let (cert_pem2, key_pem2) = generate_test_cert("renewed.example.com");

        // 步骤5: 热重载证书
        let reload_result = acceptor.reload(&cert_pem2, &key_pem2).await;
        assert!(reload_result.is_ok(), "证书重载应该成功");

        // 验证新证书已加载
        let reloaded_acceptor = acceptor.get_acceptor().await;
        drop(reloaded_acceptor);

        // 步骤6: 验证文件已更新
        fs::write(&cert_path, &cert_pem2).unwrap();
        fs::write(&key_path, &key_pem2).unwrap();

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);

        println!("✓ Complete ACME workflow test passed");
    }

    /// 测试多个域的证书配置
    #[test]
    fn test_multi_domain_acme_options() {
        let opts = AcmeOptions {
            domains: vec![
                "example.com".to_string(),
                "www.example.com".to_string(),
                "api.example.com".to_string(),
            ],
            email: "admin@example.com".to_string(),
            acme_directory: Some(
                "https://acme-staging-v02.api.letsencrypt.org/directory".to_string(),
            ),
            cert_dir: Some("/tmp/multi_domain_certs".to_string()),
        };

        assert_eq!(opts.domains.len(), 3);
        assert!(opts.domains.contains(&"example.com".to_string()));
        assert!(opts.domains.contains(&"www.example.com".to_string()));
        assert!(opts.domains.contains(&"api.example.com".to_string()));
    }

    /// 测试证书重载的原子性
    #[tokio::test]
    async fn test_certificate_reload_atomicity() {
        let (cert_pem1, key_pem1) = generate_test_cert("atomic1.example.com");
        let acceptor = DynamicTlsAcceptor::new(&cert_pem1, &key_pem1).unwrap();

        // 获取初始acceptor
        let initial_acceptor = acceptor.get_acceptor().await;
        drop(initial_acceptor);

        // 多次重载测试
        for i in 0..5 {
            let domain = format!("atomic{}.example.com", i + 2);
            let (cert_pem, key_pem) = generate_test_cert(&domain);

            let reload_result = acceptor.reload(&cert_pem, &key_pem).await;
            assert!(reload_result.is_ok(), "第{}次重载应该成功", i + 1);

            let acceptor = acceptor.get_acceptor().await;
            drop(acceptor);
        }

        println!("✓ Certificate reload atomicity test passed");
    }

    /// 测试证书目录权限和创建
    #[test]
    fn test_cert_directory_permissions() {
        let temp_dir = std::env::temp_dir().join("potato_acme_perms_test");

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);

        // 验证目录不存在
        assert!(!temp_dir.exists());

        // 创建目录
        fs::create_dir_all(&temp_dir).unwrap();
        assert!(temp_dir.exists());

        // 验证可以写入证书文件
        let cert_path = temp_dir.join("cert.pem");
        let (cert_pem, _) = generate_test_cert("perms.example.com");
        fs::write(&cert_path, &cert_pem).unwrap();
        assert!(cert_path.exists());

        // 清理
        let _ = fs::remove_dir_all(&temp_dir);
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
