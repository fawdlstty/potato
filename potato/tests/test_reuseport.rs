#![cfg(feature = "std")]
/// 端口复用（set_reuseport）特性测试：
/// - 默认禁用：未启用时同端口第二个监听必须失败
/// - 启用后：SO_REUSEPORT 允许两个监听并行共存（平滑重启语义）
/// - Windows：set_reuseport(true) 直接报错（SO_REUSEPORT 不存在于 Windows）
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use potato::HttpServer;
#[cfg(all(
    unix,
    not(target_os = "solaris"),
    not(target_os = "illumos"),
    not(target_os = "cygwin"),
    not(target_os = "nuttx"),
))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

static PORT_COUNTER: AtomicU16 = AtomicU16::new(33000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// 启动 server，返回任务句柄与 shutdown 发送端（发送端保活期间 server 持续运行）
fn spawn_server(
    mut srv: HttpServer,
) -> anyhow::Result<(
    tokio::task::JoinHandle<anyhow::Result<()>>,
    tokio::sync::oneshot::Sender<()>,
)> {
    let shutdown = srv.shutdown_signal()?;
    let task = tokio::spawn(async move { srv.serve_http().await });
    Ok((task, shutdown))
}

/// 轮询连接直到 listener 就绪
async fn wait_server_ready(addr: &str) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("server not ready within 5s: {addr}");
        }
        if TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
}

/// 默认禁用：未调用 set_reuseport 时，同端口第二个监听必须 bind 失败
#[tokio::test]
async fn reuse_port_default_disabled() -> anyhow::Result<()> {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");

    let s1 = HttpServer::new(addr.clone());
    let (_t1, _guard1) = spawn_server(s1)?;
    wait_server_ready(&addr).await?;

    let s2 = HttpServer::new(addr);
    let (t2, _guard2) = spawn_server(s2)?;
    let res2 = timeout(Duration::from_secs(5), t2).await??;
    assert!(
        res2.is_err(),
        "expected second bind to fail without reuseport, got: {res2:?}"
    );
    Ok(())
}

/// 启用后：两个监听并行共存，连接持续可建立
#[cfg(all(
    unix,
    not(target_os = "solaris"),
    not(target_os = "illumos"),
    not(target_os = "cygwin"),
    not(target_os = "nuttx"),
))]
#[tokio::test]
async fn reuse_port_allows_parallel_listeners() -> anyhow::Result<()> {
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");

    let mut s1 = HttpServer::new(addr.clone());
    s1.set_reuseport(true)?;
    let (t1, _guard1) = spawn_server(s1)?;
    wait_server_ready(&addr).await?;

    let mut s2 = HttpServer::new(addr.clone());
    s2.set_reuseport(true)?;
    let (t2, _guard2) = spawn_server(s2)?;
    wait_server_ready(&addr).await?;
    // 两个任务都还活着（没有因 bind 失败退出）
    assert!(!t1.is_finished(), "first server exited unexpectedly");
    assert!(!t2.is_finished(), "second server exited unexpectedly");

    // 内核把连接按四元组哈希到其中一个监听，连续建连都应成功并收到响应
    for _ in 0..5 {
        let mut stream = TcpStream::connect(&addr).await?;
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .await?;
        let mut buf = Vec::new();
        timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await??;
        assert!(!buf.is_empty(), "empty response from server");
    }
    Ok(())
}

/// 开关语义：默认禁用，可反复启用/禁用（仅 SO_REUSEPORT 支持平台；
/// Windows 上启用会报错，见平台专属测试）
#[cfg(all(
    unix,
    not(target_os = "solaris"),
    not(target_os = "illumos"),
    not(target_os = "cygwin"),
    not(target_os = "nuttx"),
))]
#[tokio::test]
async fn set_reuseport_toggle() -> anyhow::Result<()> {
    let mut srv = HttpServer::new(format!("127.0.0.1:{}", get_test_port()));
    // 默认禁用：不调用 set_reuseport 直接 serve 即默认行为
    assert!(srv.set_reuseport(true).is_ok());
    assert!(srv.set_reuseport(false).is_ok());
    assert!(srv.set_reuseport(true).is_ok());
    Ok(())
}

/// Windows：set_reuseport(true) 直接报错（该平台无 SO_REUSEPORT）
#[cfg(windows)]
#[tokio::test]
async fn reuse_port_rejected_on_windows() -> anyhow::Result<()> {
    let mut srv = HttpServer::new(format!("127.0.0.1:{}", get_test_port()));
    assert!(srv.set_reuseport(true).is_err());
    // 禁用方向永远合法，跨平台代码无需按平台分支
    assert!(srv.set_reuseport(false).is_ok());
    Ok(())
}
