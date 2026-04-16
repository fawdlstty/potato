# 处理函数标注与声明

函数标注为服务器端处理函数专用，用于指定处理函数的HTTP方法和路径。当前支持六种，分别为：

- `http_get`：GET方法
- `http_post`：POST方法
- `http_put`：PUT方法
- `http_delete`：DELETE方法
- `http_head`：HEAD方法
- `http_options`：OPTIONS方法

这六种除了描述处理方法外，其他特性完全一致。通过修改标注名即可实现不同的HTTP方法。

标注用于指定处理函数的HTTP方法和路径，直接传递路径即可。示例：

```rust
#[potato::http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

如需用户会话管理，请使用 SessionCache（见下方 SessionCache 参数章节）。

## 参数

参数可以直接接受请求对象,也可以定义自定义请求参数,这请求参数将要求HTTP请求的query string或者body附带此值。示例请求对象:

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

下面是一个websocket服务器端示例代码:

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

另外就是处理函数的参数了。除了前文提到的鉴权用的参数外,剩余的均要求通过请求的query里或body里附带。示例:

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

### OnceCache 参数

支持 `cache: &mut OnceCache` 参数,用于在单次请求的前处理、后处理及handler方法间传递参数。收到请求时创建,请求完成后自动释放。

```rust
#[potato::preprocess]
async fn pre_handler(req: &mut HttpRequest, cache: &mut OnceCache) {
    cache.set("user_id", 12345u32);
}

#[potato::http_get("/profile")]
#[potato::preprocess(pre_handler)]
async fn get_profile(cache: &mut OnceCache) -> HttpResponse {
    let user_id: u32 = *cache.get::<u32>("user_id");
    HttpResponse::text(format!("User: {}", user_id))
}

#[potato::postprocess]
fn post_handler(_req: &mut HttpRequest, res: &mut HttpResponse, cache: &mut OnceCache) {
    cache.set("processed", true);
}
```

常用方法:
- `cache.get::<T>(name)` - 获取不可变引用（返回Option<&T>）
- `cache.get_or_default::<T>(name, default)` - 获取值或返回默认值（需Clone）
- `cache.get_mut::<T>(name)` - 获取可变引用  
- `cache.set::<T>(name, value)` - 设置值
- `cache.remove::<T>(name)` - 移除并返回值

### SessionCache 参数

支持 `cache: &mut SessionCache` 参数,用于跨请求保持用户会话数据。只需声明此参数,宏会自动验证 Bearer token 并加载对应会话。

```rust
// 登录接口签发token
#[potato::http_post("/login")]
async fn login(req: &mut HttpRequest) -> HttpResponse {
    let user_id = 12345i64; // 从请求获取
    let token = SessionCache::generate_token(user_id, std::time::Duration::from_secs(3600)).unwrap();
    HttpResponse::json(serde_json::json!({ "token": token }))
}

// 直接使用SessionCache,宏自动处理token验证
#[potato::http_get("/profile")]
async fn get_profile(cache: &mut SessionCache) -> HttpResponse {
    let count: u32 = cache.get("visits").unwrap_or(0);
    cache.set("visits", count + 1);
    HttpResponse::text(format!("Visits: {}", count + 1))
}

// 与OnceCache同时使用
#[potato::http_get("/data")]
async fn get_data(once: &mut OnceCache, session: &mut SessionCache) -> HttpResponse {
    once.set("req_id", "abc"); // 单次请求
    session.set("user", "john"); // 跨请求保持
    HttpResponse::text("ok")
}
```

常用方法:
- `SessionCache::set_jwt_secret(secret)` - 设置JWT密钥(全局)
- `SessionCache::generate_token(user_id, duration)` - 签发token
- `cache.get::<T>(key)` - 获取值(需Clone)
- `cache.set::<T>(key, value)` - 设置值
- `cache.with_get::<T>(key, |v| ...)` - 读取并处理
- `cache.with_mut::<T>(key, |v| ...)` - 可变引用处理

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

## 响应头标注

可通过 `#[header(...)]` 为处理函数添加响应头，支持标准头和自定义头：

```rust
// 标准头（使用下划线命名）
#[potato::http_get("/api")]
#[header(Cache_Control = "no-cache")]
async fn api_handler() -> HttpResponse {
    HttpResponse::text("response")
}

// 自定义头（使用 Custom 语法）
#[potato::http_get("/custom")]
#[header(Custom("X-Custom-Header") = "custom-value")]
async fn custom_header() -> String {
    "custom header".to_string()
}

// 多个header混合使用
#[potato::http_get("/multi")]
#[header(Cache_Control = "no-store")]
#[header(Custom("X-Custom") = "value")]
async fn multi_headers() -> String {
    "multiple headers".to_string()
}
```

## 预处理与后处理

可以在处理函数上叠加 `preprocess`、`postprocess` 标注，用于在处理函数前后执行固定签名的钩子函数。

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

示例：

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

同一个 handler 上可重复标注多行，例如：

```rust
#[potato::preprocess(pre1)]
#[potato::preprocess(pre2, pre3)]
#[potato::postprocess(post1)]
#[potato::postprocess(post2)]
```

- `preprocess` 按声明顺序执行；若某个预处理返回 `HttpResponse`，会跳过实际 handler，但仍会继续执行 `postprocess`。
- `postprocess` 按声明顺序执行，接收最终 `HttpResponse` 并可原地修改。
- `preprocess`/`postprocess` 支持拆分多行声明，顺序按"从左到右、从上到下"执行。

## CORS标注

通过 `#[potato::cors(...)]` 为处理函数启用CORS（跨域资源共享）支持。该标注会自动添加CORS响应头，并为PUT/POST/DELETE方法自动生成HEAD方法支持。

### 基本用法

```rust
// 最简用法：使用最小限制默认值
// - Access-Control-Allow-Origin: *
// - Access-Control-Allow-Headers: *
// - Access-Control-Allow-Methods: 自动计算
// - Access-Control-Max-Age: 86400
#[potato::http_get("/api/data")]
#[potato::cors]
async fn get_data() -> &'static str {
    "data"
}
```

### 自定义配置

```rust
// 限制来源和方法
#[potato::http_post("/api/create")]
#[potato::cors(
    origin = "https://example.com",
    methods = "GET,POST,PUT,DELETE",
    headers = "Content-Type,Authorization",
    max_age = "3600"
)]
async fn create_item() -> &'static str {
    "created"
}

// 允许携带凭证（cookies）
#[potato::http_put("/api/update")]
#[potato::cors(
    origin = "https://secure.example.com",
    methods = "GET,PUT",
    credentials = true
)]
async fn update_item() -> &'static str {
    "updated"
}
```

### 参数说明

- `origin`: 允许的来源域名，默认 `*`（允许所有）
- `methods`: 允许的方法列表，默认自动计算（扫描所有已注册方法）
- `headers`: 允许的请求头，默认 `*`（允许所有）
- `max_age`: 预检请求缓存时间（秒），默认 `86400`（24小时）
- `credentials`: 是否允许携带凭证（cookies），默认 `false`
- `expose_headers`: 允许浏览器访问的响应头

### 智能特性

- **自动HEAD支持**：PUT/POST/DELETE方法标注cors后，自动生成对应的HEAD方法处理
- **自动OPTIONS处理**：CORS预检请求自动返回正确的允许方法列表
- **方法补充**：指定的methods会自动补充HEAD和OPTIONS
- **凭证模式约束**：当`credentials=true`时，origin不能使用`*`，必须指定具体域名

## 请求体大小限制标注

通过 `#[potato::limit_size(...)]` 为处理函数设置请求体大小限制，返回 413 Payload Too Large。

### 基本用法

```rust
// 限制 body 为 10MB
#[potato::http_post("/upload")]
#[potato::limit_size(10 * 1024 * 1024)]
async fn upload(req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::text("uploaded")
}
```

### 分别限制 header 和 body

```rust
// Header 512KB, Body 50MB
#[potato::http_post("/large-upload")]
#[potato::limit_size(header = 512 * 1024, body = 50 * 1024 * 1024)]
async fn large_upload(req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::text("large uploaded")
}
```

### 优先级

- Handler 注解 > 中间件 `use_limit_size` > 全局配置（默认 100MB）
- 注解仅对当前 handler 生效，覆盖全局和中间件限制

## 传输速率限制

通过 `use_transfer_limit` 中间件限制连接的数据传输速率（单位：bits/sec）。

**支持情况**：
- ✅ **HTTP/1.1**：完全支持，基于 TCP 流的令牌桶算法实现
- ❌ **HTTP/2**：不支持（使用 h2 框架，未实现速率限制）
- ❌ **HTTP/3**：不支持（使用 QUIC/h3 框架，未实现速率限制）

```rust
server.configure(|ctx| {
    // 入站 10 Mbps，出站 20 Mbps
    ctx.use_transfer_limit(10_000_000, 20_000_000);
    ctx.use_handlers();
});
```

- 入站速率：限制接收请求数据的速度
- 出站速率：限制发送响应数据的速度
- 超出限制时自动延迟传输
- 仅对 HTTP/1.1 连接生效
