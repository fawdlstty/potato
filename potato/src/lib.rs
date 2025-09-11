pub mod client;
pub mod global_config;
pub mod server;
pub mod utils;

pub use client::*;
pub use global_config::*;
pub use inventory;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
pub use serde_json;
pub use server::*;
use tokio::sync::Mutex;
pub use utils::refstr::Headers;

#[cfg(feature = "jemalloc")]
pub use utils::jemalloc_helper::*;

use anyhow::{anyhow, Error};
use chrono::Utc;
use core::str;
use http::uri::Scheme;
use http::Uri;
use rust_embed::Embed;
use sha1::{Digest, Sha1};
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::fs::{File, Metadata};
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, future::Future, pin::Pin};
use strum::Display;
use utils::bytes::CompressExt;
use utils::enums::{HttpConnection, HttpContentType};
use utils::number::HttpCodeExt;
use utils::refbuf::{RefOrBuffer, ToRefBufExt};
use utils::refstr::{HeaderItem, HeaderRefOrString, RefOrString, ToRefStrExt};
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

type HttpHandler =
    fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + Sync + '_>>;

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
        let mut req = sess.start_request(HttpMethod::GET, url).await?;
        for arg in args.into_iter() {
            req.apply_header(arg);
        }
        req.apply_header(Headers::Connection("Upgrade".to_string()));
        req.apply_header(Headers::Upgrade("Websocket".to_string()));
        req.apply_header(Headers::Sec_WebSocket_Version("13".to_string()));
        req.apply_header(Headers::Sec_WebSocket_Key("VerySecurity".to_string()));
        let res = sess.end_request(req).await?;
        if res.http_code != 101 {
            Err(anyhow!(
                "Server return code[{}]: {}",
                res.http_code,
                str::from_utf8(&res.body)?
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
    pub filename: RefOrString,
    pub data: RefOrBuffer,
}

unsafe impl Send for PostFile {}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url_path: RefOrString,
    pub url_query: HashMap<RefOrString, RefOrString>,
    pub version: u8,
    pub headers: HashMap<HeaderRefOrString, RefOrString>,
    pub body: RefOrBuffer,
    pub body_pairs: HashMap<RefOrString, RefOrString>,
    pub body_files: HashMap<RefOrString, PostFile>,
    pub exts: HashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>,
}

unsafe impl Send for HttpRequest {}
unsafe impl Sync for HttpRequest {}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            url_path: "/".to_ref_string(),
            url_query: HashMap::with_capacity(16),
            version: 11,
            headers: HashMap::with_capacity(16),
            body: [].to_ref_buffer(),
            body_pairs: HashMap::with_capacity(16),
            body_files: HashMap::with_capacity(4),
            exts: HashMap::new(),
        }
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
            q.push_str(k.to_str());
            q.push('=');
            q.push_str(v.to_str());
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
        req.url_path = uri.path().to_ref_string();
        req.headers
            .insert("Host".into(), uri.host().unwrap_or("localhost").into());
        let use_ssl = uri.scheme() == Some(&Scheme::HTTPS);
        let port = uri.port_u16().unwrap_or(if use_ssl { 443 } else { 80 });
        Ok((req, use_ssl, port))
    }

    pub fn set_header(&mut self, key: impl Into<HeaderRefOrString>, value: impl Into<RefOrString>) {
        self.headers.insert(key.into(), value.into());
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers
            .get(&key.to_ref_string().into())
            .map(|a| a.to_str())
    }

    pub fn get_header_key(&self, key: HeaderItem) -> Option<&str> {
        self.headers.get(&key.into()).map(|a| a.to_str())
    }

    pub fn get_header_accept_encoding(&self) -> CompressMode {
        for item in self
            .get_header_key(HeaderItem::Accept_Encoding)
            .unwrap_or("")
            .split(',')
        {
            match item.trim() {
                "gzip" => return CompressMode::Gzip,
                _ => continue,
            };
        }
        CompressMode::None
    }

    pub fn get_header_host(&self) -> Option<&str> {
        self.get_header_key(HeaderItem::Host)
    }

    pub fn get_header_connection(&self) -> HttpConnection {
        if let Some(conn) = self.get_header_key(HeaderItem::Connection) {
            HttpConnection::from_str(conn).unwrap_or(HttpConnection::Close)
        } else {
            HttpConnection::Close
        }
    }

    pub fn get_header_content_length(&self) -> usize {
        self.get_header_key(HeaderItem::Content_Length)
            .map_or(0, |val| val.parse::<usize>().unwrap_or(0))
    }

    pub fn get_header_content_type(&self) -> Option<HttpContentType> {
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
        match self.get_ext::<SocketAddr>() {
            Some(addr) => Ok((*addr).clone()),
            None => Err(anyhow!("no addr info")),
        }
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: Arc<Mutex<HttpStream>>,
    ) -> anyhow::Result<(Self, usize)> {
        let mut stream = stream.lock().await;
        let mut tmp_buf = [0u8; 1024];
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
        let bdy_len = req.get_header_content_length();
        while hdr_len + bdy_len > buf.len() {
            let t = stream.read(&mut tmp_buf).await?;
            if t == 0 {
                return Err(anyhow::Error::msg("connection closed"));
            }
            buf.extend(&tmp_buf[0..t]);
        }
        req.body = buf[hdr_len..hdr_len + bdy_len].to_ref_buffer();
        if let Some(cnt_type) = req.get_header_content_type() {
            match cnt_type {
                HttpContentType::ApplicationJson => {
                    if let Ok(body_str) = std::str::from_utf8(req.body.to_buf()) {
                        if let Ok(root) = serde_json::from_str::<serde_json::Value>(&body_str) {
                            if let serde_json::Value::Object(obj) = root {
                                for (k, v) in obj {
                                    req.body_pairs.insert(k.into(), v.to_string().into());
                                }
                            }
                        }
                    }
                }
                HttpContentType::ApplicationXWwwFormUrlencoded => {
                    if let Ok(body_str) = std::str::from_utf8(req.body.to_buf()) {
                        body_str.split('&').for_each(|s| {
                            if let Some((a, b)) = s.split_once('=') {
                                req.body_pairs
                                    .insert(a.url_decode().into(), b.url_decode().into());
                            }
                        });
                    }
                }
                HttpContentType::MultipartFormData(boundary) => {
                    let boundary = boundary.to_str();
                    if let Ok(body_str) = std::str::from_utf8(req.body.to_buf()) {
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
                                            name = Some(v[1..v.len() - 1].to_ref_string());
                                        } else if k == "filename" {
                                            filename = Some(v[1..v.len() - 1].to_ref_string());
                                        }
                                    }
                                }
                                if let Some(name) = name {
                                    if let Some(filename) = filename {
                                        let data = content.as_bytes().to_ref_buffer();
                                        req.body_files.insert(name, PostFile { filename, data });
                                    } else {
                                        req.body_pairs.insert(name, content.to_ref_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok((req, hdr_len + bdy_len))
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        let mut headers = [httparse::EMPTY_HEADER; 96];
        let (rreq, n) = {
            let mut req = httparse::Request::new(&mut headers);
            let n = match httparse::ParserConfig::default().parse_request(&mut req, buf)? {
                httparse::Status::Complete(n) => n,
                httparse::Status::Partial => return Ok(None),
            };
            (req, n)
        };

        let mut req = HttpRequest::new();
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
                _ => return Err(Error::msg(format!("unrecognized method: {method}"))),
            }
        };
        let url = rreq.path.unwrap();
        match url.find('?') {
            Some(p) => {
                req.url_path = url[..p].to_ref_string();
                req.url_query = url[p + 1..]
                    .split('&')
                    .into_iter()
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| (a.to_ref_string(), b.to_ref_string()))
                    .collect();
            }
            None => req.url_path = url.to_ref_string(),
        }
        req.version = rreq.version.unwrap_or(1) + 10;
        for h in rreq.headers.iter() {
            if h.name == "" {
                break;
            }
            req.headers
                .insert(h.name.to_header_ref_string(), h.value.to_ref_string());
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

        // Check If-None-Match header (if ETag matches, return 304)
        if let Some(if_none_match) = self.get_header_key(HeaderItem::If_None_Match) {
            if if_none_match == "*" {
                // If "*" and resource exists, return 304
                if etag.is_some() || meta.is_some() {
                    return PreflightResult::NotModified;
                }
            } else if let Some(current_etag) = etag {
                // Check if ETag is in If-None-Match list
                let match_found = if_none_match
                    .split(',')
                    .map(|s| s.trim())
                    .any(|expected_etag| expected_etag == current_etag);

                if match_found {
                    return PreflightResult::NotModified;
                }
            }
        }

        // Check If-Modified-Since header (if resource is not modified, return 304)
        // Note: Only check If-Modified-Since when there's no If-None-Match header
        if self.get_header_key(HeaderItem::If_None_Match).is_none() {
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
        let mut req_str = format!("{} {} HTTP/1.1\r\n", self.method, self.url_path.to_str());
        for (k, v) in self.headers.iter() {
            if let HeaderRefOrString::HeaderItem(HeaderItem::Content_Length) = k {
                continue;
            }
            req_str.push_str(&format!("{}: {}\r\n", k.to_str(), v.to_str()));
        }
        req_str.push_str(&format!(
            "{}: {}\r\n",
            HeaderItem::Content_Length.to_str(),
            self.body.to_buf().len()
        ));
        req_str.push_str("\r\n");
        let mut ret = req_str.as_bytes().to_vec();
        ret.extend(self.body.to_buf());
        ret
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub version: String,
    pub http_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
unsafe impl Send for HttpResponse {}
unsafe impl Sync for HttpResponse {}

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: impl Into<String>) -> Self {
            let body = body.into();
            Self {
                version: "HTTP/1.1".into(),
                http_code: 200,
                headers: Self::default_headers($cnt_type),
                body: body.as_bytes().to_vec(),
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
                body: body.to_vec(),
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

    fn default_headers(cnt_type: impl Into<String>) -> HashMap<String, String> {
        [
            (
                "Date".into(),
                Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
            ),
            ("Server".into(), SERVER_STR.clone()),
            ("Connection".into(), "keep-alive".into()),
            ("Content-Type".into(), cnt_type.into()),
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
            body: vec![],
        }
    }

    pub fn add_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers.insert(key.into(), value.into());
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
                        "Last-Modified",
                        modified_time
                            .format("%a, %d %b %Y %H:%M:%S GMT")
                            .to_string(),
                    );
                }
            }

            // Add ETag header (format: "hex-file-modified-time-hex-file-size")
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_secs = duration.as_secs();
                    let file_size = meta.len();
                    let etag = format!("\"{:x}-{:x}\"", modified_secs, file_size);
                    ret.add_header("ETag", etag);
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
            ret.add_header("Content-Type", mime_type);
            if download {
                let file = match path.rfind('/') {
                    Some(p) => &path[p + 1..],
                    None => path,
                };
                if file.len() > 0 {
                    ret.add_header(
                        "Content-Disposition",
                        format!("attachment; filename={file}"),
                    );
                }
            }
            ret.body = data;
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
                    Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                ),
                ("Server".into(), SERVER_STR.clone()),
                ("Connection".into(), "Upgrade".into()),
                ("Upgrade".into(), "websocket".into()),
                ("Sec-WebSocket-Accept".into(), ws_accept.into()),
            ]
            .into(),
            body: vec![],
        }
    }

    pub fn as_bytes(&self, mut cmode: CompressMode) -> Vec<u8> {
        let mut payload_tmp = vec![];
        let mut payload_ref = &self.body;
        if cmode == CompressMode::Gzip
            && self.body.len() >= 32
            && self.get_header("Content-Encoding").is_none()
        {
            if let Ok(data) = self.body.compress() {
                payload_tmp = data;
                payload_ref = &payload_tmp;
            }
        }
        if payload_tmp.len() == 0 {
            cmode = CompressMode::None;
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
            if key == "Content-Length" {
                continue;
            }
            ret.push_str(&ssformat!(512, "{key}: {value}\r\n"));
        }
        if self.http_code != 101 {
            ret.push_str(&ssformat!(64, "Content-Length: {}\r\n", payload_ref.len()));
            if cmode == CompressMode::Gzip {
                ret.push_str("Content-Encoding: gzip\r\n");
            }
        }
        ret.push_str("\r\n");
        let mut ret: Vec<u8> = ret.as_bytes().to_vec();
        ret.extend(payload_ref);
        ret
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
    ) -> anyhow::Result<(Self, usize)> {
        let (mut res, hdr_len) = loop {
            buf.extend_by_streams(stream).await?;
            match HttpResponse::from_headers_part(&buf[..])? {
                Some((res, hdr_len)) => break (res, hdr_len),
                None => continue,
            }
        };
        let mut bdy_len = 0;
        if let Some(cnt_len) = res.headers.get("Content-Length") {
            bdy_len = cnt_len.parse::<usize>().unwrap_or(0);
            while hdr_len + bdy_len > buf.len() {
                buf.extend_by_streams(stream).await?;
            }
            res.body = buf[hdr_len..hdr_len + bdy_len].to_vec(); // to_ref_buf
        } else if res.headers.get("Transfer-Encoding") == Some(&"chunked".to_string()) {
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
                    bdy_len += 2;
                    break;
                }
                while hdr_len + bdy_len + chunked_len + 2 > buf.len() {
                    buf.extend_by_streams(stream).await?;
                }
                res.body
                    .extend(&buf[(hdr_len + bdy_len)..(hdr_len + bdy_len + chunked_len)]);
                bdy_len += chunked_len + 2;
            }
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
                h.name.http_std_case(),
                str::from_utf8(h.value).unwrap_or("").to_string(),
            );
        }
        Ok(Some((req, n)))
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.http_std_case()).map(|a| a.as_str())
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
