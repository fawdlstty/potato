use potato::*;

// run cmd: `cargo add potato --features webdav`

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_webdav_localfs("/webdav", "/tmp");
        //ctx.use_webdav_memfs("/webdav");
    });
    server.serve_http().await
}
