
#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_custom_sync(|req| {
            if req.url_path == "/sync" {
                return Some(potato::HttpResponse::text("hello from sync custom route"));
            }
            None
        });
        ctx.use_custom(|req| async move {
            if req.url_path == "/async" {
                return Some(potato::HttpResponse::text("hello from async custom route"));
            }
            None
        });
        ctx.use_handlers();
    });
    println!("visit: http://127.0.0.1:8080/sync");
    println!("visit: http://127.0.0.1:8080/async");
    server.serve_http().await
}
