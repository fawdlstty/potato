pub mod server;
pub mod utils;

pub use inventory;
pub use potato_macro::*;

use chrono::Utc;
use http::Uri;
use sha1::{Digest, Sha1};
use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin};
use strum::Display;
use tokio::io::AsyncWriteExt;
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
    pub async fn upgrade_websocket(&mut self, req: &HttpRequest) -> anyhow::Result<()> {
        if !req.is_websocket() {
            return Err(anyhow::Error::msg("is not a websocket request"));
        }
        self.upgrade_ws = true;
        let ws_key = req
            .get_header("Sec-WebSocket-Key")
            .unwrap_or("".to_string());
        let res = HttpResponse::from_websocket(&ws_key);
        self.stream
            .write_all(&res.as_bytes(CompressMode::None))
            .await?;
        Ok(())
    }
}

pub struct HttpRequest {
    pub method: HttpMethod,
    pub uri: http::Uri,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub payload: Vec<u8>,
}
unsafe impl Send for HttpRequest {}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            uri: Uri::default(),
            version: "HTTP/1.1".to_string(),
            headers: HashMap::new(),
            payload: vec![],
        }
    }

    pub fn get_header(&self, key: &str) -> Option<String> {
        self.headers.get(&key.http_standardization()).cloned()
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
            .map_or(false, |val| val.len() > 0)
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

    pub fn empty() -> Self {
        Self::html("")
    }

    pub fn from_websocket(ws_key: &str) -> Self {
        let ws_accept = {
            let ws_key = format!("{ws_key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let mut hasher = Sha1::new();
            hasher.update(ws_key.as_bytes());
            let result = hasher.finalize();
            //
            #[allow(deprecated)]
            base64::encode(result)
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
        if self.http_code != 101 {
            ret.push_str(&format!("Content-Length: {}\r\n", payload_ref.len()));
            if cmode == CompressMode::Gzip {
                ret.push_str("Content-Encoding: gzip\r\n");
            }
        }
        for (key, value) in self.headers.iter() {
            ret.push_str(&format!("{}: {}\r\n", key, value));
        }
        ret.push_str("\r\n");
        let mut ret: Vec<u8> = ret.as_bytes().to_vec();
        ret.extend(payload_ref);
        ret
    }
}
