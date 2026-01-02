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
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}
```

Another usage is to specify the request path and authentication parameter. Example:

```rust
#[http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

// Note: Authentication parameter is issued in the following way
let token = ServerAuth::jwt_issue("payload".to_string(), std::time::Duration::from_secs(10000000)).await?;

// Note: The authentication token can be modified in the following form. If not specified, it will be randomly generated each time by default (usually modified once at the main function entry)
ServerConfig::set_jwt_secret("AAABBBCCC").await;
```

When a function annotation includes an authentication parameter, if authentication fails, a 401 status code will be returned and the handler function will not be called.