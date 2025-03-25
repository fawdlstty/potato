pub mod client;
pub mod global_config;
pub mod server;
#[macro_use]
pub mod utils;

pub use client::*;
pub use global_config::*;
use http::uri::Scheme;
use http::Uri;
pub use inventory;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
pub use serde_json;
pub use server::*;

use anyhow::{anyhow, Error};
use chrono::Utc;
use core::str;
use rust_embed::Embed;
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;
use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin};
use strum::Display;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utils::bytes::CompressExt;
use utils::enums::{HttpConnection, HttpContentType};
use utils::number::HttpCodeExt;
use utils::refbuf::{RefBuf, ToRefBufExt};
use utils::refstr::{HeaderItem, HeaderRefOrString, RefOrString, ToRefStrExt};
use utils::string::StringExt;
use utils::tcp_stream::{TcpStreamExt, VecU8Ext};

static SERVER_STR: LazyLock<String> =
    LazyLock::new(|| format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));

type HttpHandler = fn(
    HttpRequest,
    SocketAddr,
    &mut WebsocketContext,
) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>;

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
    POST,
    PUT,
    DELETE,
    HEAD,
    OPTIONS,
    CONNECT,
    PATCH,
    TRACE,
}

#[derive(Eq, PartialEq)]
pub enum CompressMode {
    None,
    Gzip,
}

pub struct WebsocketContext {
    stream: Box<dyn TcpStreamExt>,
    upgrade_ws: bool,
}

impl WebsocketContext {
    pub fn is_upgraded_websocket(&self) -> bool {
        self.upgrade_ws
    }

    pub async fn upgrade_websocket(
        &mut self,
        req: &HttpRequest,
    ) -> anyhow::Result<WebsocketConnection> {
        if !req.is_websocket() {
            return Err(anyhow::Error::msg("is not a websocket request"));
        }
        let ws_key = req.get_header("Sec-WebSocket-Key").unwrap();
        // let ws_ext = req.get_header("Sec-WebSocket-Extensions").unwrap_or("".to_string());
        let res = HttpResponse::from_websocket(ws_key);
        self.stream
            .write_all(&res.as_bytes(CompressMode::None))
            .await?;
        self.upgrade_ws = true;
        Ok(WebsocketConnection {
            stream: &mut self.stream,
        })
    }
}

pub struct WebsocketConnection<'a> {
    stream: &'a mut Box<dyn TcpStreamExt>,
}

impl<'a> WebsocketConnection<'_> {
    pub async fn recv_frame_impl(&mut self) -> anyhow::Result<WsFrameImpl> {
        let buf = {
            let mut buf = [0u8; 2];
            self.stream.read_exact(&mut buf).await?;
            buf
        };
        //let fin = buf[0] & 0b1000_0000 != 0;
        let opcode = buf[0] & 0b0000_1111;
        let payload_len = {
            let payload_len = buf[1] & 0b0111_1111;
            match payload_len {
                126 => {
                    let mut buf = [0u8; 2];
                    self.stream.read_exact(&mut buf).await?;
                    u16::from_be_bytes(buf) as usize
                }
                127 => {
                    let mut buf = [0u8; 8];
                    self.stream.read_exact(&mut buf).await?;
                    u64::from_be_bytes(buf) as usize
                }
                _ => payload_len as usize,
            }
        };
        let omask_key = match buf[1] & 0b1000_0000 != 0 {
            true => {
                let mut mask_key = [0u8; 4];
                self.stream.read_exact(&mut mask_key).await?;
                Some(mask_key)
            }
            false => None,
        };
        let mut payload = vec![0u8; payload_len];
        self.stream.read_exact(&mut payload).await?;
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

    pub async fn recv_frame(&mut self) -> anyhow::Result<WsFrame> {
        let mut tmp = vec![];
        loop {
            let timeout = ServerConfig::get_ws_ping_duration().await;
            match tokio::time::timeout(timeout, self.recv_frame_impl()).await {
                Ok(ws_frame) => match ws_frame? {
                    WsFrameImpl::Close => return Err(anyhow::Error::msg("close frame")),
                    WsFrameImpl::Ping => self.write_frame_impl(WsFrameImpl::Pong).await?,
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
                Err(_) => self.write_frame_impl(WsFrameImpl::Ping).await?,
            }
        }
    }

    pub async fn write_frame_impl(&mut self, frame: WsFrameImpl) -> anyhow::Result<()> {
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
        self.stream.write_all(&buf).await?;
        self.stream.write_all(&payload).await?;
        Ok(())
    }

    pub async fn send_ping(&mut self) -> anyhow::Result<()> {
        self.write_frame_impl(WsFrameImpl::Ping).await
    }

    pub async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.write_frame_impl(WsFrameImpl::Binary(data)).await
    }

    pub async fn send_text(&mut self, data: &str) -> anyhow::Result<()> {
        self.write_frame_impl(WsFrameImpl::Text(data.as_bytes().to_vec()))
            .await
    }
}

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
    pub data: RefBuf,
}

unsafe impl Send for PostFile {}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url_path: RefOrString,
    pub url_query: HashMap<RefOrString, RefOrString>,
    pub version: u8,
    pub headers: HashMap<HeaderRefOrString, RefOrString>,
    pub body: RefBuf,
    pub body_pairs: HashMap<RefOrString, RefOrString>,
    pub body_files: HashMap<RefOrString, PostFile>,
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
            body: [].to_ref_buf(),
            body_pairs: HashMap::with_capacity(16),
            body_files: HashMap::with_capacity(4),
        }
    }

    pub fn from_url(url: &str, method: HttpMethod) -> anyhow::Result<(Self, bool, u16)> {
        let uri = url.parse::<Uri>()?;
        let mut req = Self::new();
        req.method = method;
        req.url_path = uri.path().to_ref_string();
        req.headers
            .insert("Host".into(), uri.host().unwrap_or("localhost").into());
        Ok((
            req,
            uri.scheme() == Some(&Scheme::HTTPS),
            uri.port_u16().unwrap_or(80),
        ))
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

    pub fn get_header_host(&self) -> &str {
        self.get_header_key(HeaderItem::Host).unwrap_or("")
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
        if self.get_header_connection() == HttpConnection::Upgrade {
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

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: &mut Box<dyn TcpStreamExt>,
    ) -> anyhow::Result<(Self, usize)> {
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
        req.body = buf[hdr_len..hdr_len + bdy_len].to_ref_buf();
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
                                        let data = content.as_bytes().to_ref_buf();
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
                4 if method == "HEAD" => HttpMethod::HEAD,
                4 if method == "POST" => HttpMethod::POST,
                5 if method == "PATCH" => HttpMethod::PATCH,
                5 if method == "TRACE" => HttpMethod::TRACE,
                6 if method == "DELETE" => HttpMethod::DELETE,
                7 if method == "OPTIONS" => HttpMethod::OPTIONS,
                7 if method == "CONNECT" => HttpMethod::CONNECT,
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

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: impl Into<String>) -> Self {
            let body = body.into();
            Self {
                version: "HTTP/1.1".into(),
                http_code: 200,
                headers: [
                    ("Date".into(), Utc::now().to_rfc2822()),
                    ("Server".into(), SERVER_STR.clone()),
                    ("Connection".into(), "keep-alive".into()),
                    ("Content-Type".into(), $cnt_type.into()),
                ]
                .into(),
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
                headers: [
                    ("Date".into(), Utc::now().to_rfc2822()),
                    ("Server".into(), SERVER_STR.clone()),
                    ("Connection".into(), "keep-alive".into()),
                    ("Content-Type".into(), $cnt_type.into()),
                ]
                .into(),
                body: body.to_vec(),
            }
        }
    };
}

impl HttpResponse {
    make_resp_by_text!(html, "text/html");
    make_resp_by_text!(css, "text/css");
    make_resp_by_text!(js, "text/javascript");
    make_resp_by_text!(text, "text/plain");
    make_resp_by_text!(json, "application/json");
    make_resp_by_text!(xml, "application/xml");
    make_resp_by_binary!(png, "image/png");

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

    pub fn from_file(path: &str, download: bool) -> Self {
        let mut buffer = vec![];
        if let Ok(mut file) = File::open(path) {
            _ = file.read_to_end(&mut buffer);
        }
        Self::from_mem_file(path, buffer, download)
    }

    pub fn from_mem_file(path: &str, data: Vec<u8>, download: bool) -> Self {
        let mut ret = Self::empty();
        let mime_type = match path.split('.').last() {
            Some("htm") => "text/html",
            Some("html") => "text/html",
            Some("js") => "application/javascript",
            Some("css") => "text/css",
            Some("json") => "application/json",
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
                ("Date".into(), Utc::now().to_rfc2822()),
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
        #[allow(unused_assignments)]
        let mut payload_tmp = vec![];
        let payload_ref = match cmode {
            CompressMode::Gzip if self.body.len() >= 32 => match self.body.compress() {
                Ok(data) => {
                    payload_tmp = data;
                    &payload_tmp
                }
                Err(_) => {
                    cmode = CompressMode::None;
                    &self.body
                }
            },
            CompressMode::Gzip => {
                cmode = CompressMode::None;
                &self.body
            }
            _ => &self.body,
        };
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
        stream: &mut Box<dyn TcpStreamExt>,
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
                println!("chunked_len: {chunked_len}");
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
        self.headers.get(key).map(|a| a.as_str())
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
