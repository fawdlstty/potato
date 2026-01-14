# potato

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Fpotato%2Fmain%2F/potato/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/potato/rust.yml)

English | [简体中文](README.zh.md)

High-performance, concise syntax HTTP framework.

# Usage

[Online Documentation](https://potato.fawdlstty.com)

Add the library reference:

```sh
cargo add potato
cargo add tokio --features full
```

### Hello Server

```rust
// http://127.0.0.1:8080/hello
#[http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    _ = server.serve_http().await;
}
```

### Hello Client

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
```

### More Examples

Please refer to: <https://github.com/fawdlstty/potato/tree/main/examples>

<!--
# TODO

- cookie
- CORS
-->