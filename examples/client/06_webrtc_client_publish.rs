//! WebRTC客户端推流示例
//!
//! 演示如何使用WebRTC客户端进行推流
//!
//! 运行方式:
//! ```bash
//! cargo run --features webrtc --example 29_webrtc_client_publish
//! ```

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("WebRTC 客户端推流示例");
    println!("注意: 需要先启动WebRTC服务器 (cargo run --features webrtc --example 28_webrtc_server)");
    println!();

    // 创建WebRTC客户端
    let mut client = potato::webrtc::WebRTCClient::new(
        "ws://127.0.0.1:8080/ws",
        vec!["stun:stun.l.google.com:19302".to_string()],
    )
    .await?;

    // 进入房间
    client.enter_room("test_room").await?;
    println!("已加入房间: test_room");

    // 开始推流
    client.publish().await?;
    println!("推流已启动");

    println!("按 Ctrl+C 停止推流");
    tokio::signal::ctrl_c().await?;

    // 离开房间
    client.exit_room().await?;
    println!("已离开房间");

    Ok(())
}
