#![allow(non_camel_case_types)]
pub mod http2;
pub mod http3;

use crate::utils::bytes::CompressExt;
use crate::utils::refstr::{HeaderItem, Headers};
use crate::utils::tcp_stream::HttpStream;
use crate::{HttpMethod, HttpRequest, HttpResponse, HttpResponseBody, SERVER_STR};
use anyhow::anyhow;
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;

fn transfer_encoding_has_chunked(value: &str) -> bool {
    value
        .split(',')
        .any(|item| item.trim().eq_ignore_ascii_case("chunked"))
}

fn response_disallows_body(http_code: u16) -> bool {
    (100..200).contains(&http_code) || http_code == 204 || http_code == 304
}

fn is_sse_response(res: &HttpResponse) -> bool {
    let is_sse = res
        .get_header("Content-Type")
        .map(|v| {
            v.split(';')
                .next()
                .map(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let has_chunked = res
        .get_header("Transfer-Encoding")
        .map(transfer_encoding_has_chunked)
        .unwrap_or(false);
    is_sse && has_chunked
}

async fn stream_chunked_body_to_channel(
    mut stream: HttpStream,
    mut buf: Vec<u8>,
    tx: Sender<Vec<u8>>,
) -> anyhow::Result<()> {
    let mut cursor = 0usize;
    let mut tmp_buf = [0u8; 8192];
    loop {
        let line_end = loop {
            if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n") {
                break cursor + pos;
            }
            let n = stream.read(&mut tmp_buf).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&tmp_buf[..n]);
        };

        let size_line = std::str::from_utf8(&buf[cursor..line_end])?;
        let size_token = size_line
            .split_once(';')
            .map_or(size_line, |(size, _)| size)
            .trim();
        if size_token.is_empty() {
            return Err(anyhow!("invalid chunk size"));
        }
        let chunk_size = usize::from_str_radix(size_token, 16)?;
        cursor = line_end + 2;

        if chunk_size == 0 {
            return Ok(());
        }

        while buf.len() < cursor + chunk_size + 2 {
            let n = stream.read(&mut tmp_buf).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&tmp_buf[..n]);
        }

        if &buf[cursor + chunk_size..cursor + chunk_size + 2] != b"\r\n" {
            return Err(anyhow!("invalid chunk terminator"));
        }

        if tx
            .send(buf[cursor..cursor + chunk_size].to_vec())
            .await
            .is_err()
        {
            return Ok(());
        }

        cursor += chunk_size + 2;
        if cursor > 8192 {
            buf.drain(..cursor);
            cursor = 0;
        }
    }
}

macro_rules! define_session_method {
    ($fn_name:ident, $method:ident) => {
        pub async fn $fn_name(
            &mut self,
            url: &str,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let mut req = self.new_request(HttpMethod::$method, url).await?;
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
            let mut req = self.new_request(HttpMethod::$method, url).await?;
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

pub struct SessionImpl {
    pub unique_host: (String, bool, u16),
    pub stream: HttpStream,
}

impl SessionImpl {
    pub async fn new(host: String, use_ssl: bool, port: u16) -> anyhow::Result<Self> {
        let stream: HttpStream = match use_ssl {
            #[cfg(feature = "tls")]
            true => {
                use rustls_pki_types::ServerName;
                use std::sync::Arc;
                use tokio_rustls::rustls::{ClientConfig, RootCertStore};
                use tokio_rustls::TlsConnector;
                let mut root_cert = RootCertStore::empty();
                root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = ClientConfig::builder()
                    .with_root_certificates(root_cert)
                    .with_no_client_auth();
                let connector = TlsConnector::from(Arc::new(config));
                let dnsname = ServerName::try_from(host.clone())?;
                let stream = TcpStream::connect((host.as_str(), port)).await?;
                let stream = connector.connect(dnsname, stream).await?;
                HttpStream::from_client_tls(stream)
            }
            #[cfg(not(feature = "tls"))]
            true => Err(anyhow!("unsupported tls during non-tls build"))?,
            false => {
                let stream = TcpStream::connect((host.as_str(), port)).await?;
                HttpStream::from_tcp(stream)
            }
        };
        Ok(SessionImpl {
            unique_host: (host, use_ssl, port),
            stream,
        })
    }
}

pub struct Session {
    pub sess_impl: Option<SessionImpl>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self { sess_impl: None }
    }

    pub async fn new_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<HttpRequest> {
        let (mut req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        let host = url
            .parse::<http::Uri>()?
            .host()
            .unwrap_or("127.0.0.1")
            .to_string();
        let mut is_same_host = false;
        if let Some(sess_impl) = &mut self.sess_impl {
            let (host1, use_ssl1, port1) = &sess_impl.unique_host;
            if (host1, use_ssl1, port1) == (&host, &use_ssl, &port) {
                is_same_host = true;
            }
        }
        if !is_same_host {
            self.sess_impl = Some(SessionImpl::new(host, use_ssl, port).await?);
        }
        req.apply_header(Headers::User_Agent(SERVER_STR.clone()));
        Ok(req)
    }

    pub async fn do_request(&mut self, req: HttpRequest) -> anyhow::Result<HttpResponse> {
        let request_method = req.method;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let mut sse_response: Option<HttpResponse> = None;
        {
            let sess_impl = self.session_impl()?;
            sess_impl.stream.write_all(&req.as_bytes()).await?;
            loop {
                if let Some((res, hdr_len)) = HttpResponse::from_headers_part(&buf[..])? {
                    if request_method != HttpMethod::HEAD
                        && !response_disallows_body(res.http_code)
                        && is_sse_response(&res)
                    {
                        let (tx, rx) = tokio::sync::mpsc::channel(64);
                        let body_buf = buf[hdr_len..].to_vec();
                        let mut res = res;
                        res.body = HttpResponseBody::Stream(rx);

                        let (duplex_stream, _) = tokio::io::duplex(1);
                        let stream = std::mem::replace(
                            &mut sess_impl.stream,
                            HttpStream::from_duplex_stream(duplex_stream),
                        );
                        tokio::spawn(async move {
                            _ = stream_chunked_body_to_channel(stream, body_buf, tx).await;
                        });
                        sse_response = Some(res);
                        break;
                    }
                    break;
                }
                let mut tmp_buf = [0u8; 4096];
                let n = sess_impl.stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    return Err(anyhow!("connection closed"));
                }
                buf.extend_from_slice(&tmp_buf[..n]);
            }
        }
        if let Some(res) = sse_response {
            self.sess_impl = None;
            return Ok(res);
        }
        let sess_impl = self.session_impl()?;
        let (res, _) = HttpResponse::from_stream_with_request_method(
            &mut buf,
            &mut sess_impl.stream,
            Some(request_method),
        )
        .await?;
        Ok(res)
    }

    fn session_impl(&mut self) -> anyhow::Result<&mut SessionImpl> {
        self.sess_impl
            .as_mut()
            .ok_or_else(|| anyhow!("session implementation not initialized"))
    }

    define_session_method!(get, GET);
    define_session_method!(post, post_json, post_json_str, POST);
    define_session_method!(put, put_json, put_json_str, PUT);
    define_session_method!(delete, DELETE);
    define_session_method!(head, HEAD);
    define_session_method!(options, OPTIONS);
    define_session_method!(connect, CONNECT);
    define_session_method!(patch, PATCH);
    define_session_method!(trace, TRACE);
}

macro_rules! define_client_method {
    ($fn_name:ident) => {
        pub async fn $fn_name(url: &str, args: Vec<Headers>) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name(url, args).await
        }
    };
    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident) => {
        pub async fn $fn_name(
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name(url, body, args).await
        }

        pub async fn $fn_name2(
            url: &str,
            body: serde_json::Value,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name2(url, body, args).await
        }

        pub async fn $fn_name3(
            url: &str,
            body: String,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name3(url, body, args).await
        }
    };
}
define_client_method!(get);
define_client_method!(post, post_json, post_json_str);
define_client_method!(put, put_json, put_json_str);
define_client_method!(delete);
define_client_method!(head);
define_client_method!(options);
define_client_method!(connect);
define_client_method!(patch);
define_client_method!(trace);

/// HTTP 协议版本选择器
#[derive(Clone, Debug)]
pub enum HttpVersion {
    /// HTTP/1.1 (默认)
    Http11,
    /// HTTP/2
    #[cfg(feature = "http2")]
    Http2,
    /// HTTP/3
    #[cfg(feature = "http3")]
    Http3,
}

/// URL 包装器，用于指定 HTTP 协议版本
pub struct VersionedUrl {
    pub url: String,
    pub version: HttpVersion,
}

/// 创建 HTTP/1.1 URL（默认，可省略）
pub fn http11(url: impl Into<String>) -> VersionedUrl {
    VersionedUrl {
        url: url.into(),
        version: HttpVersion::Http11,
    }
}

/// 创建 HTTP/2 URL
#[cfg(feature = "http2")]
pub fn http2(url: impl Into<String>) -> VersionedUrl {
    VersionedUrl {
        url: url.into(),
        version: HttpVersion::Http2,
    }
}

/// 创建 HTTP/3 URL
#[cfg(feature = "http3")]
pub fn http3(url: impl Into<String>) -> VersionedUrl {
    VersionedUrl {
        url: url.into(),
        version: HttpVersion::Http3,
    }
}

/// 统一的 GET 请求函数，根据 URL 版本选择器自动选择协议
pub async fn get_versioned(
    versioned_url: VersionedUrl,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => get(&versioned_url.url, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::get(&versioned_url.url, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::get(&versioned_url.url, args).await,
    }
}

/// 统一的 POST 请求函数
pub async fn post_versioned(
    versioned_url: VersionedUrl,
    body: Vec<u8>,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => post(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::post(&versioned_url.url, body, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::post(&versioned_url.url, body, args).await,
    }
}

/// 统一的 POST JSON 请求函数
pub async fn post_json_versioned(
    versioned_url: VersionedUrl,
    body: serde_json::Value,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => post_json(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::post_json(&versioned_url.url, body, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::post_json(&versioned_url.url, body, args).await,
    }
}

/// 统一的 POST JSON String 请求函数
pub async fn post_json_str_versioned(
    versioned_url: VersionedUrl,
    body: String,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => post_json_str(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => {
            crate::client::http2::post_json_str(&versioned_url.url, body, args).await
        }
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => {
            crate::client::http3::post_json_str(&versioned_url.url, body, args).await
        }
    }
}

/// 统一的 PUT 请求函数
pub async fn put_versioned(
    versioned_url: VersionedUrl,
    body: Vec<u8>,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => put(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::put(&versioned_url.url, body, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::put(&versioned_url.url, body, args).await,
    }
}

/// 统一的 PUT JSON 请求函数
pub async fn put_json_versioned(
    versioned_url: VersionedUrl,
    body: serde_json::Value,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => put_json(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::put_json(&versioned_url.url, body, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::put_json(&versioned_url.url, body, args).await,
    }
}

/// 统一的 PUT JSON String 请求函数
pub async fn put_json_str_versioned(
    versioned_url: VersionedUrl,
    body: String,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => put_json_str(&versioned_url.url, body, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => {
            crate::client::http2::put_json_str(&versioned_url.url, body, args).await
        }
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => {
            crate::client::http3::put_json_str(&versioned_url.url, body, args).await
        }
    }
}

/// 统一的 DELETE 请求函数
pub async fn delete_versioned(
    versioned_url: VersionedUrl,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => delete(&versioned_url.url, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::delete(&versioned_url.url, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::delete(&versioned_url.url, args).await,
    }
}

/// 统一的 HEAD 请求函数
pub async fn head_versioned(
    versioned_url: VersionedUrl,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => head(&versioned_url.url, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::head(&versioned_url.url, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::head(&versioned_url.url, args).await,
    }
}

/// 统一的 OPTIONS 请求函数
pub async fn options_versioned(
    versioned_url: VersionedUrl,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => options(&versioned_url.url, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::options(&versioned_url.url, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::options(&versioned_url.url, args).await,
    }
}

/// 统一的 PATCH 请求函数
pub async fn patch_versioned(
    versioned_url: VersionedUrl,
    args: Vec<Headers>,
) -> anyhow::Result<HttpResponse> {
    match versioned_url.version {
        HttpVersion::Http11 => patch(&versioned_url.url, args).await,
        #[cfg(feature = "http2")]
        HttpVersion::Http2 => crate::client::http2::patch(&versioned_url.url, args).await,
        #[cfg(feature = "http3")]
        HttpVersion::Http3 => crate::client::http3::patch(&versioned_url.url, args).await,
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __potato_headers_vec {
    () => {
        Vec::<$crate::Headers>::new()
    };
    ($($header:ident = $value:expr),+ $(,)?) => {{
        vec![$($crate::Headers::$header(($value).into())),+]
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __potato_push_header {
    ($headers:expr, $key:literal = $value:expr) => {{
        $headers.push($crate::Headers::Custom(($key.into(), ($value).into())));
    }};
    ($headers:expr, $header:ident = $value:expr) => {{
        $headers.push($crate::Headers::$header(($value).into()));
    }};
    ($headers:expr, Custom($key:expr) = $value:expr) => {{
        $headers.push($crate::Headers::Custom((($key).into(), ($value).into())));
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __potato_parse_headers {
    ($headers:expr,) => {};
    ($headers:expr, $key:literal = $value:expr, $($rest:tt)+) => {{
        $headers.push($crate::Headers::Custom(($key.into(), ($value).into())));
        $crate::__potato_parse_headers!($headers, $($rest)+);
    }};
    ($headers:expr, $header:ident = $value:expr, $($rest:tt)+) => {{
        $headers.push($crate::Headers::$header(($value).into()));
        $crate::__potato_parse_headers!($headers, $($rest)+);
    }};
    ($headers:expr, Custom($key:expr) = $value:expr, $($rest:tt)+) => {{
        $headers.push($crate::Headers::Custom((($key).into(), ($value).into())));
        $crate::__potato_parse_headers!($headers, $($rest)+);
    }};
    ($headers:expr, $key:literal = $value:expr) => {{
        $headers.push($crate::Headers::Custom(($key.into(), ($value).into())));
    }};
    ($headers:expr, $header:ident = $value:expr) => {{
        $headers.push($crate::Headers::$header(($value).into()));
    }};
    ($headers:expr, Custom($key:expr) = $value:expr) => {{
        $headers.push($crate::Headers::Custom((($key).into(), ($value).into())));
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __potato_detect_url_version {
    // 检测 http11() 包装器
    (http11($url:expr)) => {
        $crate::client::http11($url)
    };
    // 检测 http2() 包装器
    (http2($url:expr)) => {
        $crate::client::http2($url)
    };
    // 检测 http3() 包装器
    (http3($url:expr)) => {
        $crate::client::http3($url)
    };
    // 默认情况：直接使用 URL（HTTP/1.1）
    ($url:expr) => {
        $crate::client::http11($url)
    };
}

#[macro_export]
macro_rules! get {
    ($url:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::get_versioned(versioned_url, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::get_versioned(versioned_url, headers)
    }};
}

#[macro_export]
macro_rules! delete {
    ($url:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::delete_versioned(versioned_url, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::delete_versioned(versioned_url, headers)
    }};
}

#[macro_export]
macro_rules! head {
    ($url:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::head_versioned(versioned_url, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::head_versioned(versioned_url, headers)
    }};
}

#[macro_export]
macro_rules! options {
    ($url:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::options_versioned(versioned_url, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::options_versioned(versioned_url, headers)
    }};
}

#[macro_export]
macro_rules! connect {
    ($url:expr $(,)?) => {
        $crate::connect($url, $crate::__potato_headers_vec!())
    };
    ($url:expr, $($tt:tt)+) => {{
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::connect($url, headers)
    }};
}

#[macro_export]
macro_rules! trace {
    ($url:expr $(,)?) => {
        $crate::trace($url, $crate::__potato_headers_vec!())
    };
    ($url:expr, $($tt:tt)+) => {{
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::trace($url, headers)
    }};
}

#[macro_export]
macro_rules! post {
    ($url:expr, $body:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::post_versioned(versioned_url, $body, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $body:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::post_versioned(versioned_url, $body, headers)
    }};
}

#[macro_export]
macro_rules! put {
    ($url:expr, $body:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::put_versioned(versioned_url, $body, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $body:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::put_versioned(versioned_url, $body, headers)
    }};
}

#[macro_export]
macro_rules! patch {
    ($url:expr $(,)?) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        $crate::patch_versioned(versioned_url, $crate::__potato_headers_vec!())
    }};
    ($url:expr, $($tt:tt)+) => {{
        let versioned_url = $crate::__potato_detect_url_version!($url);
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::patch_versioned(versioned_url, headers)
    }};
}

#[macro_export]
macro_rules! websocket {
    ($url:expr $(,)?) => {
        $crate::Websocket::connect($url, $crate::__potato_headers_vec!())
    };
    ($url:expr, $($tt:tt)+) => {{
        let mut headers = Vec::<$crate::Headers>::new();
        $crate::__potato_parse_headers!(headers, $($tt)+);
        $crate::Websocket::connect($url, headers)
    }};
}

pub struct TransferSession {
    pub req_path_prefix: String,
    pub dest_url: Option<String>,
    #[cfg(feature = "ssh")]
    pub jumpbox_srv: Option<russh::client::Handle<AuthHandler>>,
    pub conns: HashMap<(String, bool, u16), HttpStream>,
}

fn parse_connection_option_tokens(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn is_known_hop_by_hop_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("Connection")
        || name.eq_ignore_ascii_case("Keep-Alive")
        || name.eq_ignore_ascii_case("Proxy-Authenticate")
        || name.eq_ignore_ascii_case("Proxy-Authorization")
        || name.eq_ignore_ascii_case("TE")
        || name.eq_ignore_ascii_case("Trailer")
        || name.eq_ignore_ascii_case("Transfer-Encoding")
        || name.eq_ignore_ascii_case("Upgrade")
        // Widely used de-facto hop-by-hop field in proxy deployments.
        || name.eq_ignore_ascii_case("Proxy-Connection")
}

fn format_host_header_value(host: &str, port: u16, use_ssl: bool) -> String {
    let normalized_host = if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let default_port = if use_ssl { 443 } else { 80 };
    if port == default_port {
        normalized_host
    } else {
        format!("{normalized_host}:{port}")
    }
}

fn strip_hop_by_hop_request_headers(req: &mut HttpRequest) {
    let connection_tokens = req
        .get_header_key(HeaderItem::Connection)
        .map(parse_connection_option_tokens)
        .unwrap_or_default();
    req.headers.retain(|key, _| {
        let header_name = key.to_str();
        !is_known_hop_by_hop_header(header_name)
            && !connection_tokens
                .iter()
                .any(|token| token.eq_ignore_ascii_case(header_name))
    });
}

fn strip_hop_by_hop_response_headers(res: &mut HttpResponse) {
    let connection_tokens = res
        .get_header("Connection")
        .map(parse_connection_option_tokens)
        .unwrap_or_default();
    res.headers.retain(|key, _| {
        let header_name = key.as_ref();
        !is_known_hop_by_hop_header(header_name)
            && !connection_tokens
                .iter()
                .any(|token| token.eq_ignore_ascii_case(header_name))
    });
}

impl TransferSession {
    pub fn from_forward_proxy() -> Self {
        TransferSession {
            req_path_prefix: "/".to_string(),
            dest_url: None,
            #[cfg(feature = "ssh")]
            jumpbox_srv: None,
            conns: HashMap::new(),
        }
    }

    pub fn from_reverse_proxy(
        req_path_prefix: impl Into<String>,
        dest_url: impl Into<String>,
    ) -> Self {
        TransferSession {
            req_path_prefix: req_path_prefix.into(),
            dest_url: Some(dest_url.into()),
            #[cfg(feature = "ssh")]
            jumpbox_srv: None,
            conns: HashMap::new(),
        }
    }

    #[cfg(feature = "ssh")]
    pub async fn with_ssh_jumpbox(&mut self, jumpbox: &SshJumpboxInfo) -> anyhow::Result<()> {
        use std::sync::Arc;
        let config = Arc::new(russh::client::Config::default());

        let mut handle =
            russh::client::connect(config, (&jumpbox.host[..], jumpbox.port), AuthHandler {})
                .await?;

        let auth_result = handle
            .authenticate_password(jumpbox.username.clone(), jumpbox.password.clone())
            .await?;
        if auth_result != russh::client::AuthResult::Success {
            Err(anyhow!("Authentication failed for SSH jumpbox"))?;
        }
        self.jumpbox_srv = Some(handle);
        Ok(())
    }

    pub async fn transfer(
        &mut self,
        req: &mut HttpRequest,
        modify_content: bool,
    ) -> anyhow::Result<HttpResponse> {
        if req.is_websocket() {
            return self.transfer_websocket(req).await;
        }

        let (dest_host, dest_use_ssl, dest_port) = if let Some(ref dest_url) = self.dest_url {
            let uri = dest_url.parse::<http::Uri>()?;
            let host = uri.host().unwrap_or("localhost");
            let use_ssl = uri.scheme() == Some(&http::uri::Scheme::HTTPS);
            let port = uri.port_u16().unwrap_or(if use_ssl { 443 } else { 80 });

            if self.req_path_prefix != "/" {
                let orig_path = req.url_path.to_string();
                if orig_path.starts_with(&self.req_path_prefix) {
                    let new_path = orig_path
                        .strip_prefix(&self.req_path_prefix)
                        .unwrap_or(&orig_path);
                    req.url_path = new_path.to_string().into();
                }
            }

            (host.to_string(), use_ssl, port)
        } else {
            let host_header = req.get_header("Host").unwrap_or("localhost");
            let authority = host_header
                .parse::<http::uri::Authority>()
                .ok()
                .or_else(|| {
                    format!("{host_header}:80")
                        .parse::<http::uri::Authority>()
                        .ok()
                });
            let host = authority
                .as_ref()
                .map(|a| a.host())
                .unwrap_or("localhost")
                .to_string();

            let (use_ssl, port) = if req.method == HttpMethod::CONNECT {
                (true, 443)
            } else {
                let port_from_header =
                    authority.as_ref().and_then(|a| a.port_u16()).or_else(|| {
                        host_header
                            .split_once(':')
                            .and_then(|(_, p)| p.parse::<u16>().ok())
                    });

                let use_ssl = req
                    .get_header("X-Forwarded-Proto")
                    .is_some_and(|proto| proto == "https")
                    || req.get_header("X-Forwarded-Proto-Https").is_some()
                    || port_from_header.is_some_and(|p| p == 443);
                let port = port_from_header.unwrap_or(if use_ssl { 443 } else { 80 });

                (use_ssl, port)
            };

            (host, use_ssl, port)
        };

        let conn_key = (dest_host.clone(), dest_use_ssl, dest_port);
        let stream = match self.conns.get_mut(&conn_key) {
            Some(stream) => stream,
            None => {
                #[cfg(not(feature = "ssh"))]
                let new_stream = None;
                #[cfg(feature = "ssh")]
                let mut new_stream = None;
                #[cfg(feature = "ssh")]
                if let Some(jumpbox_srv) = &self.jumpbox_srv {
                    let mut channel = jumpbox_srv
                        .channel_open_direct_tcpip(&dest_host, dest_port as u32, "127.0.0.1", 0)
                        .await
                        .map_err(|p| anyhow!("Failed to connect {dest_host} over ssh: {p}"))?;

                    let (stream1, stream2) = tokio::io::duplex(65536);

                    let (mut reader, mut writer) = tokio::io::split(stream2);

                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let mut buffer = vec![0u8; 8192];
                        loop {
                            tokio::select! {
                                msg = channel.wait() => {
                                    match msg {
                                        Some(russh::ChannelMsg::Data { data }) => {
                                            if writer.write_all(&data).await.is_err() {
                                                break;
                                            }
                                            if writer.flush().await.is_err() {
                                                break;
                                            }
                                        }
                                        Some(_) => continue,
                                        None => break,
                                    }
                                },
                                result = reader.read(&mut buffer) => {
                                    match result {
                                        Ok(0) => break,
                                        Ok(n) => {
                                            if channel.data(&buffer[..n]).await.is_err() {
                                                break;
                                            }
                                        }
                                        Err(_) => break,
                                    }
                                },
                            }
                        }
                    });

                    new_stream = Some(HttpStream::from_duplex_stream(stream1));
                }
                let new_stream = match new_stream {
                    Some(new_stream) => new_stream,
                    None => match dest_use_ssl {
                        #[cfg(feature = "tls")]
                        true => {
                            use rustls_pki_types::ServerName;
                            use std::sync::Arc;
                            use tokio_rustls::rustls::{ClientConfig, RootCertStore};
                            use tokio_rustls::TlsConnector;

                            let mut root_cert = RootCertStore::empty();
                            root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                            let config = ClientConfig::builder()
                                .with_root_certificates(root_cert)
                                .with_no_client_auth();
                            let connector = TlsConnector::from(Arc::new(config));
                            let dnsname = ServerName::try_from(dest_host.clone())?;
                            let tcp_stream =
                                TcpStream::connect((dest_host.as_str(), dest_port)).await?;
                            let tls_stream = connector.connect(dnsname, tcp_stream).await?;
                            HttpStream::from_client_tls(tls_stream)
                        }
                        #[cfg(not(feature = "tls"))]
                        true => Err(anyhow!("unsupported tls during non-tls build"))?,
                        false => {
                            let tcp_stream =
                                TcpStream::connect((dest_host.as_str(), dest_port)).await?;
                            HttpStream::from_tcp(tcp_stream)
                        }
                    },
                };

                self.conns.insert(conn_key.clone(), new_stream);
                self.conns.get_mut(&conn_key).unwrap()
            }
        };

        strip_hop_by_hop_request_headers(req);
        req.version = 11;
        req.set_header(
            HeaderItem::Host,
            format_host_header_value(&dest_host, dest_port, dest_use_ssl),
        );
        let request_method = req.method;
        stream.write_all(&req.as_bytes()).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let (mut res, _) =
            HttpResponse::from_stream_with_request_method(&mut buf, stream, Some(request_method))
                .await?;
        strip_hop_by_hop_response_headers(&mut res);

        if modify_content {
            match res.get_header("Content-Encoding") {
                Some(s) if s.to_lowercase() == "gzip" => {
                    if let HttpResponseBody::Data(ref mut body_data) = res.body {
                        if let Ok(data) = body_data.decompress() {
                            if let Ok(s) = str::from_utf8(&data) {
                                if let Some(ref dest_url) = self.dest_url {
                                    let proxy_url = if dest_url.ends_with('/') {
                                        &dest_url[..dest_url.len() - 1]
                                    } else {
                                        dest_url.as_str()
                                    };
                                    let path = if self.req_path_prefix.ends_with('/') {
                                        &self.req_path_prefix[..self.req_path_prefix.len() - 1]
                                    } else {
                                        self.req_path_prefix.as_str()
                                    };
                                    let data = s.replace(proxy_url, path).into_bytes();
                                    if let Ok(data) = data.compress() {
                                        *body_data = data;
                                    }
                                }
                            }
                        }
                    }
                }
                Some(_) => {}
                None => {
                    if let HttpResponseBody::Data(ref mut body_data) = res.body {
                        if let Ok(s) = str::from_utf8(body_data) {
                            if let Some(ref dest_url) = self.dest_url {
                                let proxy_url = if dest_url.ends_with('/') {
                                    &dest_url[..dest_url.len() - 1]
                                } else {
                                    dest_url.as_str()
                                };
                                let path = if self.req_path_prefix.ends_with('/') {
                                    &self.req_path_prefix[..self.req_path_prefix.len() - 1]
                                } else {
                                    self.req_path_prefix.as_str()
                                };
                                *body_data = s.replace(proxy_url, path).into_bytes();
                            }
                        }
                    }
                }
            }
            if let HttpResponseBody::Data(ref body_data) = res.body {
                res.headers.insert(
                    "Content-Length".to_string().into(),
                    body_data.len().to_string().into(),
                );
            }
        }

        Ok(res)
    }

    async fn transfer_websocket(&mut self, req: &mut HttpRequest) -> anyhow::Result<HttpResponse> {
        fn build_websocket_url(
            scheme_opt: Option<&str>,
            host: &str,
            port: u16,
            path: &str,
            query_str: String,
        ) -> String {
            let scheme = match scheme_opt {
                Some("https") | Some("wss") => "wss",
                _ => "ws",
            };
            let port_str = match (scheme, port) {
                ("wss", 443) | ("ws", 80) => "".to_string(),
                _ => format!(":{port}"),
            };
            format!("{scheme}://{host}{port_str}{path}{query_str}")
        }

        let dest_url = if let Some(ref dest_url_str) = self.dest_url {
            let uri = dest_url_str.parse::<http::Uri>()?;
            let path = if self.req_path_prefix != "/" {
                let orig_path = req.url_path.to_string();
                if orig_path.starts_with(&self.req_path_prefix) {
                    orig_path
                        .strip_prefix(&self.req_path_prefix)
                        .unwrap_or(&orig_path)
                        .to_string()
                } else {
                    orig_path
                }
            } else {
                req.url_path.to_string()
            };

            let host = uri.host().unwrap_or("localhost");
            let port =
                uri.port_u16()
                    .unwrap_or(if uri.scheme() == Some(&http::uri::Scheme::HTTPS) {
                        443
                    } else {
                        80
                    });
            build_websocket_url(uri.scheme_str(), host, port, &path, req.query_string())
        } else {
            let host = req.get_header_host().unwrap_or("localhost");

            let use_ssl = req
                .get_header("X-Forwarded-Proto")
                .map_or(false, |proto| proto == "https" || proto == "wss")
                || req
                    .get_header("X-Forwarded-Proto-Https")
                    .map_or(false, |_| true)
                || req.url_path.starts_with("https")
                || host.contains(".com") && !host.contains("127.") && !host.starts_with("192.")
                || host.contains("localhost");

            let (host_part, port_part) = host.split_once(':').unwrap_or((host, ""));

            let port = port_part
                .parse::<u16>()
                .unwrap_or(if use_ssl { 443 } else { 80 });

            let query_str = req.query_string();

            build_websocket_url(
                if use_ssl { Some("https") } else { None },
                host_part,
                port,
                &req.url_path,
                query_str,
            )
        };

        let mut headers = Vec::new();
        for (key, value) in req.headers.iter() {
            if key.to_str() == "Host" {
                continue;
            }
            headers.push(crate::Headers::Custom((
                key.to_str().to_string(),
                value.to_string(),
            )));
        }

        let mut target_ws = crate::Websocket::connect(&dest_url, headers)
            .await
            .map_err(|err| anyhow::anyhow!("Failed to connect to {dest_url}: {err}"))?;

        let mut client_ws = req
            .upgrade_websocket()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to upgrade to websocket: {err}"))?;

        loop {
            tokio::select! {
                frame = target_ws.recv() => {
                    match frame {
                        Ok(frame) => if client_ws.send(frame).await.is_err() {
                            break;
                        },
                        Err(_) => break,
                    }
                },
                frame = client_ws.recv() => {
                    match frame {
                        Ok(frame) => if target_ws.send(frame).await.is_err() {
                            break;
                        },
                        Err(_) => break,
                    }
                },
            };
        }

        Ok(HttpResponse::empty())
    }
}

#[cfg(feature = "ssh")]
pub struct AuthHandler {}
#[cfg(feature = "ssh")]
impl russh::client::Handler for AuthHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[derive(Clone)]
pub struct SshJumpboxInfo {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

// WebTransport 客户端实现 - 完整的实现在 crate::webtransport 模块中
// webtransport 类型已通过 lib.rs 中的 `pub use webtransport::*;` 导出

#[cfg(feature = "http3")]
pub use http3::WebTransport;

#[cfg(test)]
mod tests {
    use super::format_host_header_value;

    #[test]
    fn host_header_formatter_handles_domain() {
        assert_eq!(
            format_host_header_value("example.com", 80, false),
            "example.com"
        );
        assert_eq!(
            format_host_header_value("example.com", 8080, false),
            "example.com:8080"
        );
    }

    #[test]
    fn host_header_formatter_wraps_ipv6_literal() {
        assert_eq!(
            format_host_header_value("2001:db8::1", 80, false),
            "[2001:db8::1]"
        );
        assert_eq!(
            format_host_header_value("2001:db8::1", 8080, false),
            "[2001:db8::1]:8080"
        );
    }
}
