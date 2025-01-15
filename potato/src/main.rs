use potato::*;

#[http_get("/hello")]
async fn hello(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello world, {name}!"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::JwtAuth::set_secret("AAAAAAAAAAAAAAABBBCCC").await;
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_dispatch();
        ctx.use_doc("/doc/");
        //ctx.use_embedded_route("/", embed_dir!("assets/wwwroot"));
        //ctx.use_location_route("/", "/wwwroot");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}

// cargo run -p potato
// cargo publish -p potato-macro --registry crates-io --allow-dirty
// cargo publish -p potato --registry crates-io --allow-dirty
