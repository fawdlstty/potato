# potato

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Fpotato%2Fmain%2F/potato/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/potato/rust.yml)

高性能、简洁语法的HTTP框架。

# 用法

加入库的引用：

```sh
cargo add potato
cargo add tokio --features full
```

最简单的示例：

```rust
use potato::*;

// http://127.0.0.1:80/hello
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

// OpenAPI doc at http://127.0.0.1:80/doc/
declare_doc_path!("/doc/");

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:80"); // 0.0.0.0:443
    _ = server.serve_http().await;
    // _ = server.serve_https("cert.pem", "key.pem").await;
}
```

如上所示，定义一个HTTP请求处理函数非常简洁。通过将 `http_get` 替换为 `http_post`、`http_put`、`http_delete`、`http_options`、`http_head` 可创建对应请求的处理函数。

HTTP请求处理函数可直接指定String、i32等类型的参数，可简化从body或url query提取的步骤，简化开发。示例：

```rust
// http://127.0.0.1:8080/hello?name=miku
#[http_get("/hello")]
async fn hello(name: String) -> HttpResponse {
    HttpResponse::html("hello world, {}!")
}
```

对于POST或PUT等请求来说，可以在参数直接接受文件：

```rust
// http://127.0.0.1:8080/test
#[http_post("/test")]
async fn test(file1: PostFile) -> HttpResponse {
    HttpResponse::html(format!("file[{}] len: {}", file1.filename, file1.data.len()))
}
```

HTTP请求处理函数还可包含以下类型参数：

- `req: potato::HttpRequest` **请求结构体**
- `client: std::net::SocketAddr` **客户端IP**
- `wsctx: &mut potato::WebsocketContext` **升级Websocket连接的上下文对象**

示例参数完全体：

```rust
// http://127.0.0.1:8080/hello
#[http_get("/hello")]
async fn hello(req: HttpRequest, client: std::net::SocketAddr, wsctx: &mut WebsocketContext) -> HttpResponse {
    todo!()
}
```

按需加入即可，不需要的参数可省略。

HTTP请求处理函数返回类型支持以下几种格式：

- `anyhow::Result<()>`
- `anyhow::Result<HttpResponse>`
- `()`
- `HttpResponse`

示例Websocket：

```rust
// http://127.0.0.1:8080
#[http_get("/")]
async fn index() -> HttpResponse {
    HttpResponse::html(r#"<!DOCTYPE html><html>
        <head><title>Websocket Test</title></head>
        <body>
            <h1>Websocket Test</h1>
            <div id="status"><p><em>Connecting...</em></p></div>
            <script>
                const status = document.getElementById('status');
                const ws = new WebSocket(`ws://${location.host}/ws`);
                ws.onopen = function() {
                    status.innerHTML = '<p><em>Connected!</em></p>';
                };
            </script>
        </body>
    </html>"#)
}

// ws://127.0.0.1:8080/ws
#[http_get("/ws")]
async fn websocket(req: HttpRequest, wsctx: &mut WebsocketContext) -> anyhow::Result<()> {
    let mut ws = wsctx.upgrade_websocket(&req).await?;
    ws.send_ping().await?;
    loop {
        match ws.recv_frame().await? {
            WsFrame::Text(text) => ws.send_text(&text).await?,
            WsFrame::Binary(bin) => ws.send_binary(bin).await?,
        }
    }
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.serve_http().await;
}
```

<!--
# TODO

- static path security
- file for download
- openapi
- doc
- server session
- middleware
- http client
- cookie
- chunked
- CORS
-->
