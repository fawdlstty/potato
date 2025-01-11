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

### Hello World

```rust
use potato::*;

// http://127.0.0.1:80/hello
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:80");
    _ = server.serve_http().await;
}
```

如上所示，定义一个HTTP请求处理函数非常简洁。通过将 `http_get` 替换为 `http_post`、`http_put`、`http_delete`、`http_options`、`http_head` 可创建对应请求的处理函数。

### HTTPS

修改main函数代码为以下内容

```rust
#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:443");
    _ = server.serve_https("cert.pem", "key.pem").await;
}
```

### OpenAPI

源码里任意位置加入以下代码

```rust
// OpenAPI doc at http://127.0.0.1:80/doc/
declare_doc_path!("/doc/");
```

### 参数解析

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

### 处理函数返回类型

HTTP请求处理函数返回类型支持以下几种格式：

- `anyhow::Result<()>`
- `anyhow::Result<HttpResponse>`
- `()`
- `HttpResponse`

### Websocket

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

### JWT鉴权

JWT鉴权功能本质提供签发Token，可附带一串字符串（通常为用户标识符），存储于客户端；也可用于校验用户端Token，验证成功获取附带内容。

```rust
// 签发Token，并传入附带数据、指定过期时间。附带内容尽可能短
#[http_get("/issue")]
async fn issue(payload: String) -> anyhow::Result<HttpResponse> {
    let token = server::JwtAuth::issue(payload, Duration::from_secs(10000000)).await?;
    Ok(HttpResponse::html(token))
}

// 校验Token，并获取附带内容
// 实际请求需在HTTP header里加入：`Authorization: Bearer XXXXXXXXXXXXtoken`
#[http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

// 可选：在程序入口点指定secret key，如果不指定即为随机字符串
#[tokio::main]
async fn main() {
    potato::server::JwtAuth::set_secret("AABBCCDD").await;

    // ...
}
```

上述check函数进入即代表Token有效且未过期，参数里直接获取签发时附带的信息。上述鉴权已在OpenAPI里直接支持，可通过接口文档查看如何调用。

<!--
# TODO

- static path security
- file for download
- http client
- cookie
- chunked
- CORS
-->
