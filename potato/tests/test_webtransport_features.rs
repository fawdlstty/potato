//! WebTransport 功能测试

#[cfg(feature = "http3")]
mod tests {
    use potato::WebTransportSession;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_webtransport_session_creation() {
        // 测试会话创建 - 这里只是基本结构测试
        // 完整的集成测试需要真实的 QUIC 连接
        println!("WebTransport session structure test");
    }

    #[tokio::test]
    async fn test_webtransport_config_default() {
        use potato::server::WebTransportConfig;

        let config = WebTransportConfig::default();
        assert_eq!(config.max_sessions, 1000);
        assert_eq!(config.max_streams_per_session, 100);
        assert!(config.datagram_enabled);
        assert_eq!(config.max_datagram_size, 1200);
    }
}
