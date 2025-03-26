use potato::*;

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    println!("visit: https://127.0.0.1:8080/hello");
    server.serve_https("cert.pem", "key.pem").await
}
