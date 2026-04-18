#![cfg(feature = "http3")]

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(30000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(all(test, feature = "http3"))]
mod http3_without_encrypt_tests {
    use super::*;
    use bytes::Buf;
    use h3_quinn::quinn;
    use std::sync::Arc;

    #[potato::http_get("/http3_test")]
    async fn http3_test(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("h3-ok")
    }

    #[potato::http_post("/http3_post")]
    async fn http3_post(req: &mut potato::HttpRequest) -> potato::HttpResponse {
        let body = String::from_utf8_lossy(&req.body).to_string();
        potato::HttpResponse::text(format!("echo: {}", body))
    }

    /// 测试 HTTP/3 无加密模式 (http:// URL)
    #[tokio::test]
    async fn test_http3_without_encrypt_basic() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3_without_encrypt().await;
        });

        // 等待服务器启动（HTTP3需要更长的启动时间）
        sleep(Duration::from_millis(2000)).await;

        // 使用 http:// URL,自动使用无加密模式
        let url = format!("http://localhost:{}/http3_test", port);
        let mut res = potato::client::http3::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = String::from_utf8(res.body.data().await.to_vec())?;
        assert_eq!(body, "h3-ok");

        server_handle.abort();
        Ok(())
    }

    /// 测试 POST 请求 (无加密模式)
    #[tokio::test]
    async fn test_http3_without_encrypt_post() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3_without_encrypt().await;
        });

        sleep(Duration::from_millis(2000)).await;

        // 使用 http:// URL 进行 POST
        let url = format!("http://localhost:{}/http3_post", port);
        let mut res = potato::client::http3::post(&url, b"hello http3".to_vec(), vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = String::from_utf8(res.body.data().await.to_vec())?;
        assert_eq!(body, "echo: hello http3");

        server_handle.abort();
        Ok(())
    }

    /// 测试使用底层 QUIC 连接（不通过 potato 宏）
    #[tokio::test]
    async fn test_http3_without_encrypt_raw_quic() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3_without_encrypt().await;
        });

        // HTTP3服务器需要更长的启动时间
        sleep(Duration::from_millis(2000)).await;

        // 创建不使用证书验证的客户端
        let mut tls_config = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerification))
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        ));
        endpoint.set_default_client_config(client_config);

        // 连接到服务器，添加超时
        let connecting = endpoint
            .connect(addr.parse()?, "localhost")
            .map_err(|e| anyhow::anyhow!("quic connect failed: {e}"))?;
        let quic_conn = tokio::time::timeout(Duration::from_secs(10), connecting)
            .await
            .map_err(|e| anyhow::anyhow!("quic connect timeout: {e}"))?
            .map_err(|e| anyhow::anyhow!("quic connect failed: {e}"))?;

        // 初始化 HTTP/3 客户端
        let (mut driver, mut send_request) = h3::client::new(h3_quinn::Connection::new(quic_conn))
            .await
            .map_err(|e| anyhow::anyhow!("h3 client init failed: {e}"))?;
        let driver_handle = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        // 发送 HTTP/3 请求
        let req = http::Request::builder()
            .method("GET")
            .uri(format!("https://localhost:{}/http3_test", port))
            .body(())?;

        let mut stream = send_request
            .send_request(req)
            .await
            .map_err(|e| anyhow::anyhow!("h3 send_request failed: {e}"))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow::anyhow!("h3 request finish failed: {e}"))?;

        // 接收响应
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

    /// 测试多个并发请求
    #[tokio::test]
    async fn test_http3_without_encrypt_concurrent() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3_without_encrypt().await;
        });

        sleep(Duration::from_millis(2000)).await;

        // 并发发送多个请求
        let mut handles = vec![];
        for _i in 0..5 {
            let port = port;
            let handle = tokio::spawn(async move {
                let url = format!("http://localhost:{}/http3_test", port);
                let mut res = potato::client::http3::get(&url, vec![]).await?;
                assert_eq!(res.http_code, 200);
                let body = String::from_utf8(res.body.data().await.to_vec())?;
                assert_eq!(body, "h3-ok");
                Ok::<_, anyhow::Error>(())
            });
            handles.push(handle);
        }

        // 等待所有请求完成
        for handle in handles {
            handle.await??;
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试 Session 复用
    #[tokio::test]
    async fn test_http3_without_encrypt_session_reuse() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http3_without_encrypt().await;
        });

        sleep(Duration::from_millis(2000)).await;

        // 使用 Session 复用连接
        let mut session = potato::client::http3::H3Session::new();

        let res1 = session
            .get(&format!("http://localhost:{}/http3_test", port), vec![])
            .await?;
        assert_eq!(res1.http_code, 200);

        let res2 = session
            .get(&format!("http://localhost:{}/http3_test", port), vec![])
            .await?;
        assert_eq!(res2.http_code, 200);

        server_handle.abort();
        Ok(())
    }
}

/// 不验证证书的验证器
#[derive(Debug)]
struct NoCertVerification;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        vec![
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA256,
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA384,
            tokio_rustls::rustls::SignatureScheme::RSA_PKCS1_SHA512,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            tokio_rustls::rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA256,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA384,
            tokio_rustls::rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}
