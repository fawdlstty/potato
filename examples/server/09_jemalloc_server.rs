// notice: current only support linux
// run cmd: `cargo add potato --features jemalloc`

// for ubuntu/debian
// run cmd: `sudo apt install libjemalloc-dev graphviz ghostscript`

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_jemalloc("/profile.pdf");
    });
    println!("visit: http://127.0.0.1:8080/profile.pdf");
    server.serve_http().await
}
