use potato::*;

// run cmd: `cargo add potato --features jemalloc`

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_jemalloc("/heap.pb.gz");
    });
    println!("download file: http://127.0.0.1:8080/heap.pb.gz");
    println!("after:");
    println!("    sudo apt install golang");
    println!("    go install github.com/google/pprof@latest");
    println!("    sudo ln -s ~/go/bin/pprof /usr/bin/pprof");
    println!("    pprof -http=0.0.0.0:8848 /path/of/out /d/downloads/heap.pb.gz");
    println!("visit: http://127.0.0.1:8848");
    server.serve_http().await
}
