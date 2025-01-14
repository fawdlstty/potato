use potato::*;

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/hello");
    _ = server.serve_http().await;
}
