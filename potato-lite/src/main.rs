#[potato_lite::http_get("/hello")]
async fn hello() -> potato_lite::HttpResponse {
    potato_lite::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = potato_lite::HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/hello");
    server.configure(|ctx| {
        ctx.use_handlers();
    });
    server.serve_http().await;
}
