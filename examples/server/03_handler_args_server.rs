
#[potato::http_get("/hello")]
async fn hello(req: &mut potato::HttpRequest) -> anyhow::Result<potato::HttpResponse> {
    let addr = req.get_client_addr().await?;
    Ok(potato::HttpResponse::html(format!("hello client: {addr:?}")))
}

#[potato::http_get("/hello_user")]
async fn hello_user(name: String) -> potato::HttpResponse {
    potato::HttpResponse::html(format!("hello {name}"))
}

#[potato::http_post("/upload")]
async fn upload(file1: potato::PostFile) -> potato::HttpResponse {
    potato::HttpResponse::html(format!(
        "file[{}] len: {}",
        file1.filename,
        file1.data.to_buf().len()
    ))
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
