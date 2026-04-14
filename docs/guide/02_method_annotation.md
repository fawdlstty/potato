# 处理函数标注

函数标注为服务器端处理函数专用，用于指定处理函数的HTTP方法和路径。当前支持六种，分别为：

- `http_get`：GET方法
- `http_post`：POST方法
- `http_put`：PUT方法
- `http_delete`：DELETE方法
- `http_head`：HEAD方法
- `http_options`：OPTIONS方法

这六种除了描述处理方法外，其他特性完全一致。通过修改标注名即可实现不同的HTTP方法。

标注有两种用法，一种是直接传递请求路径。示例：

```rust
#[potato::http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

另一种用法是指定请求路径和鉴权参数。示例：

```rust
#[potato::http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

// 注：鉴权参数通过如下方式签发
let token = ServerAuth::jwt_issue("payload".to_string(), std::time::Duration::from_secs(10000000)).await?;

// 注：鉴权token通过如下形式修改，不指定默认即每次随机生成（通常在main函数入口的地方修改一次）
ServerConfig::set_jwt_secret("AAABBBCCC").await;
```

当函数标注鉴权参数后，鉴权不通过，会返回401状态码，且不会实际调用处理函数。

## 返回类型

处理函数支持多种返回类型：

- `HttpResponse` - 直接返回 HTTP 响应
- `anyhow::Result<HttpResponse>` - 返回可能出错的 HTTP 响应
- `()` - 无返回值，自动响应 "ok"
- `Result<()>` - 返回可能出错的操作
- `String` / `&'static str` - 返回字符串，自动通过 `HttpResponse::html()` 包装
- `anyhow::Result<String>` / `anyhow::Result<&'static str>` - 返回可能出错的字符串

示例：

```rust
// 返回 String
#[potato::http_get("/string")]
async fn string_handler() -> String {
    "<h1>Hello</h1>".to_string()
}

// 返回 &'static str
#[potato::http_get("/static")]
async fn static_handler() -> &'static str {
    "<h1>Static</h1>"
}

// 返回 Result<String>
#[potato::http_get("/result")]
async fn result_handler(success: bool) -> anyhow::Result<String> {
    if success {
        Ok("<h1>Success</h1>".to_string())
    } else {
        anyhow::bail!("Error")
    }
}
```

## 预处理与后处理

可以在处理函数上叠加 `preprocess`、`postprocess` 标注，用于在处理函数前后执行固定签名的钩子函数。

```rust
#[potato::preprocess]
async fn pre1(req: &mut potato::HttpRequest) -> anyhow::Result<Option<potato::HttpResponse>> {
    Ok(None)
}

#[potato::postprocess]
async fn post1(req: &mut potato::HttpRequest, res: &mut potato::HttpResponse) -> anyhow::Result<()> {
    Ok(())
}

#[potato::http_get("/hello")]
#[potato::preprocess(pre1)]
#[potato::postprocess(post1)]
#[potato::postprocess(post2)]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}
```

- `preprocess` 按声明顺序执行；若某个预处理返回 `HttpResponse`，会跳过实际 handler，但仍会继续执行 `postprocess`。
- `postprocess` 按声明顺序执行，接收最终 `HttpResponse` 并可原地修改。
- `preprocess`/`postprocess` 支持拆分多行声明，顺序按“从左到右、从上到下”执行。
