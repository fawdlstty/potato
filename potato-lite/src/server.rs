use crate::{HttpMethod, HttpRequest, HttpResponse};
use alloc::string::String;
use alloc::vec::Vec;
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;

#[cfg(feature = "websocket")]
use alloc::boxed::Box;
#[cfg(feature = "websocket")]
use core::future::Future;
#[cfg(feature = "websocket")]
use core::pin::Pin;

// ---------------------------------------------------------------------------
// Route storage (global, for embedded single-threaded context)
// ---------------------------------------------------------------------------

struct Route {
    path: &'static str,
    handler: fn(&mut HttpRequest) -> Option<HttpResponse>,
}

static mut ROUTES: Option<Vec<Route>> = None;

// ---------------------------------------------------------------------------
// WebSocket route storage
// ---------------------------------------------------------------------------

#[cfg(feature = "websocket")]
type WsHandler = for<'a> fn(&'a mut TcpSocket<'a>) -> Pin<Box<dyn Future<Output = ()> + 'a>>;

#[cfg(feature = "websocket")]
struct WsRoute {
    path: &'static str,
    handler: WsHandler,
}

#[cfg(feature = "websocket")]
static mut WS_ROUTES: Option<Vec<WsRoute>> = None;

// ---------------------------------------------------------------------------
// PipeContext
// ---------------------------------------------------------------------------

/// 路由配置上下文，用法与 potato::PipeContext 一致
pub struct PipeContext;

impl PipeContext {
    /// 注册同步路由处理器
    ///
    /// # 示例
    /// ```ignore
    /// ctx.use_custom_sync("/hello", |_req| {
    ///     Some(potato_lite::HttpResponse::text("Hello!"))
    /// });
    /// ```
    pub fn use_custom_sync(
        &mut self,
        path: &'static str,
        handler: fn(&mut HttpRequest) -> Option<HttpResponse>,
    ) {
        unsafe {
            let routes_ptr = core::ptr::addr_of_mut!(ROUTES);
            if (*routes_ptr).is_none() {
                *routes_ptr = Some(Vec::new());
            }
            (*routes_ptr)
                .as_mut()
                .unwrap()
                .push(Route { path, handler });
        }
    }

    /// 注册 WebSocket 路由处理器（需要启用 `websocket` feature）
    ///
    /// # 示例
    /// ```ignore
    /// ctx.use_websocket("/ws", |socket| {
    ///     Box::pin(async move {
    ///         potato_lite::websocket::ws_send_ping(socket).await.unwrap();
    ///         loop {
    ///             match potato_lite::websocket::ws_recv(socket).await {
    ///                 Ok(potato_lite::WsFrame::Text(t)) => {
    ///                     let _ = potato_lite::websocket::ws_send_text(socket, &t).await;
    ///                 }
    ///                 _ => break,
    ///             }
    ///         }
    ///     })
    /// });
    /// ```
    #[cfg(feature = "websocket")]
    pub fn use_websocket(&mut self, path: &'static str, handler: WsHandler) {
        unsafe {
            let routes_ptr = core::ptr::addr_of_mut!(WS_ROUTES);
            if (*routes_ptr).is_none() {
                *routes_ptr = Some(Vec::new());
            }
            (*routes_ptr)
                .as_mut()
                .unwrap()
                .push(WsRoute { path, handler });
        }
    }
}

// ---------------------------------------------------------------------------
// HttpServer
// ---------------------------------------------------------------------------

/// 嵌入式 HTTP/1.1 服务器，用法与 potato::HttpServer 一致
pub struct HttpServer {
    port: u16,
}

impl HttpServer {
    /// 创建新的 HTTP 服务器实例
    ///
    /// # 参数
    /// * `port` - 监听端口号
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    /// 配置路由
    ///
    /// # 示例
    /// ```ignore
    /// server.configure(|ctx| {
    ///     ctx.use_custom_sync("/api", |req| {
    ///         Some(potato_lite::HttpResponse::json(r#"{"ok":true}"#))
    ///     });
    /// });
    /// ```
    pub fn configure<F: FnOnce(&mut PipeContext)>(&mut self, f: F) {
        unsafe {
            let routes_ptr = core::ptr::addr_of_mut!(ROUTES);
            *routes_ptr = Some(Vec::new());
        }
        #[cfg(feature = "websocket")]
        unsafe {
            let ws_routes_ptr = core::ptr::addr_of_mut!(WS_ROUTES);
            *ws_routes_ptr = Some(Vec::new());
        }
        let mut ctx = PipeContext;
        f(&mut ctx);
    }

    /// 启动 HTTP 服务器，接受连接并顺序处理请求
    ///
    /// 由于嵌入式环境资源有限，采用单任务顺序处理模式。
    /// `Stack` 是 `Copy` 的，按值传入即可。
    pub async fn serve(&mut self, stack: Stack<'_>) {
        let port = self.port;

        let mut rx_buf = [0u8; 1024];
        let mut tx_buf = [0u8; 1024];
        let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);

        if socket.accept(port).await.is_err() {
            return;
        }

        loop {
            // 处理当前连接（顺序处理，单任务）
            let _ = Self::handle_connection(&mut socket).await;
            // 连接关闭后 socket 回到 Closed 状态，重新 accept
            socket.abort();
            if socket.accept(port).await.is_err() {
                break;
            }
        }
    }

    /// 处理单个 HTTP 连接
    async fn handle_connection(socket: &mut TcpSocket<'_>) -> Result<(), ()> {
        let mut buf = [0u8; 4096];
        let mut pos: usize = 0;

        // 1. 读取直到找到头部结束标记 \r\n\r\n
        let _body_end = loop {
            if let Some(p) = find_header_end(&buf[..pos]) {
                break p;
            }
            if pos >= buf.len() {
                return Err(()); // 头部过大
            }
            let n = socket.read(&mut buf[pos..]).await.map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            pos += n;
        };

        // 2. 用 httparse 解析请求头部
        //    在独立作用域中解析，确保 raw_req（持有 buf 的借用）在进入 body 读取前释放
        let (hdr_len, content_length, method, version, url_path, url_query, req_headers) = {
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut raw_req = httparse::Request::new(&mut headers);
            match raw_req.parse(&buf[..pos]) {
                Ok(httparse::Status::Complete(hdr_len)) => {
                    let cl = raw_req
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case("Content-Length"))
                        .and_then(|h| core::str::from_utf8(h.value).ok())
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);

                    let method = HttpMethod::from_str(raw_req.method.unwrap_or("GET"))
                        .unwrap_or(HttpMethod::GET);
                    let version = raw_req.version.unwrap_or(1);

                    let path_str = raw_req.path.unwrap_or("/");
                    let (url_path, url_query) = parse_path_query(path_str);

                    let req_headers: Vec<(String, String)> = raw_req
                        .headers
                        .iter()
                        .filter(|h| !h.name.is_empty())
                        .map(|h| {
                            (
                                String::from(h.name),
                                core::str::from_utf8(h.value).unwrap_or("").into(),
                            )
                        })
                        .collect();

                    (
                        hdr_len,
                        cl,
                        method,
                        version,
                        url_path,
                        url_query,
                        req_headers,
                    )
                }
                _ => return Err(()),
            }
            // raw_req dropped here, buf immutable borrow released
        };

        // 3. 读取请求体（如果 Content-Length > 0）
        let total_needed = hdr_len + content_length;
        while pos < total_needed {
            if pos >= buf.len() {
                return Err(());
            }
            let n = socket.read(&mut buf[pos..]).await.map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            pos += n;
        }

        // 4. 构建 HttpRequest
        let body = if content_length > 0 && hdr_len + content_length <= pos {
            buf[hdr_len..hdr_len + content_length].to_vec()
        } else {
            Vec::new()
        };

        let mut req = HttpRequest {
            method,
            url_path,
            url_query,
            headers: req_headers,
            body,
            version: if version == 1 { 11 } else { 10 },
        };

        // 5. 检测 WebSocket 升级请求
        #[cfg(feature = "websocket")]
        if req.is_websocket() {
            if let Some(ws_key) = req.get_header("Sec-WebSocket-Key").map(|s| String::from(s)) {
                // 查找匹配的 WebSocket 路由
                let handler = match_ws_route(&req.url_path);
                if let Some(handler) = handler {
                    // 发送 101 Switching Protocols 响应
                    let upgrade_resp = crate::websocket::build_ws_upgrade_response(&ws_key);
                    let mut written = 0;
                    while written < upgrade_resp.len() {
                        let n = socket
                            .write(&upgrade_resp[written..])
                            .await
                            .map_err(|_| ())?;
                        if n == 0 {
                            return Err(());
                        }
                        written += n;
                    }
                    let _ = socket.flush().await;

                    // Safety: socket 的生命周期由 serve() 管理，长于 handler 的 future。
                    // handler 的 future 在此 await 点完成后才会返回，因此 socket 引用有效。
                    let socket_ref: &mut TcpSocket<'_> =
                        unsafe { &mut *(socket as *mut TcpSocket<'_>) };
                    handler(socket_ref).await;
                    return Ok(());
                }
            }
        }

        // 6. 普通 HTTP 路由匹配
        let res = match_route(&mut req);

        // 7. 写入 HTTP 响应
        let response_bytes = format_response(&res);
        let mut written = 0;
        while written < response_bytes.len() {
            let n = socket
                .write(&response_bytes[written..])
                .await
                .map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            written += n;
        }
        let _ = socket.flush().await;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// 在缓冲区中查找 \r\n\r\n 的位置（返回结束位置，即 \r\n\r\n 之后的偏移）
fn find_header_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// 解析 URL 路径和查询参数
fn parse_path_query(path: &str) -> (String, Vec<(String, String)>) {
    if let Some((path_part, query_part)) = path.split_once('?') {
        let query = query_part
            .split('&')
            .filter_map(|pair| {
                pair.split_once('=')
                    .map(|(k, v)| (String::from(k), String::from(v)))
            })
            .collect();
        (String::from(path_part), query)
    } else {
        (String::from(path), Vec::new())
    }
}

/// 根据注册的路由表匹配请求并返回响应
fn match_route(req: &mut HttpRequest) -> HttpResponse {
    unsafe {
        let routes_ptr = core::ptr::addr_of!(ROUTES);
        if let Some(routes) = (*routes_ptr).as_ref() {
            for route in routes.iter() {
                if req.url_path == route.path {
                    if let Some(res) = (route.handler)(req) {
                        return res;
                    }
                }
            }
        }
    }
    HttpResponse::not_found()
}

/// 根据 URL 路径查找匹配的 WebSocket 路由处理器
#[cfg(feature = "websocket")]
fn match_ws_route(path: &str) -> Option<WsHandler> {
    unsafe {
        let routes_ptr = core::ptr::addr_of!(WS_ROUTES);
        if let Some(routes) = (*routes_ptr).as_ref() {
            for route in routes.iter() {
                if path == route.path {
                    return Some(route.handler);
                }
            }
        }
    }
    None
}

/// 将 HttpResponse 格式化为 HTTP/1.1 字节流
fn format_response(res: &HttpResponse) -> Vec<u8> {
    let status_text = match res.http_code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    };

    let mut out = Vec::with_capacity(256 + res.body.len());

    // 状态行
    out.extend_from_slice(
        alloc::format!("HTTP/1.1 {} {}\r\n", res.http_code, status_text).as_bytes(),
    );

    // 响应头
    for (key, value) in &res.headers {
        out.extend_from_slice(alloc::format!("{}: {}\r\n", key, value).as_bytes());
    }

    // Content-Length（如果没有手动设置）
    let has_content_length = res
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Content-Length"));
    if !has_content_length {
        out.extend_from_slice(alloc::format!("Content-Length: {}\r\n", res.body.len()).as_bytes());
    }

    // Connection: close（嵌入式场景下不维持长连接）
    let has_connection = res
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Connection"));
    if !has_connection {
        out.extend_from_slice(b"Connection: close\r\n");
    }

    // 空行 + body
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&res.body);

    out
}
