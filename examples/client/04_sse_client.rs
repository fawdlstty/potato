use potato::client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Testing SSE endpoint...");
    
    // 测试 SSE 传输
    let resp = client::get("http://127.0.0.1:3000/sse").await?;
    println!("Status: {}", resp.http_code);
    println!("Headers: {:?}", resp.headers);
    
    Ok(())
}
