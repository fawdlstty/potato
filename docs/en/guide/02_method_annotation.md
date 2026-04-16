# Handler Function Annotations and Declaration

Function annotations are dedicated to server-side handler functions, used to specify the HTTP method and path for the handler function. Currently six methods are supported:

- `http_get`: GET method
- `http_post`: POST method
- `http_put`: PUT method
- `http_delete`: DELETE method
- `http_head`: HEAD method
- `http_options`: OPTIONS method

These six methods are identical in functionality except for describing the processing method. Different HTTP methods can be achieved by modifying the annotation name.

Annotations specify the HTTP method and path for handler functions. Simply pass the path directly. Example:

```rust
#[potato::http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

For user session management, please use SessionCache (see SessionCache Parameter section below).

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

### OnceCache Parameter

Supports `cache: &mut OnceCache` parameter for passing data between preprocess, postprocess, and handler methods within a single request. Created when request is received, automatically released after request completes.

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

Common methods:
- `cache.get::<T>(name)` - Get immutable reference (returns Option<&T>)
- `cache.get_or_default::<T>(name, default)` - Get value or return default (requires Clone)
- `cache.get_mut::<T>(name)` - Get mutable reference
- `cache.set::<T>(name, value)` - Set value
- `cache.remove::<T>(name)` - Remove and return value

### SessionCache Parameter

Supports `cache: &mut SessionCache` parameter for maintaining user session data across requests. Just declare this parameter, and the macro will automatically validate the Bearer token and load the corresponding session.

```rust
// Login endpoint to issue token
#[potato::http_post("/login")]
async fn login(req: &mut HttpRequest) -> HttpResponse {
    let user_id = 12345i64; // Get from request
    let token = SessionCache::generate_token(user_id, std::time::Duration::from_secs(3600)).unwrap();
    HttpResponse::json(serde_json::json!({ "token": token }))
}

// Use SessionCache directly, macro handles token validation
#[potato::http_get("/profile")]
async fn get_profile(cache: &mut SessionCache) -> HttpResponse {
    let count: u32 = cache.get("visits").unwrap_or(0);
    cache.set("visits", count + 1);
    HttpResponse::text(format!("Visits: {}", count + 1))
}

// Use with OnceCache simultaneously
#[potato::http_get("/data")]
async fn get_data(once: &mut OnceCache, session: &mut SessionCache) -> HttpResponse {
    once.set("req_id", "abc"); // Single request
    session.set("user", "john"); // Persists across requests
    HttpResponse::text("ok")
}
```

Common methods:
- `SessionCache::set_jwt_secret(secret)` - Set JWT secret (global)
- `SessionCache::generate_token(user_id, duration)` - Issue token
- `cache.get::<T>(key)` - Get value (requires Clone)
- `cache.set::<T>(key, value)` - Set value
- `cache.with_get::<T>(key, |v| ...)` - Read and process
- `cache.with_mut::<T>(key, |v| ...)` - Mutable reference processing

## Return Types

Handler functions support multiple return types:

- `HttpResponse` - Direct HTTP response
- `anyhow::Result<HttpResponse>` - HTTP response that may error
- `()` - No return value, automatically responds with "ok"
- `Result<()>` - Operation that may error
- `String` / `&'static str` - String return, automatically wrapped with `HttpResponse::html()`
- `anyhow::Result<String>` / `anyhow::Result<&'static str>` - String that may error

Example:

```rust
// Return String
#[potato::http_get("/string")]
async fn string_handler() -> String {
    "<h1>Hello</h1>".to_string()
}

// Return &'static str
#[potato::http_get("/static")]
async fn static_handler() -> &'static str {
    "<h1>Static</h1>"
}

// Return Result<String>
#[potato::http_get("/result")]
async fn result_handler(success: bool) -> anyhow::Result<String> {
    if success {
        Ok("<h1>Success</h1>".to_string())
    } else {
        anyhow::bail!("Error")
    }
}
```

## Response Header Annotation

Add response headers to handlers using `#[header(...)]`, supporting both standard and custom headers:

```rust
// Standard header (underscore naming)
#[potato::http_get("/api")]
#[header(Cache_Control = "no-cache")]
async fn api_handler() -> HttpResponse {
    HttpResponse::text("response")
}

// Custom header (using Custom syntax)
#[potato::http_get("/custom")]
#[header(Custom("X-Custom-Header") = "custom-value")]
async fn custom_header() -> String {
    "custom header".to_string()
}

// Multiple headers (mixed usage)
#[potato::http_get("/multi")]
#[header(Cache_Control = "no-store")]
#[header(Custom("X-Custom") = "value")]
async fn multi_headers() -> String {
    "multiple headers".to_string()
}
```

## Preprocess and Postprocess

You can stack `preprocess` and `postprocess` annotations on a handler to run fixed-signature hooks before and after the handler.

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

Example:

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

You can repeat hook annotations on the same handler, for example:

```rust
#[potato::preprocess(pre1)]
#[potato::preprocess(pre2, pre3)]
#[potato::postprocess(post1)]
#[potato::postprocess(post2)]
```

- `preprocess` hooks run in declaration order; if one returns an `HttpResponse`, the handler is skipped but `postprocess` hooks still run.
- `postprocess` hooks run in declaration order and can mutate the final `HttpResponse` in place.
- `preprocess`/`postprocess` can be split across multiple annotation lines; execution order is left-to-right and top-to-bottom.

## CORS Annotation

Enable CORS (Cross-Origin Resource Sharing) support for handler functions using `#[potato::cors(...)]`. This annotation automatically adds CORS response headers and generates HEAD method support for PUT/POST/DELETE methods.

### Basic Usage

```rust
// Minimal usage: uses minimal restriction defaults
// - Access-Control-Allow-Origin: *
// - Access-Control-Allow-Headers: *
// - Access-Control-Allow-Methods: auto-calculated
// - Access-Control-Max-Age: 86400
#[potato::http_get("/api/data")]
#[potato::cors]
async fn get_data() -> &'static str {
    "data"
}
```

### Custom Configuration

```rust
// Restrict origin and methods
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

// Allow credentials (cookies)
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

### Parameters

- `origin`: Allowed origin domain, default `*` (allow all)
- `methods`: Allowed methods list, default auto-calculated (scans all registered methods)
- `headers`: Allowed request headers, default `*` (allow all)
- `max_age`: Preflight request cache time in seconds, default `86400` (24 hours)
- `credentials`: Whether to allow credentials (cookies), default `false`
- `expose_headers`: Response headers that browsers are allowed to access

### Smart Features

- **Automatic HEAD Support**: PUT/POST/DELETE methods with cors annotation automatically generate corresponding HEAD method handlers
- **Automatic OPTIONS Handling**: CORS preflight requests automatically return correct allowed methods list
- **Method Supplementation**: Specified methods automatically include HEAD and OPTIONS
- **Credentials Mode Constraint**: When `credentials=true`, origin cannot be `*`, must specify a concrete domain

## Request Body Size Limit Annotation

Set request body size limit for handler functions using `#[potato::limit_size(...)]`, returns 413 Payload Too Large.

### Basic Usage

```rust
// Limit body to 10MB
#[potato::http_post("/upload")]
#[potato::limit_size(10 * 1024 * 1024)]
async fn upload(req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::text("uploaded")
}
```

### Separate Header and Body Limits

```rust
// Header 512KB, Body 50MB
#[potato::http_post("/large-upload")]
#[potato::limit_size(header = 512 * 1024, body = 50 * 1024 * 1024)]
async fn large_upload(req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::text("large uploaded")
}
```

### Priority

- Handler annotation > Middleware `use_limit_size` > Global config (default 100MB)
- Annotation only applies to current handler, overrides global and middleware limits

## Transfer Rate Limit

Limit connection data transfer rate using `use_transfer_limit` middleware (unit: bits/sec).

**Support Status**:
- ✅ **HTTP/1.1**: Fully supported, implemented with token bucket algorithm on TCP streams
- ❌ **HTTP/2**: Not supported (uses h2 framework, rate limiting not implemented)
- ❌ **HTTP/3**: Not supported (uses QUIC/h3 framework, rate limiting not implemented)

```rust
server.configure(|ctx| {
    // Inbound 10 Mbps, Outbound 20 Mbps
    ctx.use_transfer_limit(10_000_000, 20_000_000);
    ctx.use_handlers();
});
```

- Inbound rate: limits request data reception speed
- Outbound rate: limits response data transmission speed
- Auto-delays transfer when rate limit exceeded
- Only effective for HTTP/1.1 connections
