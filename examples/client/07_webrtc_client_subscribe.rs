//! WebRTC客户端拉流示例
//!
//! 演示如何使用WebRTC客户端进行拉流
//!
//! 运行方式:
//! ```bash
//! cargo run --features webrtc --example 30_webrtc_client_subscribe
//! ```

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("WebRTC 客户端拉流示例");
    println!("注意: 需要先启动WebRTC服务器并有推流者");
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

    // 查看房间内有哪些用户
    println!("房间内的用户: {:?}", client.room.peers);

    // 订阅推流者的视频流
    if let Some(publisher) = client.room.peers.iter().find(|p| p.is_publishing) {
        println!("发现推流者: {}", publisher.id);
        
        // 订阅视频流
        let mut video_subscription = client.subscribe(&publisher.id).video().await?;
        println!("成功订阅 {} 的视频流", publisher.id);
        
        // 接收视频帧
        tokio::spawn(async move {
            let mut frame_count = 0;
            while let Some(frame) = video_subscription.recv().await {
                frame_count += 1;
                if frame.is_keyframe {
                    println!("[视频帧 #{frame_count}] 关键帧: {} 字节, 时间戳: {}", 
                        frame.data.len(), frame.timestamp);
                } else if frame_count % 30 == 0 {
                    println!("[视频帧 #{frame_count}] 收到 {} 字节", frame.data.len());
                }
            }
            println!("视频流结束，共收到 {frame_count} 帧");
        });
        
        // 订阅音频流
        let mut audio_subscription = client.subscribe(&publisher.id).audio().await?;
        println!("成功订阅 {} 的音频流", publisher.id);
        
        tokio::spawn(async move {
            let mut audio_count = 0;
            while let Some(audio) = audio_subscription.recv().await {
                audio_count += 1;
                if audio_count % 100 == 0 {
                    println!("[音频包 #{audio_count}] 收到 {} 字节", audio.data.len());
                }
            }
            println!("音频流结束，共收到 {audio_count} 包");
        });
    } else {
        println!("房间内没有推流者");
    }

    println!("按 Ctrl+C 退出");
    tokio::signal::ctrl_c().await?;

    // 离开房间
    client.exit_room().await?;

    Ok(())
}
