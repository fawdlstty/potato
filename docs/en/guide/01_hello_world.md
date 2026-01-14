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

## Client Side

Example code:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
```