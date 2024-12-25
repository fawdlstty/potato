pub mod client;
pub mod server;
pub mod utils;

pub use client::*;
pub use inventory;
pub use potato_macro::*;

use async_recursion::async_recursion;
use chrono::Utc;
use sha1::{Digest, Sha1};
use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin};
use strum::Display;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use utils::bytes::VecU8Ext;
use utils::string::StringExt;

type HttpHandler = fn(
    HttpRequest,
    SocketAddr,
    &mut WebsocketContext,
) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>;

pub struct RequestHandlerFlag {
    pub method: HttpMethod,
    pub path: &'static str,
    pub handler: HttpHandler,
}

impl RequestHandlerFlag {
    pub const fn new(method: HttpMethod, path: &'static str, handler: HttpHandler) -> Self {
        RequestHandlerFlag {
            method,
            path,
            handler,
        }
    }
}

inventory::collect!(RequestHandlerFlag);

#[derive(Clone, Copy, Display, Eq, Hash, PartialEq)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    OPTIONS,
    HEAD,
}

#[derive(Eq, PartialEq)]
pub enum CompressMode {
    None,
    Gzip,
}

pub struct WebsocketContext {
    stream: TcpStream,
    upgrade_ws: bool,
}

impl WebsocketContext {
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
    stream: &'a mut TcpStream,
}

impl WebsocketConnection<'_> {
    #[async_recursion]
    pub async fn read_frame(&mut self) -> anyhow::Result<WsFrame> {
        let buf = {
            let mut buf = [0u8; 2];
            self.stream.read_exact(&mut buf).await?;
            buf
        };
        let fin = buf[0] & 0b1000_0000 != 0;
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
        if !fin || opcode == 0x0 {
            let next_frame = self.read_frame().await?;
            Ok(match next_frame {
                WsFrame::Text(text) => {
                    WsFrame::Text(format!("{}{}", String::from_utf8(payload)?, text))
                }
                WsFrame::Binary(bin) => WsFrame::Binary([payload, bin].concat()),
                _ => next_frame,
            })
        } else {
            match opcode {
                0x1 => {
                    let payload = String::from_utf8(payload).unwrap_or("".to_string());
                    Ok(WsFrame::Text(payload))
                }
                0x2 => Ok(WsFrame::Binary(payload)),
                0x8 => Ok(WsFrame::Close),
                0x9 => Ok(WsFrame::Ping),
                0xA => Ok(WsFrame::Pong),
                _ => Err(anyhow::Error::msg("unsupported opcode")),
            }
        }
    }

    pub async fn write_frame(&mut self, frame: WsFrame) -> anyhow::Result<()> {
        let (fin, opcode, payload) = match frame {
            WsFrame::Close => (true, 0x8, vec![]),
            WsFrame::Ping => (true, 0x9, vec![]),
            WsFrame::Pong => (true, 0xA, vec![]),
            WsFrame::Binary(bin) => (true, 0x2, bin),
            WsFrame::Text(text) => (true, 0x1, text.as_bytes().to_vec()),
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
}

pub enum WsFrame {
    Close,
    Ping,
    Pong,
    Binary(Vec<u8>),
    Text(String),
}

pub struct PostFile {
    pub filename: String,
    pub data: Vec<u8>,
}

pub struct HttpRequest {
    pub method: HttpMethod,
    pub url_path: String,
    pub url_query: HashMap<String, String>,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub body_pairs: HashMap<String, String>,
    pub body_files: HashMap<String, PostFile>,
}
unsafe impl Send for HttpRequest {}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            url_path: "/".to_string(),
            url_query: HashMap::new(),
            version: "HTTP/1.1".to_string(),
            headers: HashMap::new(),
            body: vec![],
            body_pairs: HashMap::new(),
            body_files: HashMap::new(),
        }
    }

    pub fn set_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers
            .insert(key.into().http_standardization(), value.into());
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers
            .get(&key.http_standardization())
            .map(|a| &a[..])
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
}

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

impl HttpResponse {
    make_resp_by_text!(html, "text/html");
    make_resp_by_text!(text, "text/plain");
    make_resp_by_text!(json, "application/json");
    make_resp_by_text!(xml, "application/xml");

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
        let status_str = http::StatusCode::from_u16(self.http_code)
            .map(|c| c.canonical_reason())
            .ok()
            .flatten()
            .unwrap_or("UNKNOWN");
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
