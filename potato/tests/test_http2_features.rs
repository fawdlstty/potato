#[cfg(feature = "http2")]
use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "http2")]
use std::time::Duration;
#[cfg(feature = "http2")]
use tokio::time::sleep;

#[cfg(feature = "http2")]
static PORT_COUNTER: AtomicU16 = AtomicU16::new(29000);

#[cfg(feature = "http2")]
fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(feature = "http2")]
fn create_test_cert_files() -> anyhow::Result<(
    String,
    String,
    tokio_rustls::rustls::pki_types::CertificateDer<'static>,
)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    let temp_dir = std::env::temp_dir().join(format!("potato_http2_test_{}", get_test_port()));
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

#[cfg(feature = "http2")]
fn tls_client_config_with_alpn(
    cert_der: &tokio_rustls::rustls::pki_types::CertificateDer<'static>,
    alpn: Vec<Vec<u8>>,
) -> anyhow::Result<tokio_rustls::rustls::ClientConfig> {
    use tokio_rustls::rustls;

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der.clone())?;

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn;
    Ok(config)
}

#[cfg(feature = "http2")]
async fn connect_tls_with_alpn(
    addr: &str,
    cert_der: &tokio_rustls::rustls::pki_types::CertificateDer<'static>,
    alpn: Vec<Vec<u8>>,
) -> anyhow::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    use tokio::net::TcpStream;
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::TlsConnector;

    let config = tls_client_config_with_alpn(cert_der, alpn)?;
    let connector = TlsConnector::from(std::sync::Arc::new(config));
    let stream = TcpStream::connect(addr).await?;
    let server_name = ServerName::try_from("localhost")?.to_owned();
    Ok(connector.connect(server_name, stream).await?)
}

#[cfg(feature = "http2")]
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

#[cfg(feature = "http2")]
#[cfg(test)]
mod http2_tests {
    use super::*;
    use h2::client;
    use tokio::io::AsyncWriteExt;

    #[potato::http_get("/http2_https_fallback")]
    async fn http2_https_fallback(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("http1-over-tls-ok")
    }

    #[potato::http_get("/http2_native")]
    async fn http2_native(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("h2-ok")
    }

    #[potato::http_get("/http2_head_no_body")]
    async fn http2_head_no_body(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("head-body-must-not-be-sent")
    }

    #[potato::http_get("/http2_204_no_body")]
    async fn http2_204_no_body(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        let mut res = potato::HttpResponse::text("status-204-must-not-have-body");
        res.http_code = 204;
        res
    }

    #[potato::http_get("/http2_trailers")]
    async fn http2_trailers(_: &mut potato::HttpRequest) -> potato::HttpResponse {
        let mut res = potato::HttpResponse::text("with-trailer");
        res.add_trailer("X-Trace".into(), "h2-trace".into());
        res
    }

    #[tokio::test]
    async fn test_serve_http2_accepts_https_http11_fallback() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");
        let (cert_file, key_file, cert) = create_test_cert_files()?;

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http2(&cert_file, &key_file).await;
        });
        sleep(Duration::from_millis(350)).await;

        let mut tls_stream =
            connect_tls_with_alpn(&addr, &cert, vec![b"http/1.1".to_vec()]).await?;
        let request = format!(
            "GET /http2_https_fallback HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        tls_stream.write_all(request.as_bytes()).await?;

        let response = read_all_ignore_tls_close_notify(&mut tls_stream).await?;
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("http1-over-tls-ok"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_serve_http2_accepts_http2_requests() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");
        let (cert_file, key_file, cert) = create_test_cert_files()?;

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http2(&cert_file, &key_file).await;
        });
        sleep(Duration::from_millis(350)).await;

        let tls_stream = connect_tls_with_alpn(&addr, &cert, vec![b"h2".to_vec()]).await?;
        let (mut sender, connection) = client::handshake(tls_stream).await?;
        let conn_handle = tokio::spawn(async move {
            let _ = connection.await;
        });

        let request = http::Request::builder()
            .method("GET")
            .uri("https://localhost/http2_native")
            .body(())?;
        let (response_future, _) = sender.send_request(request, true)?;
        let response = response_future.await?;
        assert_eq!(response.status(), http::StatusCode::OK);

        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.data().await {
            bytes.extend_from_slice(&chunk?);
        }
        assert_eq!(bytes, b"h2-ok".to_vec());

        conn_handle.abort();
        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_http2_head_response_has_no_body() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");
        let (cert_file, key_file, cert) = create_test_cert_files()?;

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http2(&cert_file, &key_file).await;
        });
        sleep(Duration::from_millis(350)).await;

        let tls_stream = connect_tls_with_alpn(&addr, &cert, vec![b"h2".to_vec()]).await?;
        let (mut sender, connection) = client::handshake(tls_stream).await?;
        let conn_handle = tokio::spawn(async move {
            let _ = connection.await;
        });

        let request = http::Request::builder()
            .method("HEAD")
            .uri("https://localhost/http2_head_no_body")
            .body(())?;
        let (response_future, _) = sender.send_request(request, true)?;
        let response = response_future.await?;
        assert_eq!(response.status(), http::StatusCode::OK);

        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.data().await {
            bytes.extend_from_slice(&chunk?);
        }
        assert!(bytes.is_empty());

        conn_handle.abort();
        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_http2_204_and_trailers_semantics() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr = format!("127.0.0.1:{port}");
        let (cert_file, key_file, cert) = create_test_cert_files()?;

        let mut server = potato::HttpServer::new(&addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http2(&cert_file, &key_file).await;
        });
        sleep(Duration::from_millis(350)).await;

        let tls_stream = connect_tls_with_alpn(&addr, &cert, vec![b"h2".to_vec()]).await?;
        let (mut sender, connection) = client::handshake(tls_stream).await?;
        let conn_handle = tokio::spawn(async move {
            let _ = connection.await;
        });

        let request_204 = http::Request::builder()
            .method("GET")
            .uri("https://localhost/http2_204_no_body")
            .body(())?;
        let (response_future_204, _) = sender.send_request(request_204, true)?;
        let response_204 = response_future_204.await?;
        assert_eq!(response_204.status(), http::StatusCode::NO_CONTENT);
        let mut body_204 = response_204.into_body();
        let mut bytes_204 = Vec::new();
        while let Some(chunk) = body_204.data().await {
            bytes_204.extend_from_slice(&chunk?);
        }
        assert!(bytes_204.is_empty());

        let request_trailer = http::Request::builder()
            .method("GET")
            .uri("https://localhost/http2_trailers")
            .body(())?;
        let (response_future_trailer, _) = sender.send_request(request_trailer, true)?;
        let response_trailer = response_future_trailer.await?;
        assert_eq!(response_trailer.status(), http::StatusCode::OK);
        let mut body = response_trailer.into_body();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.data().await {
            bytes.extend_from_slice(&chunk?);
        }
        assert_eq!(bytes, b"with-trailer".to_vec());

        let trailers = body.trailers().await?;
        let trace = trailers
            .as_ref()
            .and_then(|map| map.get("x-trace"))
            .and_then(|val| val.to_str().ok());
        assert_eq!(trace, Some("h2-trace"));

        conn_handle.abort();
        server_handle.abort();
        Ok(())
    }
}
