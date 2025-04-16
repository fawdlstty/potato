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
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

另一种用法是指定请求路径和鉴权参数。示例：

```rust
#[http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

// 注：鉴权参数通过如下方式签发
let token = ServerAuth::jwt_issue("payload".to_string(), std::time::Duration::from_secs(10000000)).await?;

// 注：鉴权token通过如下形式修改，不指定默认即每次随机生成（通常在main函数入口的地方修改一次）
ServerConfig::set_jwt_secret("AAABBBCCC").await;
```

当函数标注鉴权参数后，鉴权不通过，会返回401状态码，且不会实际调用处理函数。
