# 处理函数声明

## 参数

参数参数有两类，一类是获取请求相关对象，一类是定义请求参数。

请求相关对象有三个，分别用于获取请求对象、获取客户端socket地址、获取websocket上下文

```rust
#[http_get("/hello")]
async fn hello(req: HttpRequest, client: std::net::SocketAddr, wsctx: &mut WebsocketContext) -> HttpResponse {
    HttpResponse::html("hello world")
}
```

下面是一个websocket服务器端示例代码：

```rust
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
```

另外就是处理函数的参数了。除了前文提到的鉴权用的参数外，剩余的均要求通过请求的query里或body里附带。示例：

```rust
#[http_get("/hello_user")]
async fn hello_user(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello {name}"))
}

#[http_post("/upload")]
async fn upload(file1: PostFile) -> HttpResponse {
    HttpResponse::html(format!("file[{}] len: {}", file1.filename, file1.data.len()))
}
```

## 返回类型

处理函数返回类型有四种选择：`()`、`anyhow::Result<()>`、`HttpResponse`、`anyhow::Result<HttpResponse>`
