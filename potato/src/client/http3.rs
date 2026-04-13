#![allow(non_camel_case_types)]
#![cfg(feature = "http3")]

use crate::utils::refstr::Headers;
use crate::{HttpMethod, HttpRequest, HttpResponse, HttpResponseBody, SERVER_STR};
use anyhow::anyhow;
use bytes::Buf;
use h3_quinn::quinn;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct H3SessionImpl {
    pub unique_host: (String, u16),
    pub endpoint: quinn::Endpoint,
    pub send_request: h3::client::SendRequest<h3_quinn::OpenStreams, bytes::Bytes>,
    pub driver_handle: tokio::task::JoinHandle<()>,
}

impl H3SessionImpl {
    pub async fn new(host: String, port: u16) -> anyhow::Result<Self> {
        // 创建 TLS 配置，ALPN 协议设置为 h3
        let mut root_cert = tokio_rustls::rustls::RootCertStore::empty();
        root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut tls_config = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(root_cert)
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        // 创建 QUIC endpoint
        let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        ));
        endpoint.set_default_client_config(client_config);

        // 连接到服务器
        let quic_conn = endpoint
            .connect(format!("{}:{}", host, port).parse()?, &host)?
            .await
            .map_err(|e| anyhow!("QUIC connection failed: {}", e))?;

        // 初始化 HTTP/3 客户端
        let (mut driver, send_request) = h3::client::new(h3_quinn::Connection::new(quic_conn))
            .await
            .map_err(|e| anyhow!("HTTP/3 client initialization failed: {}", e))?;

        // 启动驱动任务
        let driver_handle = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        Ok(H3SessionImpl {
            unique_host: (host, port),
            endpoint,
            send_request,
            driver_handle,
        })
    }
}

pub struct H3Session {
    pub sess_impl: Option<H3SessionImpl>,
}

macro_rules! define_h3_session_method {
    ($fn_name:ident, $method:ident) => {
        pub async fn $fn_name(
            &mut self,
            url: &str,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let (mut req, _) = self.new_request(HttpMethod::$method, url).await?;
            for arg in args.into_iter() {
                req.apply_header(arg);
            }
            self.do_request(req).await
        }
    };

    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident, $method:ident) => {
        pub async fn $fn_name(
            &mut self,
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let (mut req, _) = self.new_request(HttpMethod::$method, url).await?;
            req.body = body.into();
            for arg in args.into_iter() {
                req.apply_header(arg);
            }
            self.do_request(req).await
        }

        pub async fn $fn_name2(
            &mut self,
            url: &str,
            body: serde_json::Value,
            mut args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            args.push(Headers::Content_Type("application/json".into()));
            self.$fn_name(url, serde_json::to_vec(&body)?, args).await
        }

        pub async fn $fn_name3(
            &mut self,
            url: &str,
            body: String,
            mut args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            args.push(Headers::Content_Type("application/json".into()));
            self.$fn_name(url, body.into_bytes(), args).await
        }
    };
}

impl Default for H3Session {
    fn default() -> Self {
        Self::new()
    }
}

impl H3Session {
    pub fn new() -> Self {
        Self { sess_impl: None }
    }

    async fn new_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<(HttpRequest, &mut H3SessionImpl)> {
        let (mut req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        if !use_ssl {
            return Err(anyhow!("HTTP/3 requires TLS connection"));
        }

        let host = url
            .parse::<http::Uri>()?
            .host()
            .unwrap_or("127.0.0.1")
            .to_string();

        let mut is_same_host = false;
        if let Some(sess_impl) = &mut self.sess_impl {
            let (host1, port1) = &sess_impl.unique_host;
            if (host1, port1) == (&host, &port) {
                is_same_host = true;
            }
        }

        if !is_same_host {
            // 如果已有连接，先关闭
            if let Some(old_impl) = self.sess_impl.take() {
                old_impl.driver_handle.abort();
                old_impl.endpoint.wait_idle().await;
            }
            self.sess_impl = Some(H3SessionImpl::new(host, port).await?);
        }

        req.apply_header(Headers::User_Agent(SERVER_STR.clone()));
        req.version = 30; // HTTP/3 version

        let sess_impl = self
            .sess_impl
            .as_mut()
            .ok_or_else(|| anyhow!("session implementation not initialized"))?;

        Ok((req, sess_impl))
    }

    async fn do_request(&mut self, req: HttpRequest) -> anyhow::Result<HttpResponse> {
        let sess_impl = self
            .sess_impl
            .as_mut()
            .ok_or_else(|| anyhow!("session implementation not initialized"))?;

        // 构建 HTTP/3 请求
        let uri_str = format!("https://{}{}", sess_impl.unique_host.0, req.url_path);
        let uri: http::Uri = if !req.url_query.is_empty() {
            let query: Vec<String> = req
                .url_query
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!("{}?{}", uri_str, query.join("&")).parse()?
        } else {
            uri_str.parse()?
        };

        let method_str = match req.method {
            HttpMethod::GET => http::Method::GET,
            HttpMethod::POST => http::Method::POST,
            HttpMethod::PUT => http::Method::PUT,
            HttpMethod::DELETE => http::Method::DELETE,
            HttpMethod::HEAD => http::Method::HEAD,
            HttpMethod::OPTIONS => http::Method::OPTIONS,
            HttpMethod::PATCH => http::Method::PATCH,
            HttpMethod::CONNECT => http::Method::CONNECT,
            HttpMethod::TRACE => http::Method::TRACE,
            _ => http::Method::GET,
        };

        let mut builder = http::Request::builder().method(method_str).uri(uri);

        // 添加请求头
        for (key, value) in req.headers.iter() {
            if let (Ok(name), Ok(val)) = (
                http::header::HeaderName::from_str(key.to_str()),
                http::HeaderValue::from_str(value.as_ref()),
            ) {
                builder = builder.header(name, val);
            }
        }

        let has_body = !req.body.is_empty();
        let request = builder.body(())?;

        // 发送请求
        let mut stream = sess_impl
            .send_request
            .send_request(request)
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        // 如果有请求体，发送数据
        if has_body {
            stream
                .send_data(bytes::Bytes::from(req.body.to_vec()))
                .await
                .map_err(|e| anyhow!("Failed to send request body: {}", e))?;
        }

        // 完成请求发送
        stream
            .finish()
            .await
            .map_err(|e| anyhow!("Failed to finish request: {}", e))?;

        // 接收响应
        let response = stream
            .recv_response()
            .await
            .map_err(|e| anyhow!("Failed to receive response: {}", e))?;

        let status = response.status().as_u16();
        let response_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let name_str = name.to_string();
                let value_str = value.to_str().ok()?.to_string();
                Some((name_str, value_str))
            })
            .collect();

        // 检查是否是 SSE 响应
        let is_sse = response_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| {
                value
                    .split(';')
                    .next()
                    .map(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if is_sse {
            // 处理 SSE 流式响应
            let (tx, rx) = mpsc::channel(64);

            tokio::spawn(async move {
                loop {
                    match stream.recv_data().await {
                        Ok(Some(mut chunk)) => {
                            let data = chunk.copy_to_bytes(chunk.remaining()).to_vec();
                            if tx.send(data).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            });

            let mut res = HttpResponse::new();
            res.http_code = status;
            for (name, value) in response_headers.iter() {
                res.headers
                    .insert(name.clone().into(), value.clone().into());
            }
            res.body = HttpResponseBody::Stream(rx);
            Ok(res)
        } else {
            // 处理普通响应
            let mut body_data = Vec::new();
            loop {
                match stream.recv_data().await {
                    Ok(Some(mut chunk)) => {
                        body_data.extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()));
                    }
                    Ok(None) => break,
                    Err(e) => return Err(anyhow!("Failed to read response body: {}", e)),
                }
            }

            let mut res = HttpResponse::new();
            res.http_code = status;
            for (name, value) in response_headers.iter() {
                res.headers
                    .insert(name.clone().into(), value.clone().into());
            }
            res.body = HttpResponseBody::Data(body_data);
            Ok(res)
        }
    }

    define_h3_session_method!(get, GET);
    define_h3_session_method!(post, post_json, post_json_str, POST);
    define_h3_session_method!(put, put_json, put_json_str, PUT);
    define_h3_session_method!(delete, DELETE);
    define_h3_session_method!(head, HEAD);
    define_h3_session_method!(options, OPTIONS);
    define_h3_session_method!(patch, PATCH);
    define_h3_session_method!(connect, CONNECT);
    define_h3_session_method!(trace, TRACE);
}

macro_rules! define_h3_client_method {
    ($fn_name:ident) => {
        pub async fn $fn_name(url: &str, args: Vec<Headers>) -> anyhow::Result<HttpResponse> {
            H3Session::new().$fn_name(url, args).await
        }
    };
    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident) => {
        pub async fn $fn_name(
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H3Session::new().$fn_name(url, body, args).await
        }

        pub async fn $fn_name2(
            url: &str,
            body: serde_json::Value,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H3Session::new().$fn_name2(url, body, args).await
        }

        pub async fn $fn_name3(
            url: &str,
            body: String,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H3Session::new().$fn_name3(url, body, args).await
        }
    };
}

define_h3_client_method!(get);
define_h3_client_method!(post, post_json, post_json_str);
define_h3_client_method!(put, put_json, put_json_str);
define_h3_client_method!(delete);
define_h3_client_method!(head);
define_h3_client_method!(options);
define_h3_client_method!(patch);
define_h3_client_method!(connect);
define_h3_client_method!(trace);

/// WebTransport 客户端
pub struct WebTransport {
    connection: quinn::Connection,
}

impl WebTransport {
    /// 连接到 WebTransport 服务器
    pub async fn connect(url: &str, _headers: Vec<Headers>) -> anyhow::Result<Self> {
        // 解析 URL
        let uri: http::Uri = url.parse()?;
        let host = uri
            .host()
            .ok_or_else(|| anyhow!("Invalid URL: missing host"))?;
        let port = uri.port_u16().unwrap_or(443);
        let path = uri.path().to_string();

        // 创建 TLS 配置
        use tokio_rustls::rustls;
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let mut tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        // 创建 QUIC 端点
        let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        ));
        endpoint.set_default_client_config(client_config);

        // 连接到服务器
        let connection = endpoint
            .connect(format!("{}:{}", host, port).parse()?, host)?
            .await
            .map_err(|e| anyhow!("QUIC connection failed: {}", e))?;

        // 发送 HTTP/3 CONNECT 请求以建立 WebTransport 会话
        let (mut driver, mut send_request) =
            h3::client::new(h3_quinn::Connection::new(connection.clone()))
                .await
                .map_err(|e| anyhow!("HTTP/3 client initialization failed: {}", e))?;

        // 启动驱动任务
        let driver_handle = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        // 构建 CONNECT 请求
        let req = http::Request::builder()
            .method(http::Method::CONNECT)
            .uri(&path)
            .header(":protocol", "webtransport")
            .header(":scheme", "https")
            .header(":authority", format!("{}:{}", host, port))
            .body(())
            .map_err(|e| anyhow!("Failed to build CONNECT request: {}", e))?;

        // 发送 CONNECT 请求
        let mut stream = send_request
            .send_request(req)
            .await
            .map_err(|e| anyhow!("Failed to send CONNECT request: {}", e))?;

        // 等待响应
        let response = stream
            .recv_response()
            .await
            .map_err(|e| anyhow!("Failed to get response: {}", e))?;

        if response.status() != 200 {
            return Err(anyhow!(
                "WebTransport connection failed with status: {}",
                response.status()
            ));
        }

        // 完成请求
        let _ = stream
            .finish()
            .await
            .map_err(|e| anyhow!("Failed to finish request: {}", e))?;

        // 阻止 driver_handle 被 drop
        drop(driver_handle);

        Ok(Self { connection })
    }

    /// 打开一个新的双向流
    pub async fn open_bi(&self) -> anyhow::Result<crate::WebTransportStream> {
        let (send, recv) = self.connection.open_bi().await?;
        Ok(crate::WebTransportStream::new(send, recv))
    }

    /// 发送数据报
    pub async fn send_datagram(&self, data: &[u8]) -> anyhow::Result<()> {
        self.connection.send_datagram(data.to_vec().into())?;
        Ok(())
    }

    /// 接收数据报
    pub async fn recv_datagram(&self) -> anyhow::Result<Vec<u8>> {
        let data = self.connection.read_datagram().await?;
        Ok(data.to_vec())
    }
}
