#![cfg(feature = "http3")]

use crate::utils::refstr::HeaderOrHipStr;
use crate::{HttpMethod, HttpRequest, HttpRequestTargetForm};
use anyhow::Result;
use bytes::Buf;
use h3::server as h3_server;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{self, Connection, RecvStream, SendStream, VarInt};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

use super::PipeContext;

/// WebTransport 会话
pub struct WebTransportSession {
    inner: Connection,
    remote_addr: SocketAddr,
}

impl WebTransportSession {
    pub(crate) fn new(connection: Connection) -> Self {
        let remote_addr = connection.remote_address();
        Self {
            inner: connection,
            remote_addr,
        }
    }

    /// 接受一个新的双向流
    pub async fn accept_bi(&self) -> Result<Option<WebTransportStream>> {
        match self.inner.accept_bi().await {
            Ok((send, recv)) => Ok(Some(WebTransportStream::new(send, recv))),
            Err(quinn::ConnectionError::ApplicationClosed(_)) => Ok(None),
            Err(quinn::ConnectionError::ConnectionClosed(_)) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to accept bidirectional stream: {}",
                e
            )),
        }
    }

    /// 打开一个新的双向流（服务器主动发起）
    pub async fn open_bi(&self) -> Result<WebTransportStream> {
        let (send, recv) = self.inner.open_bi().await?;
        Ok(WebTransportStream::new(send, recv))
    }

    /// 打开一个新的单向流（服务器主动发送）
    pub async fn open_uni(&self) -> Result<SendStream> {
        let send = self.inner.open_uni().await?;
        Ok(send)
    }

    /// 接受一个新的单向接收流
    pub async fn accept_uni(&self) -> Result<Option<RecvStream>> {
        match self.inner.accept_uni().await {
            Ok(recv) => Ok(Some(recv)),
            Err(quinn::ConnectionError::ApplicationClosed(_)) => Ok(None),
            Err(quinn::ConnectionError::ConnectionClosed(_)) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to accept unidirectional stream: {}",
                e
            )),
        }
    }

    /// 接收数据报
    pub async fn recv_datagram(&self) -> Result<Vec<u8>> {
        match self.inner.read_datagram().await {
            Ok(data) => Ok(data.to_vec()),
            Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                Err(anyhow::anyhow!("WebTransport session closed"))
            }
            Err(quinn::ConnectionError::ConnectionClosed(_)) => {
                Err(anyhow::anyhow!("WebTransport connection closed"))
            }
            Err(e) => Err(anyhow::anyhow!("Failed to read datagram: {e}")),
        }
    }

    /// 发送数据报
    pub async fn send_datagram(&self, data: &[u8]) -> Result<()> {
        self.inner.send_datagram(data.to_vec().into())?;
        Ok(())
    }

    /// 获取远程地址
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// 关闭会话
    pub fn close(&self, error_code: u32, reason: &str) {
        self.inner
            .close(VarInt::from_u32(error_code), reason.as_bytes());
    }
}

/// WebTransport 双向流
pub struct WebTransportStream {
    send: SendStream,
    recv: RecvStream,
}

impl Drop for WebTransportStream {
    fn drop(&mut self) {
        // 当 WebTransportStream 被丢弃时，重置流以立即释放资源
        // 使用 reset 而不是 finish，因为：
        // 1. finish 会等待数据确认，在 Drop 中可能无法完成
        // 2. reset 立即终止流，避免对端继续发送数据造成资源泄漏
        // 3. 忽略错误，因为流可能已经被关闭
        _ = self.send.reset(VarInt::from_u32(0));
    }
}

impl WebTransportStream {
    pub(crate) fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }

    /// 发送数据
    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.send.write_all(data).await?;
        Ok(())
    }

    /// 接收数据
    /// 返回 None 表示流已关闭
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>> {
        Box::pin(self.recv_inner()).await
    }

    async fn recv_inner(&mut self) -> Result<Option<Vec<u8>>> {
        match self.recv.read_chunk(usize::MAX, false).await {
            // read_chunk 返回 Some(chunk) 表示有数据
            Ok(Some(chunk)) if chunk.bytes.is_empty() => Box::pin(self.recv_inner()).await,
            Ok(Some(chunk)) => Ok(Some(chunk.bytes.to_vec())),
            // 流已正常关闭
            Ok(None) => Ok(None),
            // 连接丢失，视为流关闭
            Err(quinn::ReadError::ConnectionLost(_)) => Ok(None),
            // 流被对端重置，视为流关闭
            Err(quinn::ReadError::Reset(_)) => Ok(None),
            // 流已关闭
            Err(quinn::ReadError::ClosedStream) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Failed to read from stream: {e}")),
        }
    }

    /// 完成发送，关闭发送端
    pub async fn finish(&mut self) -> Result<()> {
        self.send.finish()?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct WebTransportConfig {
    pub max_sessions: usize,
    pub max_streams_per_session: u32,
    pub datagram_enabled: bool,
    pub max_datagram_size: usize,
}

impl Default for WebTransportConfig {
    fn default() -> Self {
        Self {
            max_sessions: 1000,
            max_streams_per_session: 100,
            datagram_enabled: true,
            max_datagram_size: 1200,
        }
    }
}

pub type WebTransportHandler = Box<
    dyn Fn(WebTransportSession) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync,
>;

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
        _ => anyhow::bail!("Unsupported HTTP method: {}", method),
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

fn quinn_server_config(cert_file: &str, key_file: &str) -> anyhow::Result<quinn::ServerConfig> {
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

    server_config.alpn_protocols = vec![b"h3".to_vec()];

    let server_config = QuicServerConfig::try_from(server_config)?;

    let mut config = quinn::ServerConfig::with_crypto(Arc::new(server_config));

    // 配置 WebTransport 支持
    let transport_config = Arc::get_mut(&mut config.transport)
        .ok_or_else(|| anyhow::anyhow!("Failed to get mutable reference to transport config"))?;
    transport_config.max_concurrent_bidi_streams(100u32.into());
    transport_config.max_concurrent_uni_streams(100u32.into());

    Ok(config)
}

pub async fn serve_http3_impl(
    addr: &str,
    cert_file: &str,
    key_file: &str,
    pipe_ctx: Arc<PipeContext>,
) -> anyhow::Result<()> {
    #[cfg(feature = "jemalloc")]
    crate::init_jemalloc()?;

    let addr: SocketAddr = addr.parse()?;
    let server_config = quinn_server_config(cert_file, key_file)?;
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    while let Some(new_conn) = endpoint.accept().await {
        let pipe_ctx2 = Arc::clone(&pipe_ctx);
        _ = tokio::task::spawn(async move {
            let conn = match new_conn.await {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let client_addr = conn.remote_address();
            // 为 WebTransport 克隆连接
            let wt_conn = conn.clone();
            let mut h3_conn: h3_server::Connection<_, bytes::Bytes> =
                match h3_server::Connection::new(h3_quinn::Connection::new(conn)).await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };

            loop {
                let resolver = match h3_conn.accept().await {
                    Ok(Some(resolver)) => resolver,
                    Ok(None) => break,
                    Err(_) => break,
                };
                let pipe_ctx3 = Arc::clone(&pipe_ctx2);
                let wt_conn = wt_conn.clone();
                _ = tokio::task::spawn(async move {
                    let (req_head, mut stream) = match resolver.resolve_request().await {
                        Ok(req_stream) => req_stream,
                        Err(_) => return,
                    };

                    // 检查是否是 WebTransport CONNECT 请求
                    if req_head.method() == http::Method::CONNECT {
                        if let Some(protocol) = req_head.headers().get(":protocol") {
                            if protocol == "webtransport" {
                                // 检查路径是否匹配 WebTransport 路由
                                let path = req_head.uri().path();
                                let mut wt_handler: Option<(
                                    &WebTransportConfig,
                                    &WebTransportHandler,
                                )> = None;

                                for item in pipe_ctx3.items.iter() {
                                    if let super::PipeContextItem::WebTransport((
                                        wt_path,
                                        config,
                                        handler,
                                    )) = item
                                    {
                                        if path == wt_path
                                            || path.starts_with(&format!("{wt_path}/"))
                                        {
                                            wt_handler = Some((config, handler));
                                            break;
                                        }
                                    }
                                }

                                if let Some((_config, handler)) = wt_handler {
                                    // 发送 200 响应接受 WebTransport 会话
                                    let response =
                                        match http::Response::builder().status(200).body(()) {
                                            Ok(resp) => resp,
                                            Err(_) => return,
                                        };
                                    if stream.send_response(response).await.is_err() {
                                        return;
                                    }
                                    // 注意：不要调用 stream.finish()，因为 WebTransport 会话需要保持开放
                                    // HTTP/3 的 CONNECT 流在 WebTransport 会话期间应该保持开放

                                    // 创建 WebTransport 会话并调用处理器
                                    let wt_session = WebTransportSession::new(wt_conn);
                                    handler(wt_session).await;
                                    return;
                                }
                            }
                        }
                    }

                    let mut req = HttpRequest::new();
                    req.method = match h2_method_to_http_method(req_head.method()) {
                        Ok(method) => method,
                        Err(_) => return,
                    };
                    req.target_form = HttpRequestTargetForm::Origin;
                    req.version = 30;
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
                                let response = match http::Response::builder().status(400).body(())
                                {
                                    Ok(resp) => resp,
                                    Err(_) => return,
                                };
                                let _ = stream.send_response(response).await;
                                let _ = stream.finish().await;
                                return;
                            }
                        }
                        req.headers
                            .insert(HeaderOrHipStr::from_str("Host"), authority.into());
                    }

                    let mut request_body = Vec::new();
                    let max_body_bytes = crate::global_config::ServerConfig::get_max_body_bytes();
                    loop {
                        match stream.recv_data().await {
                            Ok(Some(mut chunk)) => {
                                let chunk_len = chunk.remaining();
                                if request_body.len() + chunk_len > max_body_bytes {
                                    let response =
                                        match http::Response::builder().status(413).body(()) {
                                            Ok(resp) => resp,
                                            Err(_) => return,
                                        };
                                    let _ = stream.send_response(response).await;
                                    let _ = stream.finish().await;
                                    return;
                                }
                                request_body
                                    .extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()));
                            }
                            Ok(None) => break,
                            Err(_) => return,
                        }
                    }
                    req.body = request_body.into();

                    let res = PipeContext::handle_request(pipe_ctx3.as_ref(), &mut req, 0).await;

                    let mut response_builder = http::Response::builder().status(res.http_code);
                    for (key, value) in res.headers.iter() {
                        if is_h2_h3_forbidden_response_header(key) {
                            continue;
                        }
                        response_builder = response_builder.header(key.as_ref(), value.as_ref());
                    }
                    let response = match response_builder.body(()) {
                        Ok(resp) => resp,
                        Err(_) => return,
                    };
                    if stream.send_response(response).await.is_err() {
                        return;
                    }

                    let suppress_body = should_suppress_response_body(res.http_code, req.method);
                    if !suppress_body {
                        match res.body {
                            crate::HttpResponseBody::Data(data) => {
                                if !data.is_empty()
                                    && stream.send_data(bytes::Bytes::from(data)).await.is_err()
                                {
                                    return;
                                }
                            }
                            crate::HttpResponseBody::Stream(mut rx) => {
                                while let Some(chunk) = rx.recv().await {
                                    if stream.send_data(bytes::Bytes::from(chunk)).await.is_err() {
                                        return;
                                    }
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
                        let _ = stream.send_trailers(trailers).await;
                    }

                    let _ = stream.finish().await;
                });
            }
        });
    }

    endpoint.wait_idle().await;
    Ok(())
}
