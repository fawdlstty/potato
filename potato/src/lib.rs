pub mod client;
pub mod global_config;
pub mod server;
pub mod utils;

pub use client::*;
pub use global_config::*;
pub use hipstr;
pub use inventory;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
pub use serde_json;
pub use server::*;
use thread_local::ThreadLocal;
pub use utils::ai::*;
pub use utils::refstr::Headers;

#[cfg(feature = "jemalloc")]
pub use utils::jemalloc_helper::*;

use anyhow::anyhow;
use chrono::Utc;
use core::str;
use hipstr::{LocalHipByt, LocalHipStr};
use http::uri::Scheme;
use http::Uri;
use rust_embed::Embed;
use sha1::{Digest, Sha1};
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::fs::{File, Metadata};
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, collections::HashSet, future::Future, pin::Pin};
use strum::Display;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use utils::bytes::CompressExt;
use utils::enums::{HttpConnection, HttpContentType};
use utils::number::HttpCodeExt;
use utils::refstr::{HeaderItem, HeaderOrHipStr};
use utils::string::StringExt;
use utils::tcp_stream::{HttpStream, VecU8Ext};

/// HTTP conditional preflight result
#[derive(Debug, PartialEq)]
pub enum PreflightResult {
    /// Pass preflight check, can continue processing
    Proceed,
    /// Return 304 Not Modified
    NotModified,
    /// Return 412 Precondition Failed
    PreconditionFailed,
}

#[derive(Debug)]
pub enum HttpRequestParseError {
    BadRequest(String),
    NotImplemented(String),
    ExpectationFailed(String),
    RequestHeaderFieldsTooLarge(String),
}

impl fmt::Display for HttpRequestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpRequestParseError::BadRequest(msg) => write!(f, "{msg}"),
            HttpRequestParseError::NotImplemented(msg) => write!(f, "{msg}"),
            HttpRequestParseError::ExpectationFailed(msg) => write!(f, "{msg}"),
            HttpRequestParseError::RequestHeaderFieldsTooLarge(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for HttpRequestParseError {}

fn parse_declared_trailer_names(raw: Option<&str>) -> HashSet<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect::<HashSet<_>>()
    })
    .unwrap_or_default()
}

fn is_forbidden_trailer_field(name: &str) -> bool {
    // RFC 9110/9112: trailers must not carry framing or hop-by-hop control fields.
    matches!(
        name,
        "transfer-encoding"
            | "content-length"
            | "trailer"
            | "host"
            | "connection"
            | "keep-alive"
            | "te"
            | "upgrade"
            | "proxy-authenticate"
            | "proxy-authorization"
    )
}

fn parse_trailer_line(line: &[u8]) -> anyhow::Result<(String, String)> {
    let line = str::from_utf8(line)?.trim();
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid trailer field line"))?;
    let name = name.trim();
    if name.is_empty() {
        Err(anyhow!("empty trailer field name"))?;
    }
    Ok((name.to_string(), value.trim().to_string()))
}

/// Parse HTTP date format to Unix timestamp
/// Supports RFC 7231 standard HTTP date formats:
/// - RFC 1123: "Mon, 06 Nov 1994 08:49:37 GMT"
/// - RFC 850: "Monday, 06-Nov-94 08:49:37 GMT"
/// - ANSI C asctime(): "Mon Nov  6 08:49:37 1994"
pub fn parse_http_date(date_str: &str) -> Result<u64, ()> {
    // Use simple manual parsing method to handle RFC 1123 format
    // Format: "Fri, 12 Sep 2025 00:00:00 GMT"
    if let Some(caps) =
        regex::Regex::new(r"^\w+, (\d{1,2}) (\w+) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$")
            .unwrap()
            .captures(date_str)
    {
        let day: u32 = caps[1].parse().map_err(|_| ())?;
        let month_str = &caps[2];
        let year: i32 = caps[3].parse().map_err(|_| ())?;
        let hour: u32 = caps[4].parse().map_err(|_| ())?;
        let minute: u32 = caps[5].parse().map_err(|_| ())?;
        let second: u32 = caps[6].parse().map_err(|_| ())?;

        let month = match month_str {
            "Jan" => 1,
            "Feb" => 2,
            "Mar" => 3,
            "Apr" => 4,
            "May" => 5,
            "Jun" => 6,
            "Jul" => 7,
            "Aug" => 8,
            "Sep" => 9,
            "Oct" => 10,
            "Nov" => 11,
            "Dec" => 12,
            _ => return Err(()),
        };

        if let Some(dt) = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|d| d.and_hms_opt(hour, minute, second))
        {
            let timestamp = dt.and_utc().timestamp() as u64;
            return Ok(timestamp);
        }
    }

    // Try RFC 1123 format
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str, "%a, %d %b %Y %H:%M:%S GMT") {
        let timestamp = dt.timestamp() as u64;
        return Ok(timestamp);
    }

    // Try RFC 850 format
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str, "%A, %d-%b-%y %H:%M:%S GMT") {
        let timestamp = dt.timestamp() as u64;
        return Ok(timestamp);
    }

    // Try ANSI C asctime() format
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%a %b %e %H:%M:%S %Y") {
        let timestamp = dt.and_utc().timestamp() as u64;
        return Ok(timestamp);
    }

    Err(())
}

static SERVER_STR: LazyLock<String> =
    LazyLock::new(|| format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));

type AsyncHttpHandler =
    fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>;
type SyncHttpHandler = fn(&mut HttpRequest) -> HttpResponse;

#[derive(Clone, Copy)]
pub enum HttpHandler {
    Async(AsyncHttpHandler),
    Sync(SyncHttpHandler),
}

pub struct RequestHandlerFlagDoc {
    pub show: bool,
    pub auth: bool,
    pub summary: &'static str,
    pub desp: &'static str,
    pub args: &'static str,
}

impl RequestHandlerFlagDoc {
    pub const fn new(
        show: bool,
        auth: bool,
        summary: &'static str,
        desp: &'static str,
        args: &'static str,
    ) -> Self {
        RequestHandlerFlagDoc {
            show,
            auth,
            summary,
            desp,
            args,
        }
    }
}

pub struct RequestHandlerFlag {
    pub method: HttpMethod,
    pub path: &'static str,
    pub handler: HttpHandler,
    pub doc: RequestHandlerFlagDoc,
}

impl RequestHandlerFlag {
    pub const fn new(
        method: HttpMethod,
        path: &'static str,
        handler: HttpHandler,
        doc: RequestHandlerFlagDoc,
    ) -> Self {
        RequestHandlerFlag {
            method,
            path,
            handler,
            doc,
        }
    }
}

inventory::collect!(RequestHandlerFlag);

#[derive(Clone, Copy, Debug, Display, Eq, Hash, PartialEq)]
pub enum HttpMethod {
    GET,
    PUT,
    COPY,
    HEAD,
    LOCK,
    MOVE,
    POST,
    MKCOL,
    PATCH,
    TRACE,
    DELETE,
    UNLOCK,
    CONNECT,
    OPTIONS,
    PROPFIND,
    PROPPATCH,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpRequestTargetForm {
    Origin,
    Absolute,
    Authority,
    Asterisk,
}

#[derive(Debug, Eq, PartialEq)]
pub enum CompressMode {
    None,
    Gzip,
}

pub struct Websocket {
    stream: Arc<Mutex<HttpStream>>,
}

impl Websocket {
    pub async fn connect(url: &str, args: Vec<Headers>) -> anyhow::Result<Self> {
        let mut sess = Session::new();
        let mut req = sess.new_request(HttpMethod::GET, url).await?;
        for arg in args.into_iter() {
            req.apply_header(arg);
        }
        req.apply_header(Headers::Connection("Upgrade".to_string()));
        req.apply_header(Headers::Upgrade("Websocket".to_string()));
        req.apply_header(Headers::Sec_WebSocket_Version("13".to_string()));
        req.apply_header(Headers::Sec_WebSocket_Key("VerySecurity".to_string()));
        let res = sess.do_request(req).await?;
        if res.http_code != 101 {
            let body_str = match &res.body {
                HttpResponseBody::Data(data) => str::from_utf8(&data[..])?,
                HttpResponseBody::Stream(_) => "stream response",
            };
            Err(anyhow!(
                "Server return code[{}]: {}",
                res.http_code,
                body_str
            ))?;
        }
        let stream = sess
            .sess_impl
            .ok_or_else(|| anyhow!("session impl is null"))?
            .stream;
        let stream = Arc::new(Mutex::new(stream));
        Ok(Self { stream })
    }

    async fn recv_impl(&mut self) -> anyhow::Result<WsFrameImpl> {
        let mut stream = self.stream.lock().await;
        let buf = {
            let mut buf = [0u8; 2];
            stream.read_exact(&mut buf).await?;
            buf
        };
        //let fin = buf[0] & 0b1000_0000 != 0;
        let opcode = buf[0] & 0b0000_1111;
        let payload_len = {
            let payload_len = buf[1] & 0b0111_1111;
            match payload_len {
                126 => {
                    let mut buf = [0u8; 2];
                    stream.read_exact(&mut buf).await?;
                    u16::from_be_bytes(buf) as usize
                }
                127 => {
                    let mut buf = [0u8; 8];
                    stream.read_exact(&mut buf).await?;
                    u64::from_be_bytes(buf) as usize
                }
                _ => payload_len as usize,
            }
        };
        let omask_key = match buf[1] & 0b1000_0000 != 0 {
            true => {
                let mut mask_key = [0u8; 4];
                stream.read_exact(&mut mask_key).await?;
                Some(mask_key)
            }
            false => None,
        };
        let mut payload = vec![0u8; payload_len];
        stream.read_exact(&mut payload).await?;
        if let Some(mask_key) = omask_key {
            for i in 0..payload.len() {
                payload[i] ^= mask_key[i % 4];
            }
        }
        match opcode {
            0x0 => Ok(WsFrameImpl::PartData(payload)),
            0x1 => Ok(WsFrameImpl::Text(payload)),
            0x2 => Ok(WsFrameImpl::Binary(payload)),
            0x8 => Ok(WsFrameImpl::Close),
            0x9 => Ok(WsFrameImpl::Ping),
            0xA => Ok(WsFrameImpl::Pong),
            _ => Err(anyhow::Error::msg("unsupported opcode")),
        }
    }

    pub async fn recv(&mut self) -> anyhow::Result<WsFrame> {
        let mut tmp = vec![];
        loop {
            let timeout = ServerConfig::get_ws_ping_duration().await;
            match tokio::time::timeout(timeout, self.recv_impl()).await {
                Ok(ws_frame) => match ws_frame? {
                    WsFrameImpl::Close => return Err(anyhow::Error::msg("close frame")),
                    WsFrameImpl::Ping => self.send_impl(WsFrameImpl::Pong).await?,
                    WsFrameImpl::Pong => (),
                    WsFrameImpl::Binary(data) => {
                        tmp.extend(data);
                        return Ok(WsFrame::Binary(tmp));
                    }
                    WsFrameImpl::Text(data) => {
                        tmp.extend(data);
                        let ret_str = String::from_utf8(tmp).unwrap_or("".to_string());
                        return Ok(WsFrame::Text(ret_str));
                    }
                    WsFrameImpl::PartData(data) => tmp.extend(data),
                },
                Err(_) => self.send_impl(WsFrameImpl::Ping).await?,
            }
        }
    }

    async fn send_impl(&mut self, frame: WsFrameImpl) -> anyhow::Result<()> {
        let (fin, opcode, payload) = match frame {
            WsFrameImpl::Close => (true, 0x8, vec![]),
            WsFrameImpl::Ping => (true, 0x9, vec![]),
            WsFrameImpl::Pong => (true, 0xA, vec![]),
            WsFrameImpl::Binary(data) => (true, 0x2, data),
            WsFrameImpl::Text(data) => (true, 0x1, data),
            WsFrameImpl::PartData(data) => (false, 0x0, data),
        };
        let payload_len = payload.len();
        let mut buf = vec![];
        buf.push((fin as u8) << 7 | opcode);
        if payload_len < 126 {
            buf.push(payload_len as u8);
        } else if payload_len < 65536 {
            buf.push(126);
            buf.extend((payload_len as u16).to_be_bytes().iter());
        } else {
            buf.push(127);
            buf.extend((payload_len as u64).to_be_bytes().iter());
        }
        let mut stream = self.stream.lock().await;
        stream.write_all(&buf).await?;
        stream.write_all(&payload).await?;
        Ok(())
    }

    pub async fn send_ping(&mut self) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Ping).await
    }

    pub async fn send(&mut self, frame: WsFrame) -> anyhow::Result<()> {
        match frame {
            WsFrame::Binary(data) => self.send_impl(WsFrameImpl::Binary(data)).await,
            WsFrame::Text(text) => {
                self.send_impl(WsFrameImpl::Text(text.as_bytes().to_vec()))
                    .await
            }
        }
    }

    pub async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Binary(data)).await
    }

    pub async fn send_text(&mut self, data: &str) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Text(data.as_bytes().to_vec()))
            .await
    }
}

#[derive(Debug)]
pub enum WsFrame {
    Binary(Vec<u8>),
    Text(String),
}

pub enum WsFrameImpl {
    Close,
    Ping,
    Pong,
    Binary(Vec<u8>),
    Text(Vec<u8>),
    PartData(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct PostFile {
    pub filename: LocalHipStr<'static>,
    pub data: LocalHipByt<'static>,
}

unsafe impl Send for PostFile {}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub target_form: HttpRequestTargetForm,
    pub url_path: LocalHipStr<'static>,
    pub url_query: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    pub version: u8,
    pub headers: HashMap<HeaderOrHipStr, LocalHipStr<'static>>,
    pub trailers: HashMap<HeaderOrHipStr, LocalHipStr<'static>>,
    pub body: LocalHipByt<'static>,
    pub body_pairs: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    pub body_files: HashMap<LocalHipStr<'static>, PostFile>,
    pub client_addr: Option<SocketAddr>,
    pub exts: HashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>,
}

unsafe impl Send for HttpRequest {}
unsafe impl Sync for HttpRequest {}

impl HttpRequest {
    fn bad_request(msg: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(HttpRequestParseError::BadRequest(msg.into()))
    }

    fn expectation_failed(msg: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(HttpRequestParseError::ExpectationFailed(msg.into()))
    }

    fn not_implemented(msg: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(HttpRequestParseError::NotImplemented(msg.into()))
    }

    fn request_header_fields_too_large(msg: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(HttpRequestParseError::RequestHeaderFieldsTooLarge(msg.into()))
    }

    pub fn bad_request_message(err: &anyhow::Error) -> Option<&str> {
        err.downcast_ref::<HttpRequestParseError>()
            .and_then(|parse_err| match parse_err {
                HttpRequestParseError::BadRequest(msg) => Some(msg.as_str()),
                HttpRequestParseError::NotImplemented(_) => None,
                HttpRequestParseError::ExpectationFailed(_) => None,
                HttpRequestParseError::RequestHeaderFieldsTooLarge(_) => None,
            })
    }

    pub fn parse_error_response(err: &anyhow::Error) -> Option<HttpResponse> {
        err.downcast_ref::<HttpRequestParseError>()
            .map(|parse_err| {
                let (status, msg) = match parse_err {
                    HttpRequestParseError::BadRequest(msg) => (400, msg.as_str()),
                    HttpRequestParseError::NotImplemented(msg) => (501, msg.as_str()),
                    HttpRequestParseError::ExpectationFailed(msg) => (417, msg.as_str()),
                    HttpRequestParseError::RequestHeaderFieldsTooLarge(msg) => {
                        (431, msg.as_str())
                    }
                };
                let mut res = HttpResponse::text(msg.to_string());
                res.http_code = status;
                res.add_header("Connection".into(), "close".into());
                res
            })
    }

    fn ensure_header_section_size(buf: &[u8], max_header_bytes: usize) -> anyhow::Result<()> {
        let header_len = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(pos) => pos + 4,
            None => buf.len(),
        };
        if header_len > max_header_bytes {
            Err(Self::request_header_fields_too_large(format!(
                "request headers too large: {header_len} bytes exceeds {max_header_bytes} bytes"
            )))?;
        }
        Ok(())
    }

    async fn process_expect_header(
        &self,
        stream: &mut HttpStream,
        has_request_body: bool,
    ) -> anyhow::Result<()> {
        let Some(expect) = self.get_header_key(HeaderItem::Expect) else {
            return Ok(());
        };
        if self.version < 11 {
            Err(Self::expectation_failed(
                "Expect header is not supported for HTTP versions below 1.1",
            ))?;
        }
        let mut has_100_continue = false;
        for token in expect.split(',').map(|token| token.trim()) {
            if token.is_empty() {
                Err(Self::expectation_failed("empty Expect header"))?;
            }
            if token.eq_ignore_ascii_case("100-continue") {
                has_100_continue = true;
                continue;
            }
            Err(Self::expectation_failed(format!(
                "unsupported Expect header: {token}"
            )))?;
        }
        if has_100_continue && has_request_body {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            target_form: HttpRequestTargetForm::Origin,
            url_path: LocalHipStr::from("/"),
            url_query: HashMap::with_capacity(16),
            version: 11,
            headers: HashMap::with_capacity(16),
            trailers: HashMap::with_capacity(4),
            body: LocalHipByt::new(),
            body_pairs: HashMap::with_capacity(16),
            body_files: HashMap::with_capacity(4),
            client_addr: None,
            exts: HashMap::with_capacity(2),
        }
    }

    pub fn query_string(&self) -> String {
        let mut q = "?".to_string();
        for (k, v) in self.url_query.iter() {
            q.push_str(k);
            q.push('=');
            q.push_str(v);
            q.push('&');
        }
        q.pop();
        q
    }

    fn add_ext<T: Any + Send + Sync + 'static>(&mut self, item: Arc<T>) {
        let type_id = TypeId::of::<T>();
        self.exts.insert(type_id, item);
    }

    fn get_ext<T: Any + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.exts
            .get(&TypeId::of::<T>())
            .and_then(|any| any.clone().downcast().ok())
    }

    fn remove_ext<T: Any + Send + Sync + 'static>(&mut self) -> Option<Arc<T>> {
        self.exts
            .remove(&TypeId::of::<T>())
            .and_then(|any| any.clone().downcast().ok())
    }

    fn parse_path_and_query(&mut self, target: &str) {
        self.url_query.clear();
        match target.find('?') {
            Some(p) => {
                self.url_path = LocalHipStr::from(&target[..p]);
                self.url_query = target[p + 1..]
                    .split('&')
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| (LocalHipStr::from(a), LocalHipStr::from(b)))
                    .collect();
            }
            None => {
                self.url_path = LocalHipStr::from(target);
            }
        }
    }

    fn parse_request_target(&mut self, target: &str) -> anyhow::Result<()> {
        if target == "*" {
            if self.method != HttpMethod::OPTIONS {
                Err(Self::bad_request("asterisk-form request-target requires OPTIONS"))?;
            }
            self.target_form = HttpRequestTargetForm::Asterisk;
            self.url_query.clear();
            self.url_path = LocalHipStr::from("*");
            return Ok(());
        }

        if target.starts_with('/') {
            if self.method == HttpMethod::CONNECT {
                Err(Self::bad_request("CONNECT requires authority-form request-target"))?;
            }
            self.target_form = HttpRequestTargetForm::Origin;
            self.parse_path_and_query(target);
            return Ok(());
        }

        if target.contains("://") {
            if self.method == HttpMethod::CONNECT {
                Err(Self::bad_request(
                    "CONNECT requires authority-form, absolute-form is invalid",
                ))?;
            }
            let uri = target
                .parse::<Uri>()
                .map_err(|_| Self::bad_request("invalid absolute-form request-target"))?;
            if uri.scheme().is_none() || uri.authority().is_none() {
                Err(Self::bad_request(
                    "absolute-form request-target must include scheme and authority",
                ))?;
            }
            self.target_form = HttpRequestTargetForm::Absolute;
            let path_and_query = uri.path_and_query().map(|v| v.as_str()).unwrap_or("/");
            self.parse_path_and_query(path_and_query);
            return Ok(());
        }

        if http::uri::Authority::from_str(target).is_ok() {
            if self.method != HttpMethod::CONNECT {
                Err(Self::bad_request("authority-form request-target is only valid for CONNECT"))?;
            }
            self.target_form = HttpRequestTargetForm::Authority;
            self.url_query.clear();
            self.url_path = LocalHipStr::from(target);
            return Ok(());
        }

        Err(Self::bad_request("unsupported request-target form"))
    }

    pub fn get_uri(&self, is_https: bool) -> anyhow::Result<http::Uri> {
        let mut q = self.url_path.to_string();
        let mut is_first = true;
        for (k, v) in self.url_query.iter() {
            match is_first {
                true => {
                    is_first = false;
                    q.push('?');
                }
                false => q.push('&'),
            }
            q.push_str(k);
            q.push('=');
            q.push_str(v);
        }
        Ok(http::uri::Builder::new()
            .scheme(if is_https { "https" } else { "http" })
            .path_and_query(q)
            .build()?)
    }

    pub fn from_url(url: &str, method: HttpMethod) -> anyhow::Result<(Self, bool, u16)> {
        let uri = url.parse::<Uri>()?;
        let mut req = Self::new();
        req.method = method;
        req.url_path = LocalHipStr::from(uri.path());
        req.headers.insert(
            HeaderOrHipStr::from_str("Host"),
            uri.host().unwrap_or("localhost").into(),
        );
        let use_ssl = uri.scheme() == Some(&Scheme::HTTPS);
        let port = uri.port_u16().unwrap_or(if use_ssl { 443 } else { 80 });
        Ok((req, use_ssl, port))
    }

    pub fn set_header(
        &mut self,
        key: impl Into<HeaderOrHipStr>,
        value: impl Into<LocalHipStr<'static>>,
    ) {
        self.headers.insert(key.into(), value.into());
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        if let Some(header_item) = HeaderItem::try_from_str(key) {
            if let Some(value) = self.headers.get(&HeaderOrHipStr::HeaderItem(header_item)) {
                return Some(&value[..]);
            }
        }
        self.headers
            .get(&HeaderOrHipStr::HipStr(LocalHipStr::from(key)))
            .map(|a| &a[..])
    }

    pub fn get_header_key(&self, key: HeaderItem) -> Option<&str> {
        self.headers.get(&key.into()).map(|a| &a[..])
    }

    pub fn set_trailer(
        &mut self,
        key: impl Into<HeaderOrHipStr>,
        value: impl Into<LocalHipStr<'static>>,
    ) {
        self.trailers.insert(key.into(), value.into());
    }

    pub fn get_trailer(&self, key: &str) -> Option<&str> {
        if let Some(header_item) = HeaderItem::try_from_str(key) {
            if let Some(value) = self
                .trailers
                .get(&HeaderOrHipStr::HeaderItem(header_item))
            {
                return Some(&value[..]);
            }
        }
        self.trailers
            .get(&HeaderOrHipStr::HipStr(LocalHipStr::from(key)))
            .map(|a| &a[..])
    }

    pub fn get_header_accept_encoding(&self) -> CompressMode {
        Self::negotiate_accept_encoding(self.get_header_key(HeaderItem::Accept_Encoding).unwrap_or(""))
    }

    fn negotiate_accept_encoding(header: &str) -> CompressMode {
        let mut explicit_gzip_q: Option<u16> = None;
        let mut wildcard_q: Option<u16> = None;

        for item in header.split(',') {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut parts = trimmed.split(';');
            let coding = parts.next().unwrap_or("").trim().to_ascii_lowercase();
            if coding.is_empty() {
                continue;
            }

            let mut quality = 1000u16;
            let mut malformed_q = false;
            for param in parts {
                let param = param.trim();
                if param.is_empty() {
                    continue;
                }
                let mut key_val = param.splitn(2, '=');
                let key = key_val.next().unwrap_or("").trim().to_ascii_lowercase();
                if key != "q" {
                    continue;
                }
                let val = key_val.next().unwrap_or("").trim();
                if let Some(parsed_q) = Self::parse_qvalue_thousandths(val) {
                    quality = parsed_q;
                } else {
                    malformed_q = true;
                }
                break;
            }

            if malformed_q {
                continue;
            }

            match coding.as_str() {
                "gzip" => {
                    explicit_gzip_q = Some(explicit_gzip_q.map_or(quality, |prev| prev.max(quality)));
                }
                "*" => {
                    wildcard_q = Some(wildcard_q.map_or(quality, |prev| prev.max(quality)));
                }
                _ => {}
            }
        }

        let selected_q = explicit_gzip_q.or(wildcard_q).unwrap_or(0);
        if selected_q > 0 {
            CompressMode::Gzip
        } else {
            CompressMode::None
        }
    }

    fn parse_qvalue_thousandths(raw: &str) -> Option<u16> {
        let val = raw.trim();
        if val == "1" || val == "1.0" || val == "1.00" || val == "1.000" {
            return Some(1000);
        }
        if val == "0" {
            return Some(0);
        }
        let frac = val.strip_prefix("0.")?;
        if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let mut digits = frac.to_string();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits.parse::<u16>().ok()
    }

    pub fn get_header_host(&self) -> Option<&str> {
        self.get_header_key(HeaderItem::Host)
    }

    pub fn get_header_connection(&self) -> HttpConnection {
        if let Some(conn) = self.get_header_key(HeaderItem::Connection) {
            HttpConnection::from_str(conn).unwrap_or(HttpConnection::Close)
        } else if self.version >= 11 {
            HttpConnection::KeepAlive
        } else {
            HttpConnection::Close
        }
    }

    pub fn get_header_content_length(&self) -> usize {
        self.get_header_key(HeaderItem::Content_Length)
            .map_or(0, |val| val.parse::<usize>().unwrap_or(0))
    }

    fn parse_header_content_length(&self) -> anyhow::Result<Option<usize>> {
        let Some(raw_val) = self.get_header_key(HeaderItem::Content_Length) else {
            return Ok(None);
        };
        let value = raw_val.trim();
        if value.is_empty() {
            Err(anyhow!("empty Content-Length header"))?
        }
        Ok(Some(value.parse::<usize>()?))
    }

    fn has_chunked_transfer_encoding(&self) -> anyhow::Result<bool> {
        let Some(raw_val) = self.get_header_key(HeaderItem::Transfer_Encoding) else {
            return Ok(false);
        };
        let codings: Vec<String> = raw_val
            .split(',')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect();
        if codings.is_empty() {
            Err(Self::bad_request("empty Transfer-Encoding header"))?
        }
        if codings.len() == 1 && codings[0] == "chunked" {
            return Ok(true);
        }
        Err(Self::not_implemented(format!(
            "unsupported Transfer-Encoding: {raw_val}"
        )))
    }

    async fn read_chunked_body(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        hdr_len: usize,
        allowed_trailers: &HashSet<String>,
    ) -> anyhow::Result<(LocalHipByt<'static>, HashMap<HeaderOrHipStr, LocalHipStr<'static>>, usize)> {
        let mut cursor = hdr_len;
        let mut body = Vec::new();
        let mut trailers = HashMap::with_capacity(4);
        let mut tmp_buf = [0u8; 4096];

        loop {
            let line_end = loop {
                if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n") {
                    break cursor + pos;
                }
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            };

            let chunk_size = {
                let size_line = str::from_utf8(&buf[cursor..line_end])?;
                let size_token = size_line
                    .split_once(';')
                    .map_or(size_line, |(size, _)| size)
                    .trim();
                if size_token.is_empty() {
                    Err(anyhow!("invalid chunk size"))?;
                }
                usize::from_str_radix(size_token, 16)?
            };
            cursor = line_end + 2;

            if chunk_size == 0 {
                let trailer_end = loop {
                    let line_start = cursor;
                    let line_end = loop {
                        if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n")
                        {
                            break cursor + pos;
                        }
                        let n = stream.read(&mut tmp_buf).await?;
                        if n == 0 {
                            Err(anyhow::Error::msg("connection closed"))?;
                        }
                        buf.extend(&tmp_buf[..n]);
                    };
                    cursor = line_end + 2;
                    if line_end == line_start {
                        break cursor;
                    }

                    let (name, value) = parse_trailer_line(&buf[line_start..line_end])?;
                    let name_lower = name.to_ascii_lowercase();
                    if is_forbidden_trailer_field(&name_lower) {
                        Err(anyhow!("forbidden trailer field: {name}"))?;
                    }
                    if !allowed_trailers.contains(&name_lower) {
                        Err(anyhow!("unexpected trailer field: {name}"))?;
                    }
                    trailers.insert(HeaderOrHipStr::from_str(&name), value.into());
                };
                cursor = trailer_end;
                break;
            }

            while buf.len() < cursor + chunk_size + 2 {
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            }
            body.extend_from_slice(&buf[cursor..cursor + chunk_size]);
            if &buf[cursor + chunk_size..cursor + chunk_size + 2] != b"\r\n" {
                Err(anyhow!("invalid chunk terminator"))?;
            }
            cursor += chunk_size + 2;
        }

        Ok((LocalHipByt::from(body), trailers, cursor - hdr_len))
    }

    pub fn get_header_content_type<'a>(&'a self) -> Option<HttpContentType<'a>> {
        HttpContentType::from_str(self.get_header_key(HeaderItem::Content_Type).unwrap_or(""))
    }

    pub fn is_websocket(&self) -> bool {
        if self.method != HttpMethod::GET {
            return false;
        }
        if self.get_header_connection() != HttpConnection::Upgrade {
            return false;
        }
        if self
            .get_header_key(HeaderItem::Upgrade)
            .map_or(false, |val| val.to_lowercase() != "websocket")
        {
            return false;
        }
        if self
            .get_header("Sec-WebSocket-Version")
            .map_or(false, |val| val != "13")
        {
            return false;
        }
        if self
            .get_header("Sec-WebSocket-Key")
            .map_or(false, |val| val.len() == 0)
        {
            return false;
        }
        true
    }

    pub async fn upgrade_websocket(&mut self) -> anyhow::Result<Websocket> {
        if !self.is_websocket() {
            Err(anyhow!("it is not a websocket request"))?;
        }
        let ws_key = self
            .get_header("Sec-WebSocket-Key")
            .unwrap_or("")
            .to_string();
        // let ws_ext = req.get_header("Sec-WebSocket-Extensions").unwrap_or("".to_string());
        let stream = match self.remove_ext::<Mutex<HttpStream>>() {
            Some(stream) => stream,
            None => Err(anyhow!("connot get stream"))?,
        };
        {
            let mut stream = stream.lock().await;
            let res = HttpResponse::from_websocket(&ws_key);
            stream.write_all(&res.as_bytes(CompressMode::None)).await?;
        }
        Ok(Websocket { stream })
    }

    pub async fn get_client_addr(&self) -> anyhow::Result<SocketAddr> {
        if let Some(addr) = self.client_addr {
            return Ok(addr);
        }
        match self.get_ext::<SocketAddr>() {
            Some(addr) => Ok((*addr).clone()),
            None => Err(anyhow!("no addr info")),
        }
    }

    async fn from_stream_impl(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
    ) -> anyhow::Result<(Self, usize)> {
        let mut tmp_buf = [0u8; 4096];
        let (mut req, hdr_len) = loop {
            let n = stream.read(&mut tmp_buf).await?;
            if n == 0 {
                return Err(anyhow::Error::msg("connection closed"));
            }
            buf.extend(&tmp_buf[0..n]);
            match HttpRequest::from_headers_part(&buf[..])? {
                Some((req, hdr_len)) => break (req, hdr_len),
                None => continue,
            }
        };
        let has_chunked_transfer_encoding = req.has_chunked_transfer_encoding()?;
        let mut content_length = 0usize;
        if has_chunked_transfer_encoding {
            if req.get_header_key(HeaderItem::Content_Length).is_some() {
                Err(Self::bad_request(
                    "conflicting headers: Transfer-Encoding and Content-Length"
                ))?;
            }
        } else {
            content_length = req
                .parse_header_content_length()
                .map_err(|err| Self::bad_request(err.to_string()))?
                .unwrap_or(0);
        }

        let has_request_body = has_chunked_transfer_encoding || content_length > 0;
        req.process_expect_header(stream, has_request_body).await?;

        let bdy_len;
        if has_chunked_transfer_encoding {
            let allowed_trailers =
                parse_declared_trailer_names(req.get_header_key(HeaderItem::Trailer));
            let (body, trailers, consumed_len) =
                Self::read_chunked_body(buf, stream, hdr_len, &allowed_trailers)
                    .await
                    .map_err(|err| Self::bad_request(err.to_string()))?;
            req.body = body;
            req.trailers = trailers;
            bdy_len = consumed_len;
        } else {
            while hdr_len + content_length > buf.len() {
                let t = stream.read(&mut tmp_buf).await?;
                if t == 0 {
                    return Err(anyhow::Error::msg("connection closed"));
                }
                buf.extend(&tmp_buf[0..t]);
            }
            if content_length > 0 {
                req.body = LocalHipByt::from(&buf[hdr_len..hdr_len + content_length]);
            }
            bdy_len = content_length;
        }

        // 先获取Content-Type的字符串值，避免借用冲突
        let content_type_str = {
            req.get_header_key(HeaderItem::Content_Type)
                .map(|s| s.to_string())
        };

        // 根据内容类型字符串处理请求体
        if let Some(content_type_str) = content_type_str {
            // 解析内容类型
            let content_type_parsed = HttpContentType::from_str(&content_type_str);

            match content_type_parsed {
                Some(HttpContentType::ApplicationJson) => {
                    if let Ok(body_str) = std::str::from_utf8(&req.body) {
                        if let Ok(root) = serde_json::from_str::<serde_json::Value>(&body_str) {
                            if let serde_json::Value::Object(obj) = root {
                                for (k, v) in obj {
                                    req.body_pairs.insert(
                                        LocalHipStr::from(k),
                                        LocalHipStr::from(v.to_string()),
                                    );
                                }
                            }
                        }
                    }
                }
                Some(HttpContentType::ApplicationXWwwFormUrlencoded) => {
                    if let Ok(body_str) = std::str::from_utf8(&req.body) {
                        body_str.split('&').for_each(|s| {
                            if let Some((a, b)) = s.split_once('=') {
                                req.body_pairs
                                    .insert(a.url_decode().into(), b.url_decode().into());
                            }
                        });
                    }
                }
                Some(HttpContentType::MultipartFormData(boundary)) => {
                    if let Ok(body_str) = std::str::from_utf8(&req.body) {
                        let split_str = ssformat!(64, "--{boundary}");
                        for mut s in body_str.split(split_str.as_str()) {
                            if s == "--" {
                                break;
                            }
                            if s.ends_with("\r\n") {
                                s = &s[..s.len() - 2];
                            }
                            if let Some((key_str, content)) = s.split_once("\r\n\r\n") {
                                let keys: Vec<&str> = key_str
                                    .split("\r\n")
                                    .map(|p| p.split(";").collect::<Vec<_>>())
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .flatten()
                                    .collect();
                                let mut name = None;
                                let mut filename = None;
                                for key in keys.into_iter() {
                                    if let Some((k, v)) = key.trim().split_once('=') {
                                        if k == "name" {
                                            name = Some(LocalHipStr::from(&v[1..v.len() - 1]));
                                        } else if k == "filename" {
                                            filename = Some(LocalHipStr::from(&v[1..v.len() - 1]));
                                        }
                                    }
                                }
                                if let Some(name) = name {
                                    if let Some(filename) = filename {
                                        let data = LocalHipByt::from(content.as_bytes());
                                        req.body_files.insert(name, PostFile { filename, data });
                                    } else {
                                        req.body_pairs.insert(name, LocalHipStr::from(content));
                                    }
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        Ok((req, hdr_len + bdy_len))
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: Arc<Mutex<HttpStream>>,
    ) -> anyhow::Result<(Self, usize)> {
        let mut stream = stream.lock().await;
        Self::from_stream_impl(buf, &mut stream).await
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        let max_header_count = ServerConfig::get_max_header_count();
        let max_header_bytes = ServerConfig::get_max_header_bytes();
        Self::ensure_header_section_size(buf, max_header_bytes)?;

        let mut headers = vec![httparse::EMPTY_HEADER; max_header_count];
        let (rreq, n) = {
            let mut req = httparse::Request::new(&mut headers);
            let n = match httparse::ParserConfig::default().parse_request(&mut req, buf) {
                Ok(httparse::Status::Complete(n)) => n,
                Ok(httparse::Status::Partial) => return Ok(None),
                Err(httparse::Error::TooManyHeaders) => {
                    Err(Self::request_header_fields_too_large(format!(
                        "too many request headers: exceeds configured limit {max_header_count}"
                    )))?
                }
                Err(err) => Err(anyhow!(err))?,
            };
            (req, n)
        };
        let parsed_header_count = rreq.headers.iter().filter(|h| !h.name.is_empty()).count();
        if parsed_header_count > max_header_count {
            Err(Self::request_header_fields_too_large(format!(
                "too many request headers: {parsed_header_count} exceeds {max_header_count}"
            )))?;
        }
        let mut req = HttpRequest::new();
        let mut content_length_seen: Option<String> = None;
        let mut host_header_count = 0usize;
        let mut has_valid_host = false;
        req.method = {
            let method = rreq.method.unwrap();
            match method.len() {
                3 if method == "GET" => HttpMethod::GET,
                3 if method == "PUT" => HttpMethod::PUT,
                4 if method == "COPY" => HttpMethod::COPY,
                4 if method == "HEAD" => HttpMethod::HEAD,
                4 if method == "LOCK" => HttpMethod::LOCK,
                4 if method == "MOVE" => HttpMethod::MOVE,
                4 if method == "POST" => HttpMethod::POST,
                5 if method == "MKCOL" => HttpMethod::MKCOL,
                5 if method == "PATCH" => HttpMethod::PATCH,
                5 if method == "TRACE" => HttpMethod::TRACE,
                6 if method == "DELETE" => HttpMethod::DELETE,
                6 if method == "UNLOCK" => HttpMethod::UNLOCK,
                7 if method == "OPTIONS" => HttpMethod::OPTIONS,
                7 if method == "CONNECT" => HttpMethod::CONNECT,
                8 if method == "PROPFIND" => HttpMethod::PROPFIND,
                9 if method == "PROPPATCH" => HttpMethod::PROPPATCH,
                _ => {
                    Err(Self::not_implemented(format!("unsupported method: {method}")))?
                }
            }
        };
        let target = rreq.path.unwrap();
        req.parse_request_target(target)?;
        req.version = rreq.version.unwrap_or(1) + 10;
        for h in rreq.headers.iter() {
            if h.name == "" {
                break;
            }
            let header_value = str::from_utf8(h.value)?;
            let normalized_header_value = header_value.trim();
            if h.name.eq_ignore_ascii_case("Content-Length") {
                let cl = normalized_header_value;
                if cl.is_empty() {
                    Err(anyhow!("empty Content-Length header"))?;
                }
                if let Some(prev) = &content_length_seen {
                    if prev != cl {
                        Err(anyhow!("conflicting duplicate Content-Length headers"))?;
                    }
                } else {
                    content_length_seen = Some(cl.to_string());
                }
            }
            if h.name.eq_ignore_ascii_case("Host") {
                host_header_count += 1;
                if host_header_count > 1 {
                    Err(Self::bad_request("multiple Host headers are not allowed"))?;
                }
                if normalized_header_value.is_empty() {
                    Err(Self::bad_request("empty Host header"))?;
                }
                if http::uri::Authority::from_str(normalized_header_value).is_err() {
                    Err(Self::bad_request("invalid Host header"))?;
                }
                has_valid_host = true;
            }
            if h.name.eq_ignore_ascii_case("Expect") {
                let expect_key: HeaderOrHipStr = HeaderItem::Expect.into();
                if let Some(existing) = req.headers.get(&expect_key) {
                    req.headers.insert(
                        expect_key,
                        LocalHipStr::from(format!("{}, {}", existing.as_str(), normalized_header_value)),
                    );
                } else {
                    req.headers
                        .insert(expect_key, LocalHipStr::from(normalized_header_value));
                }
            } else {
                req.headers.insert(
                    HeaderOrHipStr::from_str(h.name),
                    LocalHipStr::from(normalized_header_value),
                );
            }
        }
        if req.version >= 11 && !has_valid_host {
            Err(Self::bad_request("missing required Host header"))?;
        }
        Ok(Some((req, n)))
    }

    /// Check HTTP conditional preflight headers to determine if special status codes should be returned
    ///
    /// This method handles the following HTTP conditional headers:
    /// - If-Modified-Since: Return 304 if resource is not modified
    /// - If-None-Match: Return 304 if ETag matches
    /// - If-Match: Return 412 if ETag doesn't match
    /// - If-Unmodified-Since: Return 412 if resource is modified
    ///
    /// # Parameters
    /// - `meta`: File metadata (optional)
    /// - `etag`: Resource's ETag value (optional)
    ///
    /// # Return Values
    /// - `PreflightResult::Proceed`: Pass preflight check, can continue processing
    /// - `PreflightResult::NotModified`: Should return 304 status code
    /// - `PreflightResult::PreconditionFailed`: Should return 412 status code
    pub fn check_precondition_headers(
        &self,
        meta: Option<&Metadata>,
        etag: Option<&str>,
    ) -> PreflightResult {
        use crate::utils::refstr::HeaderItem;
        use std::time::UNIX_EPOCH;

        let is_get_or_head = matches!(self.method, HttpMethod::GET | HttpMethod::HEAD);

        // Get file's last modified time (Unix timestamp in seconds)
        let last_modified_timestamp = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        // Check If-Match header (if exists and doesn't match, return 412)
        if let Some(if_match) = self.get_header_key(HeaderItem::If_Match) {
            if if_match != "*" {
                if let Some(current_etag) = etag {
                    // Parse ETag list in If-Match
                    let match_found = if_match
                        .split(',')
                        .map(|s| s.trim())
                        .any(|expected_etag| expected_etag == current_etag);

                    if !match_found {
                        return PreflightResult::PreconditionFailed;
                    }
                } else {
                    // No ETag but client requires match, return 412
                    return PreflightResult::PreconditionFailed;
                }
            }
        }

        // Check If-Unmodified-Since header (if resource is modified, return 412)
        if let Some(if_unmodified_since) = self.get_header_key(HeaderItem::If_Unmodified_Since) {
            if let Some(last_modified) = last_modified_timestamp {
                if let Ok(since_timestamp) = parse_http_date(if_unmodified_since) {
                    if last_modified > since_timestamp {
                        return PreflightResult::PreconditionFailed;
                    }
                }
            }
        }

        // Check If-None-Match header
        if let Some(if_none_match) = self.get_header_key(HeaderItem::If_None_Match) {
            if if_none_match == "*" {
                // If resource exists, GET/HEAD -> 304, others -> 412.
                if etag.is_some() || meta.is_some() {
                    return if is_get_or_head {
                        PreflightResult::NotModified
                    } else {
                        PreflightResult::PreconditionFailed
                    };
                }
            } else if let Some(current_etag) = etag {
                // Check if ETag is in If-None-Match list.
                let match_found = if_none_match
                    .split(',')
                    .map(|s| s.trim())
                    .any(|expected_etag| expected_etag == current_etag);

                if match_found {
                    return if is_get_or_head {
                        PreflightResult::NotModified
                    } else {
                        PreflightResult::PreconditionFailed
                    };
                }
            }
        }

        // Check If-Modified-Since header (if resource is not modified, return 304)
        // Note: only applies to GET/HEAD and only when there's no If-None-Match header.
        if is_get_or_head && self.get_header_key(HeaderItem::If_None_Match).is_none() {
            if let Some(if_modified_since) = self.get_header_key(HeaderItem::If_Modified_Since) {
                if let Some(last_modified) = last_modified_timestamp {
                    if let Ok(since_timestamp) = parse_http_date(if_modified_since) {
                        if last_modified <= since_timestamp {
                            return PreflightResult::NotModified;
                        }
                    }
                }
            }
        }

        PreflightResult::Proceed
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let use_chunked = self
            .get_header_key(HeaderItem::Transfer_Encoding)
            .map(|encodings| {
                encodings
                    .split(',')
                    .map(|coding| coding.trim())
                    .any(|coding| coding.eq_ignore_ascii_case("chunked"))
            })
            .unwrap_or(false);

        let declared_trailer_names = parse_declared_trailer_names(self.get_header_key(HeaderItem::Trailer));
        let mut outbound_trailers: Vec<(String, String)> = Vec::with_capacity(self.trailers.len());
        for (key, value) in self.trailers.iter() {
            let key_str = key.to_str();
            let lower = key_str.to_ascii_lowercase();
            if is_forbidden_trailer_field(&lower) {
                continue;
            }
            if !declared_trailer_names.is_empty() && !declared_trailer_names.contains(&lower) {
                continue;
            }
            outbound_trailers.push((key_str.to_string(), value.to_string()));
        }

        let mut req_str = format!("{} {} HTTP/1.1\r\n", self.method, self.url_path);
        for (k, v) in self.headers.iter() {
            if let HeaderOrHipStr::HeaderItem(HeaderItem::Content_Length) = k {
                continue;
            }
            req_str.push_str(&format!("{}: {v}\r\n", k.to_str()));
        }
        if use_chunked {
            if declared_trailer_names.is_empty() && !outbound_trailers.is_empty() {
                let trailer_names = outbound_trailers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                req_str.push_str(&format!("Trailer: {trailer_names}\r\n"));
            }
            req_str.push_str("\r\n");
            let mut ret = req_str.as_bytes().to_vec();
            if self.body.is_empty() {
                if outbound_trailers.is_empty() {
                    ret.extend_from_slice(b"0\r\n\r\n");
                } else {
                    ret.extend_from_slice(b"0\r\n");
                    for (name, value) in outbound_trailers.iter() {
                        ret.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
                    }
                    ret.extend_from_slice(b"\r\n");
                }
            } else {
                ret.extend_from_slice(format!("{:x}\r\n", self.body.len()).as_bytes());
                ret.extend(&self.body[..]);
                if outbound_trailers.is_empty() {
                    ret.extend_from_slice(b"\r\n0\r\n\r\n");
                } else {
                    ret.extend_from_slice(b"\r\n0\r\n");
                    for (name, value) in outbound_trailers.iter() {
                        ret.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
                    }
                    ret.extend_from_slice(b"\r\n");
                }
            }
            ret
        } else {
            req_str.push_str(&format!(
                "{}: {}\r\n",
                HeaderItem::Content_Length.to_str(),
                self.body.len()
            ));
            req_str.push_str("\r\n");
            let mut ret = req_str.as_bytes().to_vec();
            ret.extend(&self.body[..]);
            ret
        }
    }
}

#[derive(Debug)]
pub enum HttpResponseBody {
    Data(Vec<u8>),
    Stream(Receiver<Vec<u8>>),
}

#[derive(Debug)]
pub struct HttpResponse {
    pub version: String,
    pub http_code: u16,
    pub headers: HashMap<Cow<'static, str>, Cow<'static, str>>,
    pub trailers: HashMap<Cow<'static, str>, Cow<'static, str>>,
    pub body: HttpResponseBody,
}
unsafe impl Send for HttpResponse {}
unsafe impl Sync for HttpResponse {}
impl Clone for HttpResponse {
    fn clone(&self) -> Self {
        Self {
            version: self.version.clone(),
            http_code: self.http_code,
            headers: self.headers.clone(),
            trailers: self.trailers.clone(),
            body: match &self.body {
                HttpResponseBody::Data(data) => HttpResponseBody::Data(data.clone()),
                HttpResponseBody::Stream(_) => panic!("Cannot clone Stream response"),
            },
        }
    }
}

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: impl Into<String>) -> Self {
            let body = body.into();
            Self {
                version: "HTTP/1.1".into(),
                http_code: 200,
                headers: Self::default_headers($cnt_type),
                trailers: HashMap::with_capacity(4),
                body: HttpResponseBody::Data(body.as_bytes().to_vec()),
            }
        }
    };
}

macro_rules! make_resp_by_binary {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: &[u8]) -> Self {
            Self {
                version: "HTTP/1.1".into(),
                http_code: 200,
                headers: Self::default_headers($cnt_type),
                trailers: HashMap::with_capacity(4),
                body: HttpResponseBody::Data(body.to_vec()),
            }
        }
    };
}

impl HttpResponse {
    make_resp_by_text!(html, "text/html");
    make_resp_by_text!(css, "text/css");
    make_resp_by_text!(csv, "text/csv");
    make_resp_by_text!(js, "text/javascript");
    make_resp_by_text!(text, "text/plain");
    make_resp_by_text!(json, "application/json");
    make_resp_by_text!(xml, "application/xml");
    make_resp_by_binary!(png, "image/png");

    fn default_headers(
        cnt_type: impl Into<String>,
    ) -> HashMap<Cow<'static, str>, Cow<'static, str>> {
        let now = Utc::now();
        let current_ts = now.timestamp();

        static TL_TIMESTAMP: ThreadLocal<RefCell<(i64, Cow<'static, str>)>> = ThreadLocal::new();
        let mut tl_timestamp = TL_TIMESTAMP.get_or_default().borrow_mut();
        let date_str = if current_ts == tl_timestamp.0 {
            tl_timestamp.1.clone()
        } else {
            let new_date: Cow<'_, str> = now.format("%a, %d %b %Y %H:%M:%S GMT").to_string().into();
            *tl_timestamp = (current_ts, new_date.clone());
            new_date
        };

        [
            ("Date".into(), date_str),
            ("Server".into(), SERVER_STR.clone().into()),
            ("Connection".into(), "keep-alive".into()),
            ("Content-Type".into(), cnt_type.into().into()),
            ("Pragma".into(), "no-cache".into()),
            ("Cache-Control".into(), "no-cache".into()),
        ]
        .into()
    }

    pub fn new() -> Self {
        Self {
            version: "".into(),
            http_code: 0,
            headers: HashMap::with_capacity(16),
            trailers: HashMap::with_capacity(4),
            body: HttpResponseBody::Data(vec![]),
        }
    }

    pub fn add_header(&mut self, key: Cow<'static, str>, value: Cow<'static, str>) {
        self.headers.insert(key, value);
    }

    pub fn not_found() -> Self {
        let mut ret = Self::html("404 not found");
        ret.http_code = 404;
        ret
    }

    pub fn error(payload: impl Into<String>) -> Self {
        let mut ret = Self::html(payload);
        ret.http_code = 500;
        ret
    }

    pub fn empty() -> Self {
        Self::html("")
    }

    /// Create a SSE response with chunked transfer encoding
    pub fn sse(rx: Receiver<Vec<u8>>) -> Self {
        Self {
            version: "HTTP/1.1".into(),
            http_code: 200,
            headers: [
                ("Transfer-Encoding".into(), "chunked".into()),
                ("Content-Type".into(), "application/octet-stream".into()),
            ]
            .into(),
            trailers: HashMap::with_capacity(4),
            body: HttpResponseBody::Stream(rx),
        }
    }

    /// Create a SSE response with custom content type
    pub fn sse_with_content_type(rx: Receiver<Vec<u8>>, content_type: impl Into<String>) -> Self {
        Self {
            version: "HTTP/1.1".into(),
            http_code: 200,
            headers: [
                ("Transfer-Encoding".into(), "chunked".into()),
                ("Content-Type".into(), content_type.into().into()),
            ]
            .into(),
            trailers: HashMap::with_capacity(4),
            body: HttpResponseBody::Stream(rx),
        }
    }

    pub fn from_file(path: &str, download: bool, meta: Option<Metadata>) -> Self {
        let mut buffer = vec![];
        if let Ok(mut file) = File::open(path) {
            _ = file.read_to_end(&mut buffer);
        }
        Self::from_mem_file(path, buffer, download, meta)
    }

    pub fn from_mem_file(
        path: &str,
        data: Vec<u8>,
        download: bool,
        meta: Option<Metadata>,
    ) -> Self {
        if let Some(meta) = meta {
            let mut ret = Self::from_mem_file(path, data, download, None);
            // Add Last-Modified header
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_time =
                        chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH + duration);
                    ret.add_header(
                        "Last-Modified".into(),
                        modified_time
                            .format("%a, %d %b %Y %H:%M:%S GMT")
                            .to_string()
                            .into(),
                    );
                }
            }

            // Add ETag header (format: "hex-file-modified-time-hex-file-size")
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_secs = duration.as_secs();
                    let file_size = meta.len();
                    let etag = format!("\"{:x}-{:x}\"", modified_secs, file_size);
                    ret.add_header("ETag".into(), etag.into());
                }
            }
            ret
        } else {
            let mut ret = Self::empty();
            let mime_type = match path.split('.').last() {
                Some("css") => "text/css",
                Some("csv") => "text/csv",
                Some("htm") => "text/html",
                Some("html") => "text/html",
                Some("js") => "application/javascript",
                Some("json") => "application/json",
                Some("pdf") => "application/pdf",
                Some("xml") => "application/xml",
                _ if path.ends_with('/') => "text/html",
                _ => "application/octet-stream",
            };
            ret.add_header("Content-Type".into(), mime_type.into());
            if download {
                let file = match path.rfind('/') {
                    Some(p) => &path[p + 1..],
                    None => path,
                };
                if file.len() > 0 {
                    ret.add_header(
                        "Content-Disposition".into(),
                        format!("attachment; filename={file}").into(),
                    );
                }
            }
            ret.body = HttpResponseBody::Data(data);
            ret
        }
    }

    pub fn from_websocket(ws_key: &str) -> Self {
        #[allow(deprecated)]
        let ws_accept = {
            let mut sha1 = Sha1::default();
            sha1.update(ws_key);
            sha1.update(&b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"[..]);
            base64::encode(&sha1.finalize())
        };
        Self {
            version: "HTTP/1.1".into(),
            http_code: 101,
            headers: [
                (
                    "Date".into(),
                    Utc::now()
                        .format("%a, %d %b %Y %H:%M:%S GMT")
                        .to_string()
                        .into(),
                ),
                ("Server".into(), SERVER_STR.clone().into()),
                ("Connection".into(), "Upgrade".into()),
                ("Upgrade".into(), "websocket".into()),
                ("Sec-WebSocket-Accept".into(), ws_accept.into()),
            ]
            .into(),
            trailers: HashMap::with_capacity(4),
            body: HttpResponseBody::Data(vec![]),
        }
    }

    pub fn add_trailer(&mut self, key: Cow<'static, str>, value: Cow<'static, str>) {
        self.trailers.insert(key, value);
    }

    pub fn get_trailer(&self, key: &str) -> Option<&str> {
        self.trailers.get(key).map(|v| v.as_ref())
    }

    fn status_disallows_response_body(status: u16) -> bool {
        (100..200).contains(&status) || status == 204 || status == 304
    }

    fn method_disallows_response_body(request_method: Option<HttpMethod>) -> bool {
        request_method == Some(HttpMethod::HEAD)
    }

    pub fn as_bytes(&self, mut cmode: CompressMode) -> Vec<u8> {
        match &self.body {
            HttpResponseBody::Data(data) => {
                let suppress_body = Self::status_disallows_response_body(self.http_code);
                let mut payload_tmp: Vec<u8> = vec![];
                if cmode == CompressMode::Gzip
                    && data.len() >= 32
                    && self.get_header("Content-Encoding").is_none()
                    && !suppress_body
                {
                    if let Ok(compressed_data) = data.compress() {
                        payload_tmp = compressed_data;
                    }
                }
                let mut payload_ref = if payload_tmp.is_empty() {
                    cmode = CompressMode::None;
                    data.as_slice()
                } else {
                    payload_tmp.as_slice()
                };
                if suppress_body {
                    cmode = CompressMode::None;
                    payload_ref = &[];
                }
                //
                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                ret.push_str(&ssformat!(
                    64,
                    "{} {} {status_str}\r\n",
                    self.version,
                    self.http_code
                ));
                for (key, value) in self.headers.iter() {
                    if key == "Content-Length"
                        || (suppress_body && key.eq_ignore_ascii_case("Transfer-Encoding"))
                    {
                        continue;
                    }
                    ret.push_str(&ssformat!(512, "{key}: {value}\r\n"));
                }
                if !suppress_body {
                    ret.push_str(&ssformat!(64, "Content-Length: {}\r\n", payload_ref.len()));
                }
                if cmode == CompressMode::Gzip && !suppress_body {
                    ret.push_str("Content-Encoding: gzip\r\n");
                }
                ret.push_str("\r\n");
                let mut ret: Vec<u8> = ret.as_bytes().to_vec();
                ret.extend(payload_ref);
                ret
            }
            HttpResponseBody::Stream(_) => vec![], // Stream responses are handled separately
        }
    }

    /// Write response to stream, handling both Data and Stream body types
    pub async fn write_to_stream(
        &mut self,
        stream: &mut crate::utils::tcp_stream::HttpStream,
        cmode: CompressMode,
        request_method: Option<HttpMethod>,
    ) -> anyhow::Result<()> {
        let suppress_body_by_status = Self::status_disallows_response_body(self.http_code);
        let suppress_body_by_method = Self::method_disallows_response_body(request_method);
        let suppress_body = suppress_body_by_status || suppress_body_by_method;
        let no_content_encoding = self.get_header("Content-Encoding").is_none();
        let declared_trailer_names = parse_declared_trailer_names(self.get_header("Trailer"));
        let mut outbound_stream_trailers: Vec<(String, String)> =
            Vec::with_capacity(self.trailers.len());
        for (key, value) in self.trailers.iter() {
            let lower = key.to_ascii_lowercase();
            if is_forbidden_trailer_field(&lower) {
                continue;
            }
            if !declared_trailer_names.is_empty() && !declared_trailer_names.contains(&lower) {
                continue;
            }
            outbound_stream_trailers.push((key.to_string(), value.to_string()));
        }
        match &mut self.body {
            HttpResponseBody::Data(data) => {
                let mut payload_tmp: Vec<u8> = vec![];
                let mut cmode = cmode;
                if cmode == CompressMode::Gzip
                    && data.len() >= 32
                    && no_content_encoding
                    && !suppress_body_by_status
                {
                    if let Ok(compressed_data) = data.compress() {
                        payload_tmp = compressed_data;
                    }
                }
                let mut payload_ref = if payload_tmp.is_empty() {
                    cmode = CompressMode::None;
                    data.as_slice()
                } else {
                    payload_tmp.as_slice()
                };
                if suppress_body_by_status {
                    cmode = CompressMode::None;
                    payload_ref = &[];
                }

                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                ret.push_str(&ssformat!(
                    64,
                    "{} {} {status_str}\r\n",
                    self.version,
                    self.http_code
                ));
                if self.headers.len() == 6 {
                    if let (
                        Some(date),
                        Some(server),
                        Some(connection),
                        Some(content_type),
                        Some(pragma),
                        Some(cache_control),
                    ) = (
                        self.headers.get("Date"),
                        self.headers.get("Server"),
                        self.headers.get("Connection"),
                        self.headers.get("Content-Type"),
                        self.headers.get("Pragma"),
                        self.headers.get("Cache-Control"),
                    ) {
                        ret.push_str(&ssformat!(512, "Date: {date}\r\n"));
                        ret.push_str(&ssformat!(512, "Server: {server}\r\n"));
                        ret.push_str(&ssformat!(512, "Connection: {connection}\r\n"));
                        ret.push_str(&ssformat!(512, "Content-Type: {content_type}\r\n"));
                        ret.push_str(&ssformat!(512, "Pragma: {pragma}\r\n"));
                        ret.push_str(&ssformat!(512, "Cache-Control: {cache_control}\r\n"));
                    } else {
                        for (key, value) in self.headers.iter() {
                            if key == "Content-Length"
                                || (suppress_body && key.eq_ignore_ascii_case("Transfer-Encoding"))
                            {
                                continue;
                            }
                            ret.push_str(&ssformat!(512, "{key}: {value}\r\n"));
                        }
                    }
                } else {
                    for (key, value) in self.headers.iter() {
                        if key == "Content-Length"
                            || (suppress_body && key.eq_ignore_ascii_case("Transfer-Encoding"))
                        {
                            continue;
                        }
                        ret.push_str(&ssformat!(512, "{key}: {value}\r\n"));
                    }
                }
                if !suppress_body_by_status {
                    ret.push_str(&ssformat!(64, "Content-Length: {}\r\n", payload_ref.len()));
                }
                if cmode == CompressMode::Gzip && !suppress_body_by_status {
                    ret.push_str("Content-Encoding: gzip\r\n");
                }
                ret.push_str("\r\n");

                if suppress_body || payload_ref.is_empty() {
                    stream.write_all(ret.as_bytes()).await?;
                } else {
                    stream
                        .write_all_vectored2(ret.as_bytes(), payload_ref)
                        .await?;
                }
            }
            HttpResponseBody::Stream(rx) => {
                // For Stream body, send headers first, then chunks
                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                ret.push_str(&ssformat!(
                    64,
                    "{} {} {status_str}\r\n",
                    self.version,
                    self.http_code
                ));
                for (key, value) in self.headers.iter() {
                    if key == "Content-Length"
                        || (suppress_body && key.eq_ignore_ascii_case("Transfer-Encoding"))
                    {
                        continue;
                    }
                    ret.push_str(&ssformat!(512, "{key}: {value}\r\n"));
                }
                if declared_trailer_names.is_empty() && !outbound_stream_trailers.is_empty() {
                    let trailer_names = outbound_stream_trailers
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    ret.push_str(&ssformat!(512, "Trailer: {trailer_names}\r\n"));
                }
                ret.push_str("\r\n");
                let header_bytes: Vec<u8> = ret.as_bytes().to_vec();
                stream.write_all(&header_bytes).await?;

                if suppress_body {
                    return Ok(());
                }

                // Send chunks
                while let Some(chunk) = rx.recv().await {
                    if chunk.is_empty() {
                        break;
                    }
                    // Write chunked encoding: length in hex, \r\n, data, \r\n
                    let chunk_len_hex = format!("{:x}\r\n", chunk.len());
                    stream.write_all(chunk_len_hex.as_bytes()).await?;
                    stream.write_all(&chunk).await?;
                    stream.write_all(b"\r\n").await?;
                }
                // Send final chunk (length 0)
                if outbound_stream_trailers.is_empty() {
                    stream.write_all(b"0\r\n\r\n").await?;
                } else {
                    stream.write_all(b"0\r\n").await?;
                    for (name, value) in outbound_stream_trailers.iter() {
                        stream
                            .write_all(format!("{name}: {value}\r\n").as_bytes())
                            .await?;
                    }
                    stream.write_all(b"\r\n").await?;
                }
            }
        }
        Ok(())
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
    ) -> anyhow::Result<(Self, usize)> {
        Self::from_stream_with_request_method(buf, stream, None).await
    }

    pub async fn from_stream_with_request_method(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        request_method: Option<HttpMethod>,
    ) -> anyhow::Result<(Self, usize)> {
        let (mut res, hdr_len) = loop {
            buf.extend_by_streams(stream).await?;
            match HttpResponse::from_headers_part(&buf[..])? {
                Some((res, hdr_len)) => break (res, hdr_len),
                None => continue,
            }
        };
        let mut bdy_len = 0;
        let skip_body = request_method == Some(HttpMethod::HEAD)
            || Self::status_disallows_response_body(res.http_code);
        if skip_body {
            return Ok((res, hdr_len));
        }
        if let Some(cnt_len) = res.headers.get("Content-Length") {
            bdy_len = cnt_len.parse::<usize>().unwrap_or(0);
            while hdr_len + bdy_len > buf.len() {
                buf.extend_by_streams(stream).await?;
            }
            res.body = HttpResponseBody::Data(buf[hdr_len..hdr_len + bdy_len].to_vec());
        // to_ref_buf
        } else if res
            .headers
            .get("Transfer-Encoding")
            .is_some_and(|v| v == "chunked")
        {
            let mut chunked_body = Vec::new();
            let allowed_trailers = parse_declared_trailer_names(res.get_header("Trailer"));
            loop {
                let chunked_len = {
                    let mut chunked_len = 0;
                    let mut is_fin = false;
                    for c in buf[(hdr_len + bdy_len)..].iter() {
                        bdy_len += 1;
                        match *c {
                            b'\r' => continue,
                            b'\n' => {
                                is_fin = true;
                                break;
                            }
                            b'0'..=b'9' => chunked_len = chunked_len * 16 + (*c - b'0') as usize,
                            b'a'..=b'z' => {
                                chunked_len = chunked_len * 16 + (*c - b'a' + 10) as usize
                            }
                            b'A'..=b'Z' => {
                                chunked_len = chunked_len * 16 + (*c - b'A' + 10) as usize
                            }
                            _ => Err(anyhow!("unexpected char: {}", *c as char))?,
                        }
                    }
                    if !is_fin {
                        buf.extend_by_streams(stream).await?;
                        continue;
                    }
                    chunked_len
                };
                if chunked_len == 0 {
                    loop {
                        loop {
                            if buf[(hdr_len + bdy_len)..].windows(2).any(|part| part == b"\r\n") {
                                break;
                            }
                            buf.extend_by_streams(stream).await?;
                        }

                        let line_start = hdr_len + bdy_len;
                        let rel_line_end = buf[line_start..]
                            .windows(2)
                            .position(|part| part == b"\r\n")
                            .ok_or_else(|| anyhow!("invalid trailer terminator"))?;
                        let line_end = line_start + rel_line_end;
                        bdy_len += rel_line_end + 2;
                        if rel_line_end == 0 {
                            break;
                        }

                        let (name, value) = parse_trailer_line(&buf[line_start..line_end])?;
                        let name_lower = name.to_ascii_lowercase();
                        if is_forbidden_trailer_field(&name_lower) {
                            Err(anyhow!("forbidden trailer field: {name}"))?;
                        }
                        if !allowed_trailers.contains(&name_lower) {
                            Err(anyhow!("unexpected trailer field: {name}"))?;
                        }
                        res.trailers.insert(name.into(), value.into());
                    }
                    break;
                }
                while hdr_len + bdy_len + chunked_len + 2 > buf.len() {
                    buf.extend_by_streams(stream).await?;
                }
                chunked_body.extend(&buf[(hdr_len + bdy_len)..(hdr_len + bdy_len + chunked_len)]);
                bdy_len += chunked_len + 2;
            }
            res.body = HttpResponseBody::Data(chunked_body);
        }

        Ok((res, hdr_len + bdy_len))
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        let mut headers = [httparse::EMPTY_HEADER; 96];
        let (rres, n) = {
            let mut res = httparse::Response::new(&mut headers);
            let n = match httparse::ParserConfig::default().parse_response(&mut res, buf)? {
                httparse::Status::Complete(n) => n,
                httparse::Status::Partial => return Ok(None),
            };
            (res, n)
        };

        let mut req = HttpResponse::new();
        req.version = format!("HTTP/1.{}", rres.version.unwrap_or(0));
        req.http_code = rres.code.unwrap_or(0);
        for h in rres.headers.iter() {
            if h.name == "" {
                break;
            }
            req.headers.insert(
                h.name.http_std_case().into(),
                str::from_utf8(h.value).unwrap_or("").to_string().into(),
            );
        }
        Ok(Some((req, n)))
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        let header_key = key.http_std_case();
        self.headers.get(header_key.as_str()).map(|a| a.as_ref())
    }
}

pub fn load_embed<T: Embed>() -> HashMap<String, Cow<'static, [u8]>> {
    let mut ret = HashMap::with_capacity(16);
    for name in T::iter().into_iter() {
        if let Some(file) = T::get(&name) {
            if name.ends_with("index.htm") || name.ends_with("index.html") {
                if let Some(path) = Path::new(&name[..]).parent() {
                    if let Some(path) = path.to_str() {
                        ret.insert(path.to_string(), file.data.clone());
                    }
                }
            }
            ret.insert(name.to_string(), file.data);
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::{CompressMode, HttpRequest};

    #[test]
    fn accept_encoding_supports_simple_gzip_token() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_supports_qvalue_for_gzip() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "br;q=1, gzip;q=0.3");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_uses_wildcard_when_gzip_not_listed() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "br;q=1, *;q=0.8");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_respects_explicit_gzip_zero_over_wildcard() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip;q=0, *;q=1");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
    }

    #[test]
    fn accept_encoding_ignores_invalid_qvalue() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip;q=xyz");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
    }

    #[test]
    fn request_parser_returns_431_for_too_many_headers() {
        let mut raw = String::from("GET / HTTP/1.1\r\nHost: example.com\r\n");
        for i in 0..64 {
            raw.push_str(&format!("X-Header-{i}: value\r\n"));
        }
        raw.push_str("\r\n");

        let err = HttpRequest::from_headers_part(raw.as_bytes()).unwrap_err();
        let res = HttpRequest::parse_error_response(&err).unwrap();
        assert_eq!(res.http_code, 431);
    }

    #[test]
    fn request_parser_returns_431_for_oversized_header_section() {
        let oversized = "a".repeat(20 * 1024);
        let raw = format!(
            "GET / HTTP/1.1\r\nHost: example.com\r\nX-Large: {oversized}\r\n\r\n"
        );

        let err = HttpRequest::from_headers_part(raw.as_bytes()).unwrap_err();
        let res = HttpRequest::parse_error_response(&err).unwrap();
        assert_eq!(res.http_code, 431);
    }
}
