#![cfg(all(feature = "std", feature = "webtransport"))]
//! WebTransport 功能测试

#[cfg(feature = "webtransport")]
mod tests {
    #[tokio::test]
    async fn test_webtransport_session_creation() {
        // 测试会话创建 - 这里只是基本结构测试
        // 完整的集成测试需要真实的 QUIC 连接
        println!("WebTransport session structure test");
    }

    #[test]
    fn webtransport_api_is_experimental() {}
}
