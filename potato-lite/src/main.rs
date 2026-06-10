//! potato-lite API 用法示例（参考）
//!
//! potato-lite 基于 embassy-net 运行在 no_std 嵌入式环境中。
//! 以下展示各模块的核心 API 调用方式，完整可运行项目需搭配
//! embassy-executor、embassy-net-tuntap 等运行时组件。
//!
//! # Cargo.toml 依赖配置参考
//!
//! ```toml
//! [dependencies]
//! potato-lite = { path = ".." }
//! embassy-executor = { version = "0.7", features = ["arch-std", "executor-thread"] }
//! embassy-net = { version = "0.6", features = ["tcp", "dns", "proto-ipv4", "medium-ip"] }
//! embassy-net-tuntap = { version = "0.2" }
//! embassy-time = { version = "0.4", features = ["std"] }
//! log = "0.4"
//!
//! [features]
//! default = []
//! websocket = ["potato-lite/websocket"]
//! ```
//!
//! # 1. HTTP 服务器
//!
//! ```ignore
//! use embassy_net::Stack;
//!
//! async fn run_server(stack: Stack<'_>) {
//!     let mut server = potato_lite::server::HttpServer::new(8848);
//!
//!     // 配置路由
//!     server.configure(|ctx| {
//!         // 同步处理器: GET /hello → text/plain
//!         ctx.use_custom_sync("/hello", |_req| {
//!             Some(potato_lite::HttpResponse::text("Hello from potato-lite!"))
//!         });
//!
//!         // GET /api → application/json
//!         ctx.use_custom_sync("/api", |_req| {
//!             Some(potato_lite::HttpResponse::json(r#"{"ok":true}"#))
//!         });
//!
//!         // GET /echo → 回显请求体
//!         ctx.use_custom_sync("/echo", |req| {
//!             let body = core::str::from_utf8(&req.body).unwrap_or("");
//!             Some(potato_lite::HttpResponse::text(body))
//!         });
//!     });
//!
//!     server.serve(stack).await;
//! }
//! ```
//!
//! # 2. HTTP 客户端
//!
//! ```ignore
//! use embassy_net::Stack;
//!
//! async fn run_client(stack: Stack<'_>) {
//!     // 方式一：使用宏
//!     let resp = potato_lite::get!(stack, "http://192.168.1.1/api").await.unwrap();
//!
//!     // 方式二：直接调用
//!     let resp = potato_lite::client::get(stack, "http://10.0.0.1/data").await.unwrap();
//!
//!     // resp.http_code: u16 状态码
//!     // resp.body: Vec<u8> 响应体
//!     // resp.headers: Vec<(String, String)> 响应头
//! }
//! ```
//!
//! # 3. WebSocket 服务端（需 `websocket` feature）
//!
//! ```ignore
//! use embassy_net::Stack;
//!
//! async fn run_ws_server(stack: Stack<'_>) {
//!     let mut server = potato_lite::server::HttpServer::new(8849);
//!     server.configure(|ctx| {
//!         ctx.use_websocket("/ws", |socket| {
//!             Box::pin(async move {
//!                 // 回显服务
//!                 loop {
//!                     match potato_lite::websocket::ws_recv(socket).await {
//!                         Ok(potato_lite::WsFrame::Text(t)) => {
//!                             let _ = potato_lite::websocket::ws_send_text(socket, &t).await;
//!                         }
//!                         Ok(potato_lite::WsFrame::Binary(d)) => {
//!                             let _ = potato_lite::websocket::ws_send_binary(socket, d).await;
//!                         }
//!                         Err(_) => break,
//!                     }
//!                 }
//!             })
//!         });
//!     });
//!     server.serve(stack).await;
//! }
//! ```
//!
//! # 4. WebSocket 客户端（需 `websocket` feature）
//!
//! ```ignore
//! async fn run_ws_client(stack: Stack<'_>) {
//!     let mut rx = [0u8; 4096];
//!     let mut tx = [0u8; 4096];
//!
//!     // 方式一：使用宏
//!     let mut ws = potato_lite::websocket!(stack, "ws://10.0.0.1/ws", &mut rx, &mut tx).await.unwrap();
//!
//!     // 方式二：直接调用
//!     let mut ws = potato_lite::websocket::Websocket::connect(
//!         stack, "ws://10.0.0.1/ws", &mut rx, &mut tx,
//!     ).await.unwrap();
//!
//!     ws.send_text("hello").await.unwrap();
//!     match ws.recv().await {
//!         Ok(potato_lite::WsFrame::Text(t)) => { /* 处理文本消息 */ }
//!         Ok(potato_lite::WsFrame::Binary(d)) => { /* 处理二进制消息 */ }
//!         Err(_) => {}
//!     }
//!     let _ = ws.send_close().await;
//! }
//! ```
//!
//! # 5. HttpResponse 构建
//!
//! ```ignore
//! // 纯文本响应 (text/plain)
//! let _ = potato_lite::HttpResponse::text("hello");
//!
//! // JSON 响应 (application/json)
//! let _ = potato_lite::HttpResponse::json(r#"{"status":"ok"}"#);
//!
//! // 404 响应
//! let _ = potato_lite::HttpResponse::not_found();
//!
//! // 自定义响应
//! let mut res = potato_lite::HttpResponse::new();
//! res.http_code = 201;
//! res.add_header("X-Custom", "value");
//! res.body = b"created".to_vec();
//! ```
//!
//! # 6. 完整嵌入式入口模式
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! #[panic_handler]
//! fn panic(_: &core::panic::PanicInfo) -> ! {
//!     loop {}
//! }
//!
//! #[embassy_executor::main]
//! async fn main(spawner: embassy_executor::Spawner) -> ! {
//!     // 1. 创建 TUN 设备（Linux 需 root / CAP_NET_ADMIN）
//!     let mut tun = embassy_net_tuntap::Tun::new("tun0").unwrap();
//!     tun.set_address("10.0.0.2".parse().unwrap());
//!     tun.set_up().unwrap();
//!
//!     // 2. 初始化网络栈
//!     // ...
//!
//!     // 3. 启动 HTTP 服务器
//!     let mut server = potato_lite::server::HttpServer::new(80);
//!     server.configure(|ctx| {
//!         ctx.use_custom_sync("/", |_req| {
//!             Some(potato_lite::HttpResponse::text("Hello, embedded world!"))
//!         });
//!     });
//!     server.serve(stack).await;
//! }
//! ```
//!
//! # 便捷宏速查
//!
//! | 宏 | 用途 | 等价调用 |
//! |---|---| --- |
//! | `potato_lite::get!` | HTTP GET 请求 | `potato_lite::client::get(stack, url)` |
//! | `potato_lite::websocket!` | WebSocket 连接 | `potato_lite::websocket::Websocket::connect(...)` |

#[potato_lite::http_get("/hello")]
async fn hello() -> potato_lite::HttpResponse {
    potato_lite::HttpResponse::html("hello world")
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    let mut server = potato_lite::HttpServer::new(8080);
    println!("visit: http://127.0.0.1:8080/hello");
    // 使用宏生成的包装函数注册路由
    server.configure(|ctx| {
        ctx.use_custom_sync("/hello", __potato_lite_wrap_hello);
    });
    // serve_http 需要 embassy-net Stack，此处仅为宏功能演示
    loop {}
}
