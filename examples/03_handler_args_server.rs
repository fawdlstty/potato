use potato::*;

#[http_get("/hello")]
async fn hello(req: HttpRequest, client: std::net::SocketAddr, wsctx: &mut WebsocketContext) -> HttpResponse {
    HttpResponse::html("hello world")
}

#[http_get("/hello_user")]
async fn hello_user(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello {name}"))
}

#[http_post("/upload")]
async fn upload(file1: PostFile) -> HttpResponse {
    HttpResponse::html(format!("file[{}] len: {}", file1.filename, file1.data.len()))
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_dispatch();
        ctx.use_doc("/doc/");
    });
    println!("visit: https://127.0.0.1:8080/doc/");
    _ = server.serve_http().await;
}
