#![cfg(all(feature = "std", feature = "tls"))]
//! 每核 current-thread 拓扑（POTATO_PER_CORE=1）行为测试：
//! http1 / https / http2(ALPN 回落) 三入口在每核拓扑下可正常服务，
//! 停机信号触发后 serve future 能在宽限期内返回（worker 随停机退出）。

#[cfg(feature = "http2")]
use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "tls")]
use std::time::Duration;
#[cfg(feature = "tls")]
use tokio::time::sleep;

#[cfg(feature = "http2")]
static PORT_COUNTER: AtomicU16 = AtomicU16::new(35500);

#[cfg(feature = "http2")]
fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(feature = "tls")]
fn enable_per_core() {
    std::env::set_var("POTATO_PER_CORE", "1");
    // 测试进程核数不可控，钉成 2 个 worker 控制资源占用
    std::env::set_var("POTATO_PER_CORE_THREADS", "2");
}

#[cfg(feature = "tls")]
fn create_test_cert_files() -> anyhow::Result<(
    String,
    String,
    tokio_rustls::rustls::pki_types::CertificateDer<'static>,
)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    let temp_dir = std::env::temp_dir().join(format!("potato_per_core_test_{}", get_test_port()));
    std::fs::create_dir_all(&temp_dir)?;

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");
    std::fs::write(&cert_path, cert_pem)?;
    std::fs::write(&key_path, key_pem)?;

    Ok((
        cert_path.to_string_lossy().to_string(),
        key_path.to_string_lossy().to_string(),
        cert.cert.der().clone(),
    ))
}

#[cfg(feature = "tls")]
async fn connect_tls_with_alpn(
    addr: &str,
    cert_der: &tokio_rustls::rustls::pki_types::CertificateDer<'static>,
    alpn: Vec<Vec<u8>>,
) -> anyhow::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    use tokio::net::TcpStream;
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    // 测试二进制可能同时启用 ring 与 aws-lc-rs，进程级 provider 无法自动
    // 判定；与服务端同款显式安装（已安装时返回 Err，忽略即可）
    {
        use tokio_rustls::rustls::crypto::CryptoProvider;
        let _ = CryptoProvider::install_default(rustls::crypto::ring::default_provider());
    }

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der.clone())?;
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn;
    let connector = TlsConnector::from(std::sync::Arc::new(config));
    let stream = TcpStream::connect(addr).await?;
    let server_name = ServerName::try_from("localhost")?.to_owned();
    Ok(connector.connect(server_name, stream).await?)
}

#[cfg(feature = "tls")]
async fn read_all_ignore_tls_close_notify(
    stream: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
) -> anyhow::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let mut out = Vec::new();
    let mut buf = [0_u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(err) => {
                // rustls may report missing close_notify after the payload has already been read.
                if err.to_string().contains("close_notify") {
                    break;
                }
                return Err(err.into());
            }
        }
    }
    Ok(out)
}

/// 反复尝试 TCP 连接直到服务端监听就绪（per-core worker 异步绑定）
#[cfg(feature = "tls")]
async fn wait_tcp_ready(addr: &str) -> anyhow::Result<tokio::net::TcpStream> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(e.into());
                }
                sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

#[cfg(feature = "tls")]
#[potato::http_get("/per_core_ping")]
async fn per_core_ping() -> potato::HttpResponse {
    potato::HttpResponse::text("per-core-ok")
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn test_per_core_http1_serves_requests() -> anyhow::Result<()> {
    enable_per_core();
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");

    let mut server = potato::HttpServer::new(&addr);
    server.set_shutdown_grace(Duration::from_millis(200));
    let server_handle = tokio::spawn(async move { server.serve_http().await });

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = wait_tcp_ready(&addr).await?;
    stream
        .write_all(b"GET /per_core_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "got: {text}");
    assert!(text.contains("per-core-ok"), "got: {text}");

    server_handle.abort();
    Ok(())
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn test_per_core_https_serves_requests() -> anyhow::Result<()> {
    enable_per_core();
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");
    let (cert_file, key_file, cert) = create_test_cert_files()?;

    let mut server = potato::HttpServer::new(&addr);
    server.set_shutdown_grace(Duration::from_millis(200));
    let server_handle = tokio::spawn(async move {
        let _ = server.serve_https(&cert_file, &key_file).await;
    });

    use tokio::io::AsyncWriteExt;
    // 就绪探测：TLS 握手成功即监听已就绪
    let mut tls_stream = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match connect_tls_with_alpn(&addr, &cert, vec![]).await {
                Ok(s) => break s,
                Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        anyhow::bail!("per-core https server not ready in 5s");
                    }
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    };
    tls_stream
        .write_all(b"GET /per_core_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let response = read_all_ignore_tls_close_notify(&mut tls_stream).await?;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "got: {text}");
    assert!(text.contains("per-core-ok"), "got: {text}");

    server_handle.abort();
    Ok(())
}

#[cfg(feature = "http2")]
#[tokio::test]
async fn test_per_core_http2_serves_h1_fallback() -> anyhow::Result<()> {
    enable_per_core();
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");
    let (cert_file, key_file, cert) = create_test_cert_files()?;

    let mut server = potato::HttpServer::new(&addr);
    server.set_shutdown_grace(Duration::from_millis(200));
    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http2(&cert_file, &key_file).await;
    });

    use tokio::io::AsyncWriteExt;
    let mut tls_stream = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match connect_tls_with_alpn(&addr, &cert, vec![b"http/1.1".to_vec()]).await {
                Ok(s) => break s,
                Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        anyhow::bail!("per-core http2 server not ready in 5s");
                    }
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    };
    tls_stream
        .write_all(b"GET /per_core_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let response = read_all_ignore_tls_close_notify(&mut tls_stream).await?;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "got: {text}");
    assert!(text.contains("per-core-ok"), "got: {text}");

    server_handle.abort();
    Ok(())
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn test_per_core_shutdown_signal_stops_serving() -> anyhow::Result<()> {
    enable_per_core();
    let port = get_test_port();
    let addr = format!("127.0.0.1:{port}");

    let mut server = potato::HttpServer::new(&addr);
    server.set_shutdown_grace(Duration::from_millis(200));
    let tx = server.shutdown_signal()?;
    let server_handle = tokio::spawn(async move { server.serve_http().await });

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = wait_tcp_ready(&addr).await?;
    stream
        .write_all(b"GET /per_core_ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 200 OK"));

    // 停机信号 → serve future 必须返回 Ok（worker 随停机退出，不孤儿悬挂）
    tx.send(()).expect("shutdown sender");
    let result = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(
        matches!(result, Ok(Ok(Ok(())))),
        "serve_http must return Ok(()) on shutdown signal, got {result:?}"
    );
    Ok(())
}
