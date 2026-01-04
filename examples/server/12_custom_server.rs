use potato::*;

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_custom(|req| async { Some(HttpResponse::text("hello")) });
        ctx.use_handlers();
    });
    println!("visit: http://127.0.0.1:8080/hello");
    server.serve_http().await
}
