
#[potato::http_get("/get")]
async fn get() -> potato::HttpResponse {
    potato::HttpResponse::html("get method")
}

#[potato::http_post("/post")]
async fn post() -> potato::HttpResponse {
    potato::HttpResponse::html("get method")
}

#[potato::http_put("/put")]
async fn put() -> potato::HttpResponse {
    potato::HttpResponse::html("get method")
}

#[potato::http_options("/options")]
async fn options() -> potato::HttpResponse {
    potato::HttpResponse::html("options")
}

#[potato::http_head("/head")]
async fn head() -> potato::HttpResponse {
    potato::HttpResponse::html("head")
}

#[potato::http_delete("/delete")]
async fn delete() -> potato::HttpResponse {
    potato::HttpResponse::html("delete")
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
