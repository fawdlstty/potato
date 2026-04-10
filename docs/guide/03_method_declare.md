# 处理函数声明

## 参数

参数可以直接接受请求对象，也可以定义自定义请求参数，这请求参数将要求HTTP请求的query string或者body附带此值。示例请求对象：

```rust
#[potato::http_get("/hello")]
async fn hello(req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::html("hello world")
}

#[potato::http_get("/hello")]
async fn hello2(req: &mut HttpRequest) -> anyhow::Result<HttpResponse> {
    let addr = req.get_client_addr().await?;
    Ok(HttpResponse::html(format!("hello client: {addr:?}")))
}
```

下面是一个websocket服务器端示例代码：

```rust
#[potato::http_get("/ws")]
async fn websocket(req: &mut HttpRequest) -> anyhow::Result<()> {
    let mut ws = req.upgrade_websocket().await?;
    ws.send_ping().await?;
    loop {
        match ws.recv().await? {
            WsFrame::Text(text) => ws.send_text(&text).await?,
            WsFrame::Binary(bin) => ws.send_binary(bin).await?,
        }
    }
}
```

另外就是处理函数的参数了。除了前文提到的鉴权用的参数外，剩余的均要求通过请求的query里或body里附带。示例：

```rust
#[potato::http_get("/hello_user")]
async fn hello_user(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello {name}"))
}

#[potato::http_post("/upload")]
async fn upload(file1: PostFile) -> HttpResponse {
    HttpResponse::html(format!("file[{}] len: {}", file1.filename, file1.data.len()))
}
```

## 返回类型

处理函数返回类型有四种选择：`()`、`anyhow::Result<()>`、`HttpResponse`、`anyhow::Result<HttpResponse>`

## 预处理/后处理函数声明

`preprocess` 与 `postprocess` 都支持 `async fn` 或普通 `fn`。

预处理函数签名固定为：

```rust
#[potato::preprocess]
fn pre(req: &mut potato::HttpRequest) -> ...
```

可选返回类型：`anyhow::Result<Option<potato::HttpResponse>>`、`Option<potato::HttpResponse>`、`anyhow::Result<()>`、`()`

后处理函数签名固定为：

```rust
#[potato::postprocess]
fn post(req: &mut potato::HttpRequest, res: &mut potato::HttpResponse) -> ...
```

可选返回类型：`anyhow::Result<()>`、`()`

同一个 handler 上可重复标注多行，例如：

```rust
#[potato::preprocess(pre1)]
#[potato::preprocess(pre2, pre3)]
#[potato::postprocess(post1)]
#[potato::postprocess(post2)]
```

执行顺序按“从左到右、从上到下”展开。
