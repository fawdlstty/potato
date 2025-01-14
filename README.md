# potato

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Fpotato%2Fmain%2F/potato/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/potato/rust.yml)

高性能、简洁语法的HTTP框架。

# 用法

加入库的引用：

```sh
cargo add potato
cargo add tokio --features full
```

### Hello World

```rust
use potato::*;

// http://127.0.0.1:8080/hello
#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.serve_http().await;
}
```

更多示例请参考：<https://github.com/fawdlstty/potato/tree/main/examples>

<!--
# TODO

- static path security
- file for download
- http client
- cookie
- chunked
- CORS
-->
