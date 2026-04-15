//! WebRTC SFU服务器示例
//!
//! 演示如何使用potato的WebRTC功能
//!
//! 运行方式:
//! ```bash
//! cargo run --features webrtc --example 28_webrtc_server
//! ```

use potato::webrtc::{WebRTCSFU, WebRTCConfig, WebRtcSignalingHandler};
use std::sync::Arc;

// WebSocket信令处理函数
#[potato::http_get("/ws")]
async fn webrtc_websocket(req: &mut potato::HttpRequest) -> anyhow::Result<()> {
    // 注意：在实际使用中，需要从全局状态获取SFU实例
    // 这里仅作为示例
    let config = WebRTCConfig::default();
    let sfu = Arc::new(WebRTCSFU::new(config));
    let handler = WebRtcSignalingHandler::new(sfu);
    handler.handle_websocket(req).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 最简用法:2行代码启动完整WebRTC SFU
    let mut server = potato::HttpServer::new("0.0.0.0:8080");

    server.configure(|ctx| {
        // 一行代码启用完整 WebRTC 功能
        ctx.use_webrtc().finish();
    });

    println!("WebRTC SFU 启动:");
    println!("  WebSocket 信令: ws://127.0.0.1:8080/ws");
    println!("  REST API:      http://127.0.0.1:8080/api/webrtc");
    println!("  媒体流:         UDP 50000-60000");
    println!();
    println!("按 Ctrl+C 退出");

    server.serve_http().await
}
