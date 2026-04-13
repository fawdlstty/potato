//! WebRTC Features编译测试
//!
//! 验证webrtc feature可以正确编译

#[test]
fn test_webrtc_feature_compilation() {
    // 此测试验证webrtc feature被正确启用
    // 实际的功能测试需要完整的WebRTC环境

    #[cfg(feature = "webrtc")]
    {
        // 验证webrtc模块存在
        // 这些类型在webrtc feature启用时应该可用
        use potato::webrtc::WebRTCConfig;
        use potato::webrtc::WebRTCSFU;

        // 验证配置可以创建
        let config = WebRTCConfig::default();
        assert_eq!(config.max_peers, 100);
        assert_eq!(config.udp_port_start, 50000);
        assert_eq!(config.udp_port_end, 60000);

        // 验证SFU可以创建
        let _sfu = WebRTCSFU::new(config);
    }

    #[cfg(not(feature = "webrtc"))]
    {
        // 如果没有启用webrtc feature,测试应该跳过
        println!("WebRTC feature not enabled, skipping test");
    }
}

#[test]
fn test_webrtc_config_defaults() {
    #[cfg(feature = "webrtc")]
    {
        use potato::webrtc::WebRTCConfig;

        let config = WebRTCConfig::default();

        // 验证默认配置
        assert_eq!(config.max_peers, 100);
        assert!(config
            .ice_servers
            .contains(&"stun:stun.l.google.com:19302".to_string()));
        assert_eq!(config.udp_port_start, 50000);
        assert_eq!(config.udp_port_end, 60000);
        assert!(config.enable_datachannel);
        assert_eq!(config.ws_path, "/ws");
        assert_eq!(config.rest_prefix, "/api/webrtc");
        assert!(config.auto_reconnect);
        assert_eq!(config.log_level, "info");
    }
}
