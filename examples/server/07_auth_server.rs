use potato::*;

#[http_get("/issue")]
async fn issue(payload: String) -> anyhow::Result<HttpResponse> {
    let token = ServerAuth::jwt_issue(payload, std::time::Duration::from_secs(10000000)).await?;
    Ok(HttpResponse::html(token))
}

#[http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServerConfig::set_jwt_secret("AAABBBCCC").await; // optional, otherwise random str
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_dispatch();
        ctx.use_doc("/doc/");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
