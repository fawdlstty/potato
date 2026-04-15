//! WebRTC DataChannel示例
//!
//! 演示如何使用WebRTC DataChannel进行数据传输
//!
//! 运行方式:
//! ```bash
//! cargo run --features webrtc --example 31_webrtc_datachannel
//! ```

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("WebRTC DataChannel 示例");
    println!("注意: 需要先启动WebRTC服务器");
    println!();

    // 创建WebRTC客户端
    let mut client = potato::webrtc::WebRTCClient::new(
        "ws://127.0.0.1:8080/ws",
        vec!["stun:stun.l.google.com:19302".to_string()],
    )
    .await?;

    // 进入房间
    client.enter_room("chat_room").await?;
    println!("已加入房间: chat_room");

    // 创建DataChannel
    let chat = client.chat("chat").await?;
    println!("DataChannel 已创建: {}", chat.label());

    // 设置消息回调
    chat.on_message(|msg| {
        if msg.is_string {
            println!("收到消息: {}", String::from_utf8_lossy(&msg.data));
        } else {
            println!("收到二进制数据: {} 字节", msg.data.len());
        }
    });

    // 发送一条消息
    chat.send("大家好!").await?;
    println!("已发送消息");

    println!("按 Ctrl+C 退出");
    tokio::signal::ctrl_c().await?;

    // 离开房间
    client.exit_room().await?;

    Ok(())
}
