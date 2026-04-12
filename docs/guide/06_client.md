# 使用客户端

客户端宏支持直接传 URL，并可选追加请求头：

```rust
let mut res = potato::get!("https://www.fawdlstty.com").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);

// 带请求头
let mut res = potato::get!("https://www.fawdlstty.com", User_Agent = "my-client").await?;

// POST/PUT 第二个参数是 body
let body = vec![];
let mut res = potato::post!("https://www.fawdlstty.com", body, Content_Type = "application/json").await?;
```

所有 HTTP 方法宏（`get!`、`post!`、`put!`、`delete!`、`patch!`、`head!`、`options!`、`trace!`、`connect!`）均支持此语法。

## HTTP/2 和 HTTP/3 请求

使用 `http2()` 或 `http3()` 包装器指定协议版本（默认 HTTP/1.1）：

```rust
#[cfg(feature = "http2")]
let mut res = potato::get!(http2("https://www.fawdlstty.com")).await?;

#[cfg(feature = "http3")]
let mut res = potato::post!(http3("https://api.example.com"), body, Custom("X-Key") = "value").await?;
```

- **HTTP/1.1**: 默认，直接使用 URL
- **HTTP/2**: 需 TLS，使用 `http2("https://...")`
- **HTTP/3**: 需 TLS（基于 QUIC），使用 `http3("https://...")`

完整示例：`examples/05_http2_http3_client.rs`

## 会话与流式

**会话复用**（相同路径复用连接）：

```rust
let mut sess = Session::new();
let res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
```

**SSE 流式响应**：

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

**WebSocket 连接**：

```rust
let mut ws = potato::websocket!("ws://127.0.0.1:8080/ws", Custom("X-Key") = "value").await?;
ws.send_text("hello").await?;
let frame = ws.recv().await?;
```

## 自定义 HTTP Header

所有客户端宏支持混合使用标准 Header 和 Custom Header：

```rust
let res = potato::get!(
    "https://api.example.com/data",
    Custom("X-Custom-Header") = "value",  // Custom header（字符串 key）
    User_Agent = "my-client/1.0",          // Standard header（标识符）
    Custom(k) = v                          // 支持变量
).await?;
```

**语法规则**：
- **标准 Header**: 标识符（如 `User_Agent`、`Content_Type`）
- **Custom Header**: `Custom(key) = value`，key/value 可为字符串或变量
- 混合使用，逗号分隔，支持尾随逗号

完整示例：`examples/01_client_with_arg.rs`

## 其他功能

**Jemalloc 内存分析**（需启用 `jemalloc` feature）：

```rust
potato::init_jemalloc()?;  // main 函数开始处
// ... 运行程序 ...
let pdf_data = potato::dump_jemalloc_profile()?;  // 获取 PDF 报告
```

**反向代理与转发会话**：

```rust
// 反向代理
let mut session = potato::client::TransferSession::from_reverse_proxy(
    "/api", "http://backend-server:8080"
);

// 正向代理
let mut session = potato::client::TransferSession::from_forward_proxy();

// 使用: session.transfer(&mut request, modify_content).await?
```
