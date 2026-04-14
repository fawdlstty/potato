# Handler Function Annotations

Function annotations are dedicated to server-side handler functions, used to specify the HTTP method and path for the handler function. Currently six methods are supported:

- `http_get`: GET method
- `http_post`: POST method
- `http_put`: PUT method
- `http_delete`: DELETE method
- `http_head`: HEAD method
- `http_options`: OPTIONS method

These six methods are identical in functionality except for describing the processing method. Different HTTP methods can be achieved by modifying the annotation name.

There are two ways to use annotations. One is to directly pass the request path. Example:

```rust
#[potato::http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

Another usage is to specify the request path and authentication parameter. Example:

```rust
#[potato::http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

// Note: Authentication parameter is issued in the following way
let token = ServerAuth::jwt_issue("payload".to_string(), std::time::Duration::from_secs(10000000)).await?;

// Note: The authentication token can be modified in the following form. If not specified, it will be randomly generated each time by default (usually modified once at the main function entry)
ServerConfig::set_jwt_secret("AAABBBCCC").await;
```

When a function annotation includes an authentication parameter, if authentication fails, a 401 status code will be returned and the handler function will not be called.

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

## Preprocess and Postprocess

You can stack `preprocess` and `postprocess` annotations on a handler to run fixed-signature hooks before and after the handler.

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

- `preprocess` hooks run in declaration order; if one returns an `HttpResponse`, the handler is skipped but `postprocess` hooks still run.
- `postprocess` hooks run in declaration order and can mutate the final `HttpResponse` in place.
- `preprocess`/`postprocess` can be split across multiple annotation lines; execution order is left-to-right and top-to-bottom.
