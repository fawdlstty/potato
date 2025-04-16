# 入门示例

从最简单的示例开始，创建Rust项目，并加入potato依赖：

```bash
cargo new hello_potato
cd hello_potato
cargo add potato
```

## 服务器端

示例代码：

```rust
use potato::*;

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/hello");
    server.serve_http().await
}
```

## 客户端

示例代码：

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
```
