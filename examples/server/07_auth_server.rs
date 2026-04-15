
#[potato::http_get("/issue")]
async fn issue(payload: String) -> anyhow::Result<potato::HttpResponse> {
    let token = potato::ServerAuth::jwt_issue(payload, std::time::Duration::from_secs(10000000)).await?;
    Ok(potato::HttpResponse::html(token))
}

#[potato::http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> potato::HttpResponse {
    potato::HttpResponse::html(format!("payload: [{payload}]"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    potato::ServerConfig::set_jwt_secret("AAABBBCCC").await; // optional, otherwise random str
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers(false);
        ctx.use_openapi("/doc/");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
