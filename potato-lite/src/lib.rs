#![no_std]

extern crate alloc;

pub mod client;
pub mod server;

#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(feature = "websocket")]
pub use websocket::*;

pub use crate::server::*;
pub use potato_macro_lite::*;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

// ---------------------------------------------------------------------------
// HttpMethod
// ---------------------------------------------------------------------------

/// HTTP 请求方法
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    HEAD,
    OPTIONS,
    PATCH,
    CONNECT,
    TRACE,
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::CONNECT => "CONNECT",
            HttpMethod::TRACE => "TRACE",
        })
    }
}

impl HttpMethod {
    /// 从字符串解析 HTTP 方法
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "GET" => Some(HttpMethod::GET),
            "POST" => Some(HttpMethod::POST),
            "PUT" => Some(HttpMethod::PUT),
            "DELETE" => Some(HttpMethod::DELETE),
            "HEAD" => Some(HttpMethod::HEAD),
            "OPTIONS" => Some(HttpMethod::OPTIONS),
            "PATCH" => Some(HttpMethod::PATCH),
            "CONNECT" => Some(HttpMethod::CONNECT),
            "TRACE" => Some(HttpMethod::TRACE),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// HttpRequest
// ---------------------------------------------------------------------------

/// HTTP 请求
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url_path: String,
    pub url_query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub version: u8,
}

impl Default for HttpRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::GET,
            url_path: String::from("/"),
            url_query: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
            version: 11,
        }
    }

    /// 获取请求头（大小写不敏感）
    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// 判断请求是否为 WebSocket 升级请求（需要启用 `websocket` feature）
    ///
    /// 检测条件：
    /// - 方法为 GET
    /// - Connection 头包含 "upgrade"（大小写不敏感）
    /// - Upgrade 头为 "websocket"（大小写不敏感）
    /// - Sec-WebSocket-Version 头存在
    /// - Sec-WebSocket-Key 头存在且非空
    #[cfg(feature = "websocket")]
    pub fn is_websocket(&self) -> bool {
        if self.method != HttpMethod::GET {
            return false;
        }
        let has_upgrade_connection = self
            .get_header("Connection")
            .map(|v| v.to_ascii_lowercase().contains("upgrade"))
            .unwrap_or(false);
        if !has_upgrade_connection {
            return false;
        }
        let is_ws_upgrade = self
            .get_header("Upgrade")
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);
        if !is_ws_upgrade {
            return false;
        }
        let has_version = self
            .get_header("Sec-WebSocket-Version")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if !has_version {
            return false;
        }
        let has_key = self
            .get_header("Sec-WebSocket-Key")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        has_key
    }
}

// ---------------------------------------------------------------------------
// HttpResponse
// ---------------------------------------------------------------------------

/// HTTP 响应
pub struct HttpResponse {
    pub http_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Default for HttpResponse {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpResponse {
    pub fn new() -> Self {
        Self {
            http_code: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 创建 text/plain 响应
    pub fn text(body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            http_code: 200,
            headers: alloc::vec![(
                String::from("Content-Type"),
                String::from("text/plain; charset=utf-8"),
            )],
            body: body.into_bytes(),
        }
    }

    /// 创建 application/json 响应
    pub fn json(body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            http_code: 200,
            headers: alloc::vec![(
                String::from("Content-Type"),
                String::from("application/json"),
            )],
            body: body.into_bytes(),
        }
    }

    /// 创建 text/html 响应
    pub fn html(body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            http_code: 200,
            headers: alloc::vec![(
                String::from("Content-Type"),
                String::from("text/html; charset=utf-8"),
            )],
            body: body.into_bytes(),
        }
    }

    /// 创建 404 Not Found 响应
    pub fn not_found() -> Self {
        Self {
            http_code: 404,
            headers: alloc::vec![(String::from("Content-Type"), String::from("text/plain"),)],
            body: b"404 Not Found".to_vec(),
        }
    }

    /// 添加响应头
    pub fn add_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.headers.push((key.into(), value.into()));
    }
}

// ---------------------------------------------------------------------------
// Headers (used by client macros)
// ---------------------------------------------------------------------------

/// HTTP 头部键值对
pub struct Headers {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// HTTP date formatting (no chrono dependency)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
#[allow(dead_code)]
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
#[allow(dead_code)]
const MONTH_DAYS: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// 将 Unix 时间戳格式化为 RFC 7231 HTTP 日期字符串
#[allow(dead_code)]
pub(crate) fn format_http_date(timestamp: u64) -> String {
    let secs = timestamp % 60;
    let mins = (timestamp / 60) % 60;
    let hours = (timestamp / 3600) % 24;
    let dow = (timestamp / 86400) % 7;

    let mut days = timestamp / 86400;
    let mut year: u64 = 1970;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }

    let mut month: usize = 0;
    for i in 0..12u64 {
        let dim = MONTH_DAYS[i as usize] + if i == 1 && is_leap(year) { 1 } else { 0 };
        if days < dim {
            month = i as usize;
            break;
        }
        days -= dim;
    }
    let day = days + 1;

    alloc::format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[dow as usize],
        day,
        MONTHS[month],
        year,
        hours,
        mins,
        secs,
    )
}

#[allow(dead_code)]
fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ---------------------------------------------------------------------------
// get! macro
// ---------------------------------------------------------------------------

/// 发起 HTTP GET 请求的便捷宏。
///
/// # 示例
/// ```ignore
/// let res = potato_lite::get!(stack, "http://192.168.1.1/api").await;
/// ```
#[macro_export]
macro_rules! get {
    ($stack:expr, $url:expr $(,)?) => {
        $crate::client::get($stack, $url)
    };
}

// ---------------------------------------------------------------------------
// websocket! macro
// ---------------------------------------------------------------------------

/// 建立 WebSocket 连接的便捷宏（需要启用 `websocket` feature）。
///
/// # 示例
/// ```ignore
/// let mut rx = [0u8; 4096];
/// let mut tx = [0u8; 4096];
/// let mut ws = potato_lite::websocket!(stack, "ws://192.168.1.1/ws", &mut rx, &mut tx).await?;
/// ws.send_text("hello").await?;
/// ```
#[cfg(feature = "websocket")]
#[macro_export]
macro_rules! websocket {
    ($stack:expr, $url:expr, $rx_buf:expr, $tx_buf:expr $(,)?) => {
        $crate::websocket::Websocket::connect($stack, $url, $rx_buf, $tx_buf)
    };
}
