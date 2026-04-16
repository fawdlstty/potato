use potato::{HttpResponse, HttpServer};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(26100);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[potato::http_get("/test")]
async fn test_handler() -> HttpResponse {
    // 返回一个较大的响应体以测试速率限制
    let large_data = "x".repeat(100_000); // 100KB
    HttpResponse::text(large_data)
}

async fn connect_with_retry(addr: &str) -> anyhow::Result<TcpStream> {
    let mut last_err = None;
    for _ in 0..10 {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                last_err = Some(err);
                sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    Err(last_err.expect("retry loop must capture error").into())
}

#[tokio::test]
async fn test_transfer_rate_limit() -> anyhow::Result<()> {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&addr);

    server.configure(|ctx| {
        // 入站 10 Mbps，出站 1 Mbps 速率限制
        ctx.use_transfer_limit(10_000_000, 1_000_000);
        ctx.use_handlers();
    });

    // 在后台启动服务器
    let server_handle = tokio::spawn(async move { server.serve_http().await });

    // 给服务器一些启动时间
    sleep(std::time::Duration::from_millis(200)).await;

    // 发送请求并测量时间
    let start = Instant::now();
    let mut stream = connect_with_retry(&addr).await?;

    let request = "GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;

    // 读取响应
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;

    let elapsed = start.elapsed();

    // 找到body部分
    if let Some(pos) = response.windows(4).position(|w| w == b"\r\n\r\n") {
        let body = &response[pos + 4..];
        println!("Received {} bytes in {:?}", body.len(), elapsed);

        // 100KB = 800,000 bits
        // 在 1 Mbps 限制下，应该至少需要 0.8 秒
        // 我们允许一些误差，检查是否至少花了 0.3 秒
        assert!(
            elapsed.as_millis() > 300,
            "Rate limiting should slow down transfer. Elapsed: {:?}",
            elapsed
        );
    }

    // 停止服务器
    server_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_no_rate_limit() -> anyhow::Result<()> {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&addr);

    server.configure(|ctx| {
        ctx.use_handlers();
    });

    let server_handle = tokio::spawn(async move { server.serve_http().await });

    sleep(std::time::Duration::from_millis(200)).await;

    // 发送请求并测量时间
    let start = Instant::now();
    let mut stream = connect_with_retry(&addr).await?;

    let request = "GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;

    // 读取响应
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;

    let elapsed = start.elapsed();

    if let Some(pos) = response.windows(4).position(|w| w == b"\r\n\r\n") {
        let body = &response[pos + 4..];
        println!("Received {} bytes in {:?}", body.len(), elapsed);

        // 无速率限制应该很快完成（小于 1 秒）
        assert!(
            elapsed.as_millis() < 1000,
            "Without rate limiting, transfer should be fast. Elapsed: {:?}",
            elapsed
        );
    }

    server_handle.abort();

    Ok(())
}
