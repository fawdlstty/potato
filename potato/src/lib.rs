pub mod client;
pub mod server;
pub mod utils;

use anyhow::Error;
pub use client::*;
pub use inventory;
use lazy_static::lazy_static;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
use rust_embed::Embed;
pub use serde_json;
pub use server::*;

use chrono::Utc;
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin};
use strum::Display;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utils::bytes::VecU8Ext;
use utils::number::HttpCodeExt;
use utils::refstr::{RefStr, ToRefStrExt};
use utils::string::{StrExt, StringExt};
use utils::tcp_stream::TcpStreamExt;

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
            match self.recv_frame_impl().await? {
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
    pub filename: String,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url_path: RefStr,
    pub url_query: HashMap<RefStr, RefStr>,
    pub version: u8,
    pub headers: HashMap<RefStr, RefStr>,
    pub body: Vec<u8>,
    pub body_pairs: HashMap<RefStr, RefStr>,
    pub body_files: HashMap<RefStr, PostFile>,
}
unsafe impl Send for HttpRequest {}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            url_path: "/".to_ref_str(),
            url_query: HashMap::new(),
            version: 11,
            headers: HashMap::with_capacity(16),
            body: vec![],
            body_pairs: HashMap::with_capacity(16),
            body_files: HashMap::with_capacity(4),
        }
    }

    pub fn set_header(&mut self, key: RefStr, value: RefStr) {
        self.headers.insert(key, value);
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers.get(&key.to_ref_str()).map(|a| a.to_str())
    }

    pub fn get_header_accept_encoding(&self) -> CompressMode {
        if let Some(encodings) = self.get_header("Accept-Encoding") {
            for encoding in encodings.split(',') {
                let encoding = encoding.trim();
                if encoding == "gzip" {
                    return CompressMode::Gzip;
                }
            }
        }
        CompressMode::None
    }

    pub fn is_websocket(&self) -> bool {
        if self.method != HttpMethod::GET {
            return false;
        }
        if self
            .get_header("Connection")
            .map_or(false, |val| val.to_lowercase() != "upgrade")
        {
            return false;
        }
        if self
            .get_header("Upgrade")
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

    pub fn from_buf(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
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
                req.url_path = url[..p].to_ref_str();
                req.url_query = url[p + 1..]
                    .split('&')
                    .into_iter()
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| (a.to_ref_str(), b.to_ref_str()))
                    .collect();
            }
            None => req.url_path = url.to_ref_str(),
        }
        req.version = rreq.version.unwrap_or(1) + 10;
        for h in rreq.headers.iter() {
            if h.name == "" {
                break;
            }
            req.headers
                .insert(h.name.to_ref_str(), h.value.to_ref_str());
        }
        // // // pub body: Vec<u8>,
        // // // pub body_pairs: HashMap<RefStr, RefStr>,
        // // // pub body_files: HashMap<RefStr, PostFile>,
        // loop {
        //     let line = stream.read_line().await;
        //     if let Some((key, value)) = line.split_once(':') {
        //         req.set_header(key.trim(), value.trim());
        //     } else {
        //         break;
        //     }
        // }
        // if let Some(cnt_len) = req.get_header("Content-Length") {
        //     if let Ok(cnt_len) = cnt_len.parse::<usize>() {
        //         if cnt_len > 0 {
        //             let mut body = vec![0u8; cnt_len];
        //             stream.read_exact(&mut body).await?;
        //             req.body = body;
        //         }
        //     }
        // }
        // if let Some(cnt_type) = req.get_header("Content-Type") {
        //     if cnt_type == "application/json" {
        //         if let Ok(body_str) = std::str::from_utf8(&req.body) {
        //             if let Ok(root) = serde_json::from_str::<serde_json::Value>(&body_str) {
        //                 if let serde_json::Value::Object(obj) = root {
        //                     for (k, v) in obj {
        //                         req.body_pairs.insert(k, v.to_string());
        //                     }
        //                 }
        //             }
        //         }
        //     } else if cnt_type == "application/x-www-form-urlencoded" {
        //         if let Ok(body_str) = std::str::from_utf8(&req.body) {
        //             body_str.split('&').for_each(|s| {
        //                 if let Some((a, b)) = s.split_once('=') {
        //                     req.body_pairs.insert(a.url_decode(), b.url_decode());
        //                 }
        //             });
        //         }
        //     } else if cnt_type.starts_with("multipart/form-data") {
        //         let boundary = cnt_type.split_once("boundary=").unwrap_or(("", "")).1;
        //         if let Ok(body_str) = std::str::from_utf8(&req.body) {
        //             for mut s in body_str.split(format!("--{boundary}").as_str()) {
        //                 if s == "--" {
        //                     break;
        //                 }
        //                 if s.ends_with("\r\n") {
        //                     s = &s[..s.len() - 2];
        //                 }
        //                 if let Some((key_str, content)) = s.split_once("\r\n\r\n") {
        //                     let keys: Vec<&str> = key_str
        //                         .split("\r\n")
        //                         .map(|p| p.split(";").collect::<Vec<_>>())
        //                         .collect::<Vec<_>>()
        //                         .into_iter()
        //                         .flatten()
        //                         .collect();
        //                     let mut name = None;
        //                     let mut filename = None;
        //                     for key in keys.into_iter() {
        //                         if let Some((k, v)) = key.trim().split_once('=') {
        //                             if k == "name" {
        //                                 name = Some((&v[1..v.len() - 1]).to_string());
        //                             } else if k == "filename" {
        //                                 filename = Some((&v[1..v.len() - 1]).to_string());
        //                             }
        //                         }
        //                     }
        //                     if let Some(name) = name {
        //                         if let Some(filename) = filename {
        //                             req.body_files.insert(
        //                                 name,
        //                                 PostFile {
        //                                     filename,
        //                                     data: content.as_bytes().to_vec(),
        //                                 },
        //                             );
        //                         } else {
        //                             req.body_pairs.insert(name, content.to_string());
        //                         }
        //                     }
        //                 }
        //             }
        //         }
        //     }
        // }
        Ok(Some((req, n)))
    }

    // pub async fn process_request(self) -> HttpResponse {

    // }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub version: String,
    pub http_code: u16,
    pub headers: HashMap<String, String>,
    pub payload: Vec<u8>,
}
unsafe impl Send for HttpResponse {}

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(payload: impl Into<String>) -> Self {
            let payload = payload.into();
            Self {
                version: "HTTP/1.1".to_string(),
                http_code: 200,
                headers: [
                    ("Date".to_string(), Utc::now().to_rfc2822()),
                    ("Server".to_string(), "Potato 0.1.0".to_string()),
                    ("Content-Type".to_string(), $cnt_type.to_string()),
                ]
                .into(),
                payload: payload.as_bytes().to_vec(),
            }
        }
    };
}

macro_rules! make_resp_by_binary {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(payload: &[u8]) -> Self {
            Self {
                version: "HTTP/1.1".to_string(),
                http_code: 200,
                headers: [
                    ("Date".to_string(), Utc::now().to_rfc2822()),
                    ("Server".to_string(), "Potato 0.1.0".to_string()),
                    ("Content-Type".to_string(), $cnt_type.to_string()),
                ]
                .into(),
                payload: payload.to_vec(),
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

    pub fn from_file(path: &str) -> Self {
        let mut buffer = vec![];
        if let Ok(mut file) = File::open(path) {
            _ = file.read_to_end(&mut buffer);
        }
        Self::from_mem_file(path, buffer)
    }

    pub fn from_mem_file(path: &str, data: Vec<u8>) -> Self {
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
        ret.payload = data;
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
            version: "HTTP/1.1".to_string(),
            http_code: 101,
            headers: [
                ("Date".to_string(), Utc::now().to_rfc2822()),
                ("Server".to_string(), "Potato 0.1.0".to_string()),
                ("Connection".to_string(), "Upgrade".to_string()),
                ("Upgrade".to_string(), "websocket".to_string()),
                ("Sec-WebSocket-Accept".to_string(), ws_accept),
            ]
            .into(),
            payload: vec![],
        }
    }

    pub fn as_bytes(&self, mut cmode: CompressMode) -> Vec<u8> {
        #[allow(unused_assignments)]
        let mut payload_tmp = vec![];
        let payload_ref = match cmode {
            CompressMode::None => &self.payload,
            CompressMode::Gzip => match self.payload.compress() {
                Ok(data) => {
                    payload_tmp = data;
                    &payload_tmp
                }
                Err(_) => {
                    cmode = CompressMode::None;
                    &self.payload
                }
            },
        };
        //
        let mut ret = "".to_string();
        let status_str = self.http_code.http_code_to_desp();
        ret.push_str(&format!(
            "{} {} {}\r\n",
            self.version, self.http_code, status_str
        ));
        for (key, value) in self.headers.iter() {
            ret.push_str(&format!("{}: {}\r\n", key, value));
        }
        if self.http_code != 101 {
            ret.push_str(&format!("Content-Length: {}\r\n", payload_ref.len()));
            if cmode == CompressMode::Gzip {
                ret.push_str("Content-Encoding: gzip\r\n");
            }
        }
        ret.push_str("\r\n");
        let mut ret: Vec<u8> = ret.as_bytes().to_vec();
        ret.extend(payload_ref);
        ret
    }
}

lazy_static! {
    pub static ref DOC_RES: HashMap<&'static str, &'static str> = {
        [
            ("index.html", include_str!("../swagger_res/index.html")),
            ("index.css", include_str!("../swagger_res/index.css")),
            (
                "swagger-ui.css",
                include_str!("../swagger_res/swagger-ui.css"),
            ),
            (
                "swagger-ui-bundle.js",
                include_str!("../swagger_res/swagger-ui-bundle.js"),
            ),
            (
                "swagger-ui-standalone-preset.js",
                include_str!("../swagger_res/swagger-ui-standalone-preset.js"),
            ),
            (
                "swagger-initializer.js",
                r#"window.onload = function() {
   window.ui = SwaggerUIBundle({
       url: "./index.json",
       dom_id: '#swagger-ui',
       deepLinking: true,
       presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
       plugins: [SwaggerUIBundle.plugins.DownloadUrl],
       layout: "StandaloneLayout"
   });
 };"#,
            ),
        ]
        .into_iter()
        .collect()
    };
}

pub struct DocResource {}

impl DocResource {
    pub fn load_str(file: &str) -> &'static str {
        DOC_RES.get(file).map(|v| &**v).unwrap_or("")
    }
}

pub fn load_embed<T: Embed>() -> HashMap<String, Cow<'static, [u8]>> {
    let mut ret = HashMap::new();
    for name in T::iter().into_iter() {
        if let Some(file) = T::get(&name) {
            if name.ends_with("index.htm") || name.ends_with("index.html") {
                if let Some(path) = Path::new(&name[..]).parent() {
                    if let Some(path) = path.to_str() {
                        let path = match path.ends_with('/') {
                            true => path.to_string(),
                            false => format!("{path}/"),
                        };
                        ret.insert(path, file.data.clone());
                    }
                }
            }
            ret.insert(name.to_string(), file.data);
        }
    }
    ret
}
