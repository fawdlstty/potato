//! WebTransport 客户端示例
//!
//! 演示如何使用 potato 的 WebTransport 客户端
//!
//! 运行方式:
//! ```bash
//! cargo run --features http3 --example 33_webtransport_client
//! ```

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 使用宏连接 WebTransport 服务器（最简洁）
    let wt = potato::webtransport!("https://127.0.0.1:4433/wt").await?;

    println!("WebTransport 连接已建立");

    // 发送数据报
    println!("发送数据报...");
    wt.send_datagram(b"Hello from client!").await?;

    // 打开双向流
    println!("打开双向流...");
    let mut stream = wt.open_bi().await?;

    // 发送数据
    stream.send(b"Hello via stream!").await?;
    println!("已发送流数据");

    // 接收响应
    match stream.recv().await {
        Ok(Some(data)) => {
            println!("收到响应: {:?}", String::from_utf8_lossy(&data));
        }
        Ok(None) => {
            println!("流已关闭，无响应");
        }
        Err(e) => {
            eprintln!("接收响应失败: {}", e);
        }
    }

    println!("测试完成");
    Ok(())
}
