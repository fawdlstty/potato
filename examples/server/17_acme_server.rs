//! ACME自动证书示例
//!
//! 此示例展示如何使用potato的ACME功能自动获取和续期Let's Encrypt证书
//!
//! 使用方法：
//! 1. 确保域名已指向服务器IP
//! 2. 确保443端口可公开访问
//! 3. 运行：cargo run --example 17_acme_server --features acme

#[potato::http_get("/hello")]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("<h1>Hello from ACME TLS!</h1><p>This certificate was automatically provisioned by Let's Encrypt</p>")
}

#[potato::http_get("/")]
async fn index() -> potato::HttpResponse {
    potato::HttpResponse::html(
        r#"
        <h1>Potato ACME Server</h1>
        <p>This server uses automatic TLS certificates from Let's Encrypt</p>
        <ul>
            <li><a href="/hello">Say Hello</a></li>
        </ul>
    "#,
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 最简用法：仅需域名和邮箱
    // 注意：请将 example.com 替换为你的实际域名
    let mut server = potato::HttpServer::new("0.0.0.0:443");

    println!("Starting ACME server on port 443");
    println!("Certificate will be automatically provisioned for your domain");
    println!();
    println!("Visit: https://your-domain.com/hello");
    println!();
    println!("Note: First startup may take a moment while the certificate is being issued");

    // 启动ACME服务（自动申请证书、自动续期）
    server
        .serve_acme("your-domain.com", "your-email@example.com")
        .await
}
