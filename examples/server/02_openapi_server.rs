#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers(false);
        ctx.use_openapi("/doc/");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
