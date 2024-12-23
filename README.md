# potato

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

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.run().await;
}
```

如上所示，定义一个HTTP请求处理函数非常简洁。通过将 `http_get` 替换为 `http_post`、`http_put`、`http_delete`、`http_options`、`http_head` 可创建对应请求的处理函数。

HTTP请求处理函数可包含以下类型参数：

- `req: potato::HttpRequest` **请求结构体**
- `client: std::net::SocketAddr` **客户端IP**
- `wsctx: &mut potato::WebsocketContext` **升级Websocket连接的上下文对象**

示例参数完全体：

```rust
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
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

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

#[http_get("/ws")]
async fn websocket(req: HttpRequest, wsctx: &mut WebsocketContext) -> anyhow::Result<()> {
    let mut ws = wsctx.upgrade_websocket(&req).await?;
    loop {
        let frame = ws.read_frame().await?;
        match frame {
            WsFrame::Text(text) => ws.write_frame(WsFrame::Text(text)).await?,
            WsFrame::Binary(bin) => ws.write_frame(WsFrame::Binary(bin)).await?,
            WsFrame::Ping => ws.write_frame(WsFrame::Pong).await?,
            WsFrame::Close => break,
            WsFrame::Pong => (),
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.run().await;
}
```

<!--
# TODO

- file
- server session
- middleware
- http client
- cookie
- chunked
- CORS
-->
