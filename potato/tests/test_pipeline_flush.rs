#![cfg(feature = "std")]
//! 流水线写序安全回归测试（先解析后 flush 重构的行为锚点）：
//! 1. 待发响应必须在下一次可能阻塞的读之前冲出——否则客户端发半截请求
//!    等响应先到即死锁（flush-before-read 红线）；
//! 2. 单连接多请求批合并顺序与响应完整性。

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

static PORT_COUNTER: AtomicU16 = AtomicU16::new(36000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[potato::http_get("/pf/hello")]
async fn pf_hello() -> HttpResponse {
    HttpResponse::text("pf-hello-body")
}

use potato::{HttpResponse, HttpServer};

async fn start_server() -> anyhow::Result<String> {
    let port = get_test_port();
    let server_addr = format!("127.0.0.1:{port}");
    let mut server = HttpServer::new(&server_addr);
    let _ = server.configure(|ctx| {
        ctx.use_handlers();
    });
    let _server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });
    sleep(Duration::from_millis(300)).await;
    Ok(server_addr)
}

fn hello_request() -> Vec<u8> {
    b"GET /pf/hello HTTP/1.1\r\nHost: pf.test\r\n\r\n".to_vec()
}

/// 红线锚点：连接上先发一个完整请求 + 下一个请求的前半截（不完整），
/// 待发响应必须在阻塞读剩余字节之前冲出，客户端不等补发请求尾就能
/// 收到第一个响应。冲刷缺失时本测试在读第一响应处超时失败。
#[tokio::test]
async fn flush_pending_before_blocking_read() -> anyhow::Result<()> {
    let server_addr = start_server().await?;
    let mut stream = TcpStream::connect(&server_addr).await?;

    // 请求 1 完整 + 请求 2 前半截（无 \r\n\r\n，解析必 Partial）
    let mut first_write = hello_request();
    first_write.extend_from_slice(b"GET /pf/hello HTTP/1.1\r\nHo");
    stream.write_all(&first_write).await?;

    // 第一个响应必须独立到达（不依赖请求 2 补齐）
    let mut buf = vec![0u8; 4096];
    let n = timeout(Duration::from_secs(3), stream.read(&mut buf)).await??;
    assert!(n > 0, "connection closed before first response");
    let first = String::from_utf8_lossy(&buf[..n]).to_string();
    assert!(
        first.starts_with("HTTP/1.1 200 OK"),
        "unexpected first response: {first:?}"
    );
    assert!(first.contains("pf-hello-body"));

    // 补齐请求 2，收第二个响应
    stream.write_all(b"st: pf.test\r\n\r\n").await?;
    let n = timeout(Duration::from_secs(3), stream.read(&mut buf)).await??;
    assert!(n > 0, "connection closed before second response");
    let second = String::from_utf8_lossy(&buf[..n]).to_string();
    assert!(
        second.starts_with("HTTP/1.1 200 OK") && second.contains("pf-hello-body"),
        "unexpected second response: {second:?}"
    );
    Ok(())
}

/// 批合并顺序：一条连接一次写入 3 个完整请求，按序收回 3 个响应。
#[tokio::test]
async fn pipelined_requests_keep_order() -> anyhow::Result<()> {
    let server_addr = start_server().await?;
    let mut stream = TcpStream::connect(&server_addr).await?;

    let mut burst = Vec::new();
    for _ in 0..3 {
        burst.extend_from_slice(&hello_request());
    }
    stream.write_all(&burst).await?;

    // 收满 3 个响应（读循环聚合，容忍任意拆包）
    let deadline = Duration::from_secs(3);
    let mut received = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = timeout(deadline, stream.read(&mut buf)).await??;
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&received);
        if text.matches("HTTP/1.1 200 OK").count() >= 3
            && text.matches("pf-hello-body").count() >= 3
        {
            break;
        }
    }
    let text = String::from_utf8_lossy(&received).to_string();
    assert_eq!(
        text.matches("HTTP/1.1 200 OK").count(),
        3,
        "expected 3 responses, got: {text:?}"
    );
    assert_eq!(text.matches("pf-hello-body").count(), 3);
    Ok(())
}
