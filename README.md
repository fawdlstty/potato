# potato

A very simple and high performance http library.

# Usage

Run command:

```sh
cargo new hello_potato --bin
cd hello_potato
cargo add potato
cargo add tokio --features full
```

Paste code:

```rust
use potato::{http_get, server::HttpServer, HttpResponse, RequestContext};

async fn prefix_handler(_ctx: &mut RequestContext) -> Option<HttpResponse> {
    match _ctx.
    Some(HttpResponse::html("hello world"))
}

#[http_get("/hello")]
async fn hello(_ctx: RequestContext) -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.run().await;
}
```

# TODO

- file
- server session
- middleware
- http client
- cookie
- chunked
- CORS
