//! WebTransport 服务器示例
//!
//! 演示如何使用 potato 的 WebTransport 功能
//!
//! 运行方式:
//! ```bash
//! cargo run --features http3 --example 32_webtransport_server
//! ```
//!
//! 注意: WebTransport 需要 HTTP/3 (QUIC)，因此需要 TLS 证书
//! 可以使用自签名证书进行测试

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:4433");

    server.configure(|ctx| {
        // 一行代码启用 WebTransport
        ctx.use_webtransport("/wt", |session| async move {
            println!("新 WebTransport 会话: {:?}", session.remote_addr());

            // 处理双向流
            loop {
                match session.accept_bi().await {
                    Ok(Some(stream)) => {
                        tokio::spawn(handle_bi_stream(stream));
                    }
                    Ok(None) => {
                        println!("会话结束");
                        break;
                    }
                    Err(e) => {
                        eprintln!("接受流失败: {}", e);
                        break;
                    }
                }
            }
        });
    });

    println!("WebTransport 服务器启动:");
    println!("  端点: https://127.0.0.1:4433/wt");
    println!("  协议: HTTP/3 (QUIC)");
    println!();
    println!("注意: 需要提供 TLS 证书文件 (cert.pem 和 key.pem)");
    println!("按 Ctrl+C 退出");

    // WebTransport 需要 HTTP/3 模式
    server.serve_http3("cert.pem", "key.pem").await
}

/// 处理双向流
async fn handle_bi_stream(mut stream: potato::WebTransportStream) {
    println!("新双向流");

    // 简单回显: 接收数据并发送回去
    loop {
        match stream.recv().await {
            Ok(data) => {
                if data.is_empty() {
                    println!("流关闭");
                    break;
                }
                println!("收到数据: {:?}", String::from_utf8_lossy(&data));

                // 回显数据
                if let Err(e) = stream.send(&data).await {
                    eprintln!("发送数据失败: {}", e);
                    break;
                }
            }
            Err(e) => {
                eprintln!("接收数据失败: {}", e);
                break;
            }
        }
    }
}
