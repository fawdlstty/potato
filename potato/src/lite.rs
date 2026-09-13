#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(feature = "websocket")]
pub use websocket::*;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::sync::atomic::{AtomicPtr, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use linkme::distributed_slice;

// ---------------------------------------------------------------------------
// HttpMethod
// ---------------------------------------------------------------------------

/// HTTP 请求方法
// 变体名沿用 HTTP 协议惯例全大写（与 http::Method 保持一致），不遵循 Rust 首字母大写缩写规范
#[allow(clippy::upper_case_acronyms)]
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

impl core::str::FromStr for HttpMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GET" => Ok(HttpMethod::GET),
            "POST" => Ok(HttpMethod::POST),
            "PUT" => Ok(HttpMethod::PUT),
            "DELETE" => Ok(HttpMethod::DELETE),
            "HEAD" => Ok(HttpMethod::HEAD),
            "OPTIONS" => Ok(HttpMethod::OPTIONS),
            "PATCH" => Ok(HttpMethod::PATCH),
            "CONNECT" => Ok(HttpMethod::CONNECT),
            "TRACE" => Ok(HttpMethod::TRACE),
            _ => Err(()),
        }
    }
}

impl HttpMethod {
    /// 从字符串解析 HTTP 方法
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        <Self as core::str::FromStr>::from_str(s).ok()
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
// Minimal executor
// ---------------------------------------------------------------------------

static WAIT_HOOK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Installs the platform's low-power wait callback. Passing `None` restores
/// the readiness/spin fallback.
pub fn set_wait_hook(hook: Option<fn()>) {
    let hook = hook.map_or(core::ptr::null_mut(), |hook| hook as usize as *mut ());
    WAIT_HOOK.store(hook, Ordering::Release);
}

fn wait_for_wake() {
    let hook = WAIT_HOOK.load(Ordering::Acquire);
    if hook.is_null() {
        core::hint::spin_loop();
    } else {
        // The pointer was installed from a valid `fn()` by `set_wait_hook`.
        let hook: fn() = unsafe { core::mem::transmute(hook) };
        hook();
    }
}

use core::sync::atomic::AtomicBool;

struct ExecutorWaker {
    ready: AtomicBool,
}

impl ExecutorWaker {
    fn wake(&self) {
        self.ready.store(true, Ordering::Release);
    }
}

fn waker_vtable() -> &'static RawWakerVTable {
    &RawWakerVTable::new(clone_waker, wake_waker, wake_waker_by_ref, drop_waker)
}

unsafe fn clone_waker(data: *const ()) -> RawWaker {
    let waker =
        core::mem::ManuallyDrop::new(unsafe { Arc::from_raw(data.cast::<ExecutorWaker>()) });
    RawWaker::new(Arc::into_raw(Arc::clone(&waker)).cast(), waker_vtable())
}

unsafe fn wake_waker(data: *const ()) {
    let waker = unsafe { Arc::from_raw(data.cast::<ExecutorWaker>()) };
    waker.wake();
}

unsafe fn wake_waker_by_ref(data: *const ()) {
    let waker =
        core::mem::ManuallyDrop::new(unsafe { Arc::from_raw(data.cast::<ExecutorWaker>()) });
    waker.wake();
}

unsafe fn drop_waker(data: *const ()) {
    drop(unsafe { Arc::from_raw(data.cast::<ExecutorWaker>()) });
}

fn waker(state: Arc<ExecutorWaker>) -> Waker {
    // SAFETY: `data` remains a valid `Arc<ExecutorWaker>` pointer, and all
    // vtable functions maintain its reference count.
    unsafe { Waker::from_raw(RawWaker::new(Arc::into_raw(state).cast(), waker_vtable())) }
}

/// Polls a future to completion on the current thread.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let state = Arc::new(ExecutorWaker {
        ready: core::sync::atomic::AtomicBool::new(false),
    });
    let waker = waker(Arc::clone(&state));
    let mut cx = Context::from_waker(&waker);

    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(output) = Future::poll(future.as_mut(), &mut cx) {
            return output;
        }

        while !state.ready.swap(false, Ordering::Acquire) {
            wait_for_wake();
        }
    }
}

// ---------------------------------------------------------------------------
// Handler registry (distributed slice via linkme, compile-time registration)
// ---------------------------------------------------------------------------

/// A registered route handler entry
pub struct RouteHandler {
    pub method: &'static str,
    pub path: &'static str,
    pub handler: fn(&mut HttpRequest) -> Option<HttpResponse>,
}

/// Distributed slice collecting all route handlers registered via `#[http_*]` macros.
/// Elements are gathered by the linker at compile time — no runtime registration needed.
#[distributed_slice]
pub static ROUTE_HANDLERS: [RouteHandler];
