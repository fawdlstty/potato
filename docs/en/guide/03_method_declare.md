# Handler Function Declaration

## Parameters

Parameters can directly accept request objects, or define custom request parameters. These request parameters will require the HTTP request's query string or body to carry these values. Example request objects:

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

Below is a websocket server-side example code:

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

Additionally, there are handler function parameters. Except for the authentication parameters mentioned earlier, the rest require values to be carried in the request's query or body. Example:

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

## Return Types

Handler functions have four return type options: `()`, `anyhow::Result<()>`, `HttpResponse`, `anyhow::Result<HttpResponse>`

## Preprocess/Postprocess Function Declaration

Both `preprocess` and `postprocess` support `async fn` and regular `fn`.

Preprocess function signature:

```rust
#[potato::preprocess]
fn pre(req: &mut potato::HttpRequest) -> ...
```

Supported return types: `anyhow::Result<Option<potato::HttpResponse>>`, `Option<potato::HttpResponse>`, `anyhow::Result<()>`, `()`

Postprocess function signature:

```rust
#[potato::postprocess]
fn post(req: &mut potato::HttpRequest, res: &mut potato::HttpResponse) -> ...
```

Supported return types: `anyhow::Result<()>`, `()`

You can repeat hook annotations on the same handler, for example:

```rust
#[potato::preprocess(pre1)]
#[potato::preprocess(pre2, pre3)]
#[potato::postprocess(post1)]
#[potato::postprocess(post2)]
```

Execution order is expanded left-to-right and top-to-bottom.
