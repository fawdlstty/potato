use potato::{http_get, server::HttpServer, HttpResponse, HttpRequest};

#[http_get("/hello")]
async fn hello(_ctx: HttpRequest) -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    _ = server.run().await;
}
