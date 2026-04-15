# Using the Client

Client macros support passing URL directly with optional headers:

```rust
let mut res = potato::get!("https://www.fawdlstty.com").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);

// With headers
let mut res = potato::get!("https://www.fawdlstty.com", User_Agent = "my-client").await?;

// POST/PUT second argument is body
let body = vec![];
let mut res = potato::post!("https://www.fawdlstty.com", body, Content_Type = "application/json").await?;
```

All HTTP method macros (`get!`, `post!`, `put!`, `delete!`, `patch!`, `head!`, `options!`, `trace!`, `connect!`) support this syntax.

## HTTP/2 and HTTP/3 Requests

Use `http2()` or `http3()` wrapper to specify protocol version (default HTTP/1.1):

```rust
#[cfg(feature = "http2")]
let mut res = potato::get!(http2("https://www.fawdlstty.com")).await?;

#[cfg(feature = "http3")]
let mut res = potato::post!(http3("https://api.example.com"), body, Custom("X-Key") = "value").await?;
```

- **HTTP/1.1**: Default, use URL directly
- **HTTP/2**: Requires TLS, use `http2("https://...")`
- **HTTP/3**: Requires TLS (QUIC-based), use `http3("https://...")`

Full example: `examples/05_http2_http3_client.rs`

## Sessions and Streaming

**Session reuse** (reuses connection for same host):

```rust
let mut sess = Session::new();
let res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
```

**SSE streaming**:

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

**WebSocket connection**:

```rust
let mut ws = potato::websocket!("ws://127.0.0.1:8080/ws", Custom("X-Key") = "value").await?;
ws.send_text("hello").await?;
let frame = ws.recv().await?;
```

## Custom HTTP Headers

All client macros support mixing standard and custom headers:

```rust
let res = potato::get!(
    "https://api.example.com/data",
    Custom("X-Custom-Header") = "value",  // Custom header (string key)
    User_Agent = "my-client/1.0",          // Standard header (identifier)
    Custom(k) = v                          // Variables supported
).await?;
```

**Syntax rules**:
- **Standard Headers**: Identifiers (e.g., `User_Agent`, `Content_Type`)
- **Custom Headers**: `Custom(key) = value`, key/value can be strings or variables
- Mix freely, comma-separated, trailing comma supported

Full example: `examples/01_client_with_arg.rs`

## Other Features

**Jemalloc memory profiling** (requires `jemalloc` feature):

```rust
potato::init_jemalloc()?;  // At start of main
// ... run program ...
let pdf_data = potato::dump_jemalloc_profile()?;  // Get PDF report
```

**Reverse and forward proxy**:

```rust
// Reverse proxy
let mut session = potato::client::TransferSession::from_reverse_proxy(
    "/api", "http://backend-server:8080"
);

// Forward proxy
let mut session = potato::client::TransferSession::from_forward_proxy();

// Usage: session.transfer(&mut request, modify_content).await?
```
