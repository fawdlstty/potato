use potato::client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Testing streaming endpoint...");
    
    // 测试普通流式传输
    let resp = client::get("http://127.0.0.1:3000/stream").await?;
    println!("Status: {}", resp.http_code);
    println!("Headers: {:?}", resp.headers);
    
    // 测试 SSE
    let resp = client::get("http://127.0.0.1:3000/sse").await?;
    println!("\nSSE Status: {}", resp.http_code);
    println!("SSE Headers: {:?}", resp.headers);
    
    Ok(())
}
