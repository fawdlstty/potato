#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_reverse_proxy("/", "https://github.com", true);
    });
    println!("visit: http://127.0.0.1:8080");
    server.serve_http().await
}
