#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        // 设置传输速率限制
        // 入站速率：10 Mbps (10,000,000 bits/sec)
        // 出站速率：20 Mbps (20,000,000 bits/sec)
        ctx.use_transfer_limit(10_000_000, 20_000_000);
        ctx.use_handlers();
    });
    println!("visit: http://127.0.0.1:8080/hello");
    println!("Rate limited: inbound 10 Mbps, outbound 20 Mbps");
    server.serve_http().await
}
