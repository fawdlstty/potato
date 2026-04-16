#![cfg(feature = "http2")]

use crate::utils::refstr::HeaderOrHipStr;
use crate::{HttpMethod, HttpRequest, HttpRequestTargetForm};
use h2::server as h2_server;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use super::PipeContext;

pub async fn serve_http2_impl(
    addr: &str,
    cert_file: &str,
    key_file: &str,
    pipe_ctx: Arc<PipeContext>,
) -> anyhow::Result<()> {
    #[cfg(feature = "jemalloc")]
    crate::init_jemalloc()?;

    let addr: SocketAddr = addr.parse()?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let acceptor = tls_acceptor_with_alpn(
        cert_file,
        key_file,
        Some(vec![b"h2".to_vec(), b"http/1.1".to_vec()]),
    )?;

    loop {
        let (stream, client_addr) = listener.accept().await?;
        _ = stream.set_nodelay(true);
        let acceptor = acceptor.clone();
        let pipe_ctx2 = Arc::clone(&pipe_ctx);
        _ = tokio::task::spawn(async move {
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => return,
            };

            let negotiated_h2 = stream
                .get_ref()
                .1
                .alpn_protocol()
                .map(|p| p == b"h2")
                .unwrap_or(false);

            if !negotiated_h2 {
                // HTTP/1.1连接，使用spawn_http1_connection处理
                use crate::utils::tcp_stream::HttpStream;
                super::HttpServer::spawn_http1_connection(
                    pipe_ctx2,
                    client_addr,
                    HttpStream::from_server_tls(stream),
                );
                return;
            }

            let mut h2_conn = match h2_server::handshake(stream).await {
                Ok(conn) => conn,
                Err(_) => return,
            };

            while let Some(next) = h2_conn.accept().await {
                let (req_head, respond) = match next {
                    Ok(parts) => parts,
                    Err(_) => break,
                };
                let pipe_ctx3 = Arc::clone(&pipe_ctx2);
                _ = tokio::task::spawn(async move {
                    let _ = handle_h2_request(req_head, respond, pipe_ctx3, client_addr).await;
                });
            }
        });
    }
}

fn tls_acceptor_with_alpn(
    cert_file: &str,
    key_file: &str,
    alpn: Option<Vec<Vec<u8>>>,
) -> anyhow::Result<TlsAcceptor> {
    // 初始化 rustls CryptoProvider（如果尚未初始化）
    {
        use rustls::crypto::ring::default_provider;
        use rustls::crypto::CryptoProvider;
        let _ = CryptoProvider::install_default(default_provider());
    }

    let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key_file)?;

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    if let Some(alpn) = alpn {
        server_config.alpn_protocols = alpn;
    }

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

fn h2_method_to_http_method(method: &http::Method) -> anyhow::Result<HttpMethod> {
    Ok(match method.as_str() {
        "GET" => HttpMethod::GET,
        "PUT" => HttpMethod::PUT,
        "COPY" => HttpMethod::COPY,
        "HEAD" => HttpMethod::HEAD,
        "LOCK" => HttpMethod::LOCK,
        "MOVE" => HttpMethod::MOVE,
        "POST" => HttpMethod::POST,
        "MKCOL" => HttpMethod::MKCOL,
        "PATCH" => HttpMethod::PATCH,
        "DELETE" => HttpMethod::DELETE,
        "UNLOCK" => HttpMethod::UNLOCK,
        "OPTIONS" => HttpMethod::OPTIONS,
        "PROPFIND" => HttpMethod::PROPFIND,
        "PROPPATCH" => HttpMethod::PROPPATCH,
        "TRACE" => HttpMethod::TRACE,
        _ => anyhow::bail!("Unsupported HTTP method: {method}"),
    })
}

fn is_h2_h3_forbidden_response_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-connection")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("trailer")
}

fn is_forbidden_trailer_for_h2_h3(name: &str) -> bool {
    name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("trailer")
        || name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("upgrade")
        || name.eq_ignore_ascii_case("proxy-authenticate")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("www-authenticate")
}

fn should_suppress_response_body(status: u16, request_method: HttpMethod) -> bool {
    (100..200).contains(&status)
        || status == 204
        || status == 304
        || request_method == HttpMethod::HEAD
}

async fn handle_h2_request(
    mut req_head: http::Request<h2::RecvStream>,
    mut respond: h2_server::SendResponse<bytes::Bytes>,
    pipe_ctx: Arc<PipeContext>,
    client_addr: SocketAddr,
) -> anyhow::Result<()> {
    let mut req = HttpRequest::new();
    req.method = h2_method_to_http_method(req_head.method())?;
    req.target_form = HttpRequestTargetForm::Origin;
    req.version = 20;
    req.client_addr = Some(client_addr);

    let path_and_query = req_head
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or("/");
    match path_and_query.split_once('?') {
        Some((path, query)) => {
            req.url_path = path.into();
            req.url_query = query
                .split('&')
                .map(|s| s.split_once('=').unwrap_or((s, "")))
                .map(|(a, b)| (a.into(), b.into()))
                .collect();
        }
        None => {
            req.url_path = path_and_query.into();
        }
    }

    let authority = req_head.uri().authority().map(|v| v.as_str().to_string());
    for (key, value) in req_head.headers().iter() {
        if let Ok(value) = value.to_str() {
            req.headers
                .insert(HeaderOrHipStr::from_str(key.as_str()), value.into());
        }
    }
    if let Some(authority) = authority {
        if let Some(host) = req.get_header("Host") {
            if !host.eq_ignore_ascii_case(&authority) {
                let response = http::Response::builder()
                    .status(400)
                    .body(())
                    .map_err(|e| anyhow::anyhow!("Failed to build response: {e}"))?;
                let _ = respond.send_response(response, true);
                return Ok(());
            }
        }
        req.headers
            .insert(HeaderOrHipStr::from_str("Host"), authority.into());
    }

    let mut request_body = Vec::new();
    let max_body_bytes = crate::global_config::ServerConfig::get_max_body_bytes();
    while let Some(chunk) = req_head.body_mut().data().await {
        let chunk = chunk?;
        if request_body.len() + chunk.len() > max_body_bytes {
            let response = http::Response::builder()
                .status(413)
                .body(())
                .map_err(|e| anyhow::anyhow!("Failed to build response: {e}"))?;
            let _ = respond.send_response(response, true);
            return Ok(());
        }
        request_body.extend_from_slice(&chunk);
    }
    req.body = request_body.into();

    let res = PipeContext::handle_request(pipe_ctx.as_ref(), &mut req, 0).await;

    let mut response_builder = http::Response::builder().status(res.http_code);
    for (key, value) in res.headers.iter() {
        if is_h2_h3_forbidden_response_header(key) {
            continue;
        }
        response_builder = response_builder.header(key.as_ref(), value.as_ref());
    }
    let response = response_builder
        .body(())
        .map_err(|e| anyhow::anyhow!("Failed to build response: {e}"))?;

    let suppress_body = should_suppress_response_body(res.http_code, req.method);
    let eos = suppress_body && res.trailers.is_empty();

    let mut stream = respond.send_response(response, eos)?;

    if !suppress_body {
        match res.body {
            crate::HttpResponseBody::Data(data) => {
                if !data.is_empty() {
                    let eos = res.trailers.is_empty();
                    stream
                        .send_data(bytes::Bytes::from(data), eos)
                        .map_err(|e| anyhow::anyhow!("Failed to send data: {e}"))?;
                }
            }
            crate::HttpResponseBody::Stream(mut rx) => {
                while let Some(chunk) = rx.recv().await {
                    stream
                        .send_data(bytes::Bytes::from(chunk), false)
                        .map_err(|e| anyhow::anyhow!("Failed to send data: {e}"))?;
                }
                // 发送空帧表示结束
                if !res.trailers.is_empty() {
                    stream
                        .send_data(bytes::Bytes::new(), false)
                        .map_err(|e| anyhow::anyhow!("Failed to send data: {e}"))?;
                }
            }
        }
    }

    if !res.trailers.is_empty() {
        let mut trailers = http::HeaderMap::new();
        for (key, value) in res.trailers.iter() {
            if is_forbidden_trailer_for_h2_h3(key) {
                continue;
            }
            if let Ok(name) = http::header::HeaderName::from_bytes(key.as_bytes()) {
                if let Ok(value) = http::HeaderValue::from_str(value) {
                    trailers.insert(name, value);
                }
            }
        }
        stream
            .send_trailers(trailers)
            .map_err(|e| anyhow::anyhow!("Failed to send trailers: {e}"))?;
    }

    Ok(())
}
