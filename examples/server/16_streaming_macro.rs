use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

/// 流式传输示例 - 直接返回 Receiver<Vec<u8>>
#[potato::http_get("/stream")]
async fn stream_handler() -> tokio::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(500));
        for i in 0..10 {
            interval.tick().await;
            let data = format!("Stream message {}\n", i).into_bytes();
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });

    rx
}

/// 流式传输示例 - 返回 Result<Receiver<Vec<u8>>>
#[potato::http_get("/stream-result")]
async fn stream_result_handler() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(1000));
        for i in 0..5 {
            interval.tick().await;
            let data = format!("Result stream message {}\n", i).into_bytes();
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });

    Ok(rx)
}

/// SSE (Server-Sent Events) 示例
#[potato::http_get("/sse")]
async fn sse_handler() -> tokio::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(1000));
        for i in 0..5 {
            interval.tick().await;
            // SSE 格式：data: <message>\n\n
            let sse_data = format!("data: Event {}\n\n", i);
            if tx.send(sse_data.into_bytes()).await.is_err() {
                break;
            }
        }
    });

    rx
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");

    server.configure(|ctx| {
        ctx.use_handlers(true);
    });

    println!("Server starting on http://127.0.0.1:3000");
    println!("Test endpoints:");
    println!("  - http://127.0.0.1:3000/stream (direct Receiver return)");
    println!("  - http://127.0.0.1:3000/stream-result (Result<Receiver> return)");
    println!("  - http://127.0.0.1:3000/sse (Server-Sent Events)");

    server.serve_http().await?;

    Ok(())
}
