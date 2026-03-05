use potato::{HttpResponse, HttpServer};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("127.0.0.1:3000");

    server.configure(|ctx| {
        ctx.use_custom(|req| {
            Box::pin(async move {
                if req.url_path == "/stream" {
                    // 创建一个 channel 用于流式传输
                    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);

                    // 启动一个任务来生成数据
                    tokio::spawn(async move {
                        let mut interval = interval(Duration::from_millis(500));
                        for i in 0..10 {
                            interval.tick().await;
                            let data = format!("Message {}\\n", i).into_bytes();
                            if tx.send(data).await.is_err() {
                                break;
                            }
                        }
                    });

                    // 创建流式响应
                    Ok(Some(HttpResponse::stream(rx)))
                } else if req.url_path == "/sse" {
                    // Server-Sent Events 示例
                    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);

                    tokio::spawn(async move {
                        let mut interval = interval(Duration::from_millis(1000));
                        for i in 0..5 {
                            interval.tick().await;
                            // SSE 格式：data: <message>\\n\\n
                            let sse_data = format!("data: Event {}\\n\\n", i);
                            if tx.send(sse_data.into_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });

                    let mut resp = HttpResponse::stream(rx);
                    resp.add_header("Content-Type", "text/event-stream");
                    resp.add_header("Cache-Control", "no-cache");
                    Ok(Some(resp))
                } else {
                    Ok(None)
                }
            })
        });
    });

    println!("Server starting on http://127.0.0.1:3000");
    println!("Test endpoints:");
    println!("  - http://127.0.0.1:3000/stream (chunked stream)");
    println!("  - http://127.0.0.1:3000/sse (Server-Sent Events)");

    server.serve_http().await?;

    Ok(())
}
