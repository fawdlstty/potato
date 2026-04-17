# Getting Started

Starting with the simplest example, create a Rust project and add the potato dependency:

```bash
cargo new hello_potato
cd hello_potato
cargo add potato
```

## Server Side

Example code:

```rust
#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/hello");
    server.serve_http().await
}
```

### HTTPS / HTTP2 / HTTP3

To enable TLS and newer protocols, turn on features first:

```bash
cargo add potato --features http2,http3
```

Then choose a startup method as needed (certificate and key are PEM files):

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8443");

    // HTTPS (HTTP/1.1 over TLS)
    // server.serve_https("cert.pem", "key.pem").await

    // HTTP/2 (ALPN negotiates h2, with HTTPS/1.1 fallback)
    // server.serve_http2("cert.pem", "key.pem").await

    // HTTP/3 (QUIC)
    // server.serve_http3("cert.pem", "key.pem").await

    // HTTP/3 without encryption (non-standard QUIC, dev/test only)
    // server.serve_http3_without_encrypt().await
}
```

Advanced note: in production, manage cert rotation, ALPN policy, and reverse-proxy config together.

## Client Side

Example code:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut res = potato::get!("https://www.fawdlstty.com").await?;
    println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
    Ok(())
}
```

### Protocol Version Selection

Use `http3()` wrapper to specify HTTP/3 protocol. The library auto-selects encryption mode based on URL scheme:

```rust
// HTTP/3 encrypted mode (https:// URL)
let res = potato::get!(http3("https://127.0.0.1:8443/hello")).await?;

// HTTP/3 without encryption (http:// URL)
let res = potato::get!(http3("http://127.0.0.1:8443/hello")).await?;
```
