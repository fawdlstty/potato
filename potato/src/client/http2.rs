#![allow(non_camel_case_types)]
#![cfg(feature = "http2")]

use crate::utils::refstr::Headers;
use crate::{HttpMethod, HttpRequest, HttpResponse, HttpResponseBody, SERVER_STR};
use anyhow::anyhow;
use bytes::Bytes;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;

type H2Sender = h2::client::SendRequest<Bytes>;

pub struct H2SessionImpl {
    pub unique_host: (String, u16),
    pub sender: H2Sender,
    pub conn_handle: tokio::task::JoinHandle<()>,
}

impl H2SessionImpl {
    pub async fn new(host: String, port: u16) -> anyhow::Result<Self> {
        use tokio::net::TcpStream;
        use tokio_rustls::rustls::pki_types::ServerName;
        use tokio_rustls::TlsConnector;

        // 创建 TLS 配置，ALPN 协议设置为 h2
        let mut root_cert = tokio_rustls::rustls::RootCertStore::empty();
        root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut config = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(root_cert)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec()];

        let connector = TlsConnector::from(Arc::new(config));
        let dnsname = ServerName::try_from(host.clone())?;
        let tcp_stream = TcpStream::connect((host.as_str(), port)).await?;
        let tls_stream = connector.connect(dnsname, tcp_stream).await?;

        // 执行 HTTP/2 握手
        let (sender, connection) = h2::client::handshake(tls_stream).await?;

        // 启动连接驱动任务
        let conn_handle = tokio::spawn(async move {
            let _ = connection.await;
        });

        Ok(H2SessionImpl {
            unique_host: (host, port),
            sender,
            conn_handle,
        })
    }
}

pub struct H2Session {
    pub sess_impl: Option<H2SessionImpl>,
}

macro_rules! define_h2_session_method {
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

impl Default for H2Session {
    fn default() -> Self {
        Self::new()
    }
}

impl H2Session {
    pub fn new() -> Self {
        Self { sess_impl: None }
    }

    async fn new_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<(HttpRequest, &mut H2SessionImpl)> {
        let (mut req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        if !use_ssl {
            return Err(anyhow!("HTTP/2 requires TLS connection"));
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
                old_impl.conn_handle.abort();
            }
            self.sess_impl = Some(H2SessionImpl::new(host, port).await?);
        }

        req.apply_header(Headers::User_Agent(SERVER_STR.clone()));
        req.version = 20; // HTTP/2 version

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

        // 构建 HTTP/2 请求
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
        let (response_future, stream) = sess_impl.sender.send_request(request, !has_body)?;

        // 如果有请求体，发送数据
        if has_body {
            let mut send_stream = stream;
            send_stream
                .send_data(Bytes::from(req.body.to_vec()), true)
                .map_err(|e| anyhow!("Failed to send request body: {}", e))?;
        }

        // 等待响应
        let response = response_future.await?;
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
            let mut body = response.into_body();

            tokio::spawn(async move {
                loop {
                    match body.data().await {
                        Some(chunk_result) => match chunk_result {
                            Ok(chunk) => {
                                if tx.send(chunk.to_vec()).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        },
                        None => break,
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
            let mut body = response.into_body();
            let mut body_data = Vec::new();
            loop {
                match body.data().await {
                    Some(chunk_result) => {
                        let chunk = chunk_result?;
                        body_data.extend_from_slice(&chunk);
                    }
                    None => break,
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

    define_h2_session_method!(get, GET);
    define_h2_session_method!(post, post_json, post_json_str, POST);
    define_h2_session_method!(put, put_json, put_json_str, PUT);
    define_h2_session_method!(delete, DELETE);
    define_h2_session_method!(head, HEAD);
    define_h2_session_method!(options, OPTIONS);
    define_h2_session_method!(patch, PATCH);
    define_h2_session_method!(connect, CONNECT);
    define_h2_session_method!(trace, TRACE);
}

macro_rules! define_h2_client_method {
    ($fn_name:ident) => {
        pub async fn $fn_name(url: &str, args: Vec<Headers>) -> anyhow::Result<HttpResponse> {
            H2Session::new().$fn_name(url, args).await
        }
    };
    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident) => {
        pub async fn $fn_name(
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H2Session::new().$fn_name(url, body, args).await
        }

        pub async fn $fn_name2(
            url: &str,
            body: serde_json::Value,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H2Session::new().$fn_name2(url, body, args).await
        }

        pub async fn $fn_name3(
            url: &str,
            body: String,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            H2Session::new().$fn_name3(url, body, args).await
        }
    };
}

define_h2_client_method!(get);
define_h2_client_method!(post, post_json, post_json_str);
define_h2_client_method!(put, put_json, put_json_str);
define_h2_client_method!(delete);
define_h2_client_method!(head);
define_h2_client_method!(options);
define_h2_client_method!(patch);
define_h2_client_method!(connect);
define_h2_client_method!(trace);
