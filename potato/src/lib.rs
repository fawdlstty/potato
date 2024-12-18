pub mod server;
pub mod utils;

pub use inventory;
pub use potato_macro::*;

use chrono::Utc;
use http::Uri;
use std::{collections::HashMap, future::Future, net::SocketAddr, pin::Pin};
use strum::Display;

type HttpHandler =
    fn(RequestContext) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + 'static>>;

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

pub struct RequestContext {
    pub addr: SocketAddr,
    pub req: HttpRequest,
}
unsafe impl Send for RequestContext {}

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
}

pub struct HttpResponse {
    pub version: String,
    pub http_code: u16,
    pub headers: HashMap<String, String>,
    pub payload: String,
}
unsafe impl Send for HttpResponse {}
impl HttpResponse {
    pub fn add_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers.insert(key.into(), value.into());
    }
}

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(payload: impl Into<String>) -> Self {
            let payload = payload.into();
            Self {
                version: "HTTP/1.1".to_string(),
                http_code: 200,
                headers: [
                    ("Server".to_string(), "Potato 0.1.0".to_string()),
                    ("Content-Type".to_string(), $cnt_type.to_string()),
                    ("Content-Length".to_string(), format!("{}", payload.len())),
                ]
                .into(),
                payload,
            }
        }
    };
}

impl HttpResponse {
    make_resp_by_text!(html, "text/html");
    make_resp_by_text!(text, "text/plain");
    make_resp_by_text!(json, "application/json");
    make_resp_by_text!(xml, "application/xml");

    pub fn not_found() -> Self {
        let mut ret = Self::html("404 not found");
        ret.http_code = 404;
        ret
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut ret = "".to_string();
        ret.push_str(&format!("{} {} {}\r\n", self.version, self.http_code, "OK"));
        ret.push_str(&format!("Date: {}\r\n", Utc::now().to_rfc2822()));
        ret.push_str(&format!("Content-Length: {}\r\n", self.payload.len()));
        for (key, value) in self.headers.iter() {
            ret.push_str(&format!("{}: {}\r\n", key, value));
        }
        ret.push_str("\r\n");
        ret.push_str(&self.payload);
        ret.as_bytes().to_vec()
    }
}
