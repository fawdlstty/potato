use potato::*;

#[http_get("/get")]
async fn get() -> HttpResponse {
    HttpResponse::html("get method")
}

#[http_post("/post")]
async fn post() -> HttpResponse {
    HttpResponse::html("get method")
}

#[http_put("/put")]
async fn put() -> HttpResponse {
    HttpResponse::html("get method")
}

#[http_options("/options")]
async fn options() -> HttpResponse {
    HttpResponse::html("options")
}

#[http_head("/head")]
async fn head() -> HttpResponse {
    HttpResponse::html("head")
}

#[http_delete("/delete")]
async fn delete() -> HttpResponse {
    HttpResponse::html("delete")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers();
        ctx.use_doc("/doc/");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
