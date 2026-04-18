#[cfg(feature = "http3")]
use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "http3")]
use std::time::Duration;
#[cfg(feature = "http3")]
use tokio::time::sleep;

#[cfg(feature = "http3")]
static PORT_COUNTER: AtomicU16 = AtomicU16::new(29500);

#[cfg(feature = "http3")]
fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(feature = "http3")]
fn create_test_cert_files() -> anyhow::Result<(
    String,
    String,
    tokio_rustls::rustls::pki_types::CertificateDer<'static>,
)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    let temp_dir = std::env::temp_dir().join(format!("potato_http3_test_{}", get_test_port()));
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

#[cfg(all(test, feature = "http3"))]
mod http3_tests {
    use super::*;
    use bytes::Buf;
    use h3_quinn::quinn;
    use std::sync::Arc;

    #[potato::http_get("/http3_native")]
    async fn http3_native(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("h3-ok")
    }

    #[tokio::test]
    async fn test_serve_http3_accepts_http3_requests() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");
        let (cert_file, key_file, cert_der) = create_test_cert_files()?;

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3(&cert_file, &key_file).await;
        });
        // HTTP3服务器需要更长的启动时间
        sleep(Duration::from_millis(2000)).await;

        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        roots.add(cert_der.clone())?;
        let mut tls_config = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        ));
        endpoint.set_default_client_config(client_config);

        // 添加连接超时
        let connecting = endpoint
            .connect(addr.parse()?, "localhost")
            .map_err(|e| anyhow::anyhow!("quic connect failed: {e}"))?;
        let quic_conn = tokio::time::timeout(Duration::from_secs(10), connecting)
            .await
            .map_err(|e| anyhow::anyhow!("quic connect timeout: {e}"))?
            .map_err(|e| anyhow::anyhow!("quic connect failed: {e}"))?;

        let (mut driver, mut send_request) = h3::client::new(h3_quinn::Connection::new(quic_conn))
            .await
            .map_err(|e| anyhow::anyhow!("h3 client init failed: {e}"))?;
        let driver_handle = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        let req = http::Request::builder()
            .method("GET")
            .uri("https://localhost/http3_native")
            .body(())?;
        let mut stream = send_request
            .send_request(req)
            .await
            .map_err(|e| anyhow::anyhow!("h3 send_request failed: {e}"))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow::anyhow!("h3 request finish failed: {e}"))?;

        let resp = stream
            .recv_response()
            .await
            .map_err(|e| anyhow::anyhow!("h3 recv_response failed: {e}"))?;
        assert_eq!(resp.status(), http::StatusCode::OK);
        let mut body = Vec::new();
        while let Some(mut chunk) = stream.recv_data().await? {
            body.extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()));
        }
        assert_eq!(body, b"h3-ok".to_vec());

        driver_handle.abort();
        endpoint.wait_idle().await;
        server_handle.abort();
        Ok(())
    }
}
