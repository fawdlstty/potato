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

如果要启用 TLS 与新协议，请先打开特性：

```bash
cargo add potato --features tls,http2,http3
```

然后按需切换启动方法（证书和私钥都使用 PEM）：

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8443");
    server.serve_http().await

    // HTTPS（HTTP/1.1 over TLS）
    // server.serve_https("cert.pem", "key.pem").await

    // HTTP/2（ALPN 协商 h2，仍可回退到 HTTPS/1.1）
    // server.serve_http2("cert.pem", "key.pem").await

    // HTTP/3（QUIC）
    // server.serve_http3("cert.pem", "key.pem").await

    // HTTP/3 无加密（非标准 QUIC，仅用于开发/测试）
    // server.serve_http3_without_encrypt().await
}
```

高级用法提示：生产环境建议将证书轮换、ALPN 策略与反向代理配置一起管理。

## 客户端

示例代码：

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut res = potato::get!("https://www.fawdlstty.com").await?;
    println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
    Ok(())
}
```

### 协议版本选择

使用 `http3()` 包装器指定 HTTP/3 协议，库会根据 URL scheme 自动选择加密模式：

```rust
// HTTP/3 加密模式 (https:// URL)
let res = potato::get!(http3("https://127.0.0.1:8443/hello")).await?;

// HTTP/3 无加密模式 (http:// URL)
let res = potato::get!(http3("http://127.0.0.1:8443/hello")).await?;
```
