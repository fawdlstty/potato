#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Testing SSE endpoint...");

    let mut resp = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
    println!("Status: {}", resp.http_code);
    println!("Headers: {:?}", resp.headers);

    let mut stream = resp.body.stream_data();
    while let Some(chunk) = stream.next().await {
        let text = String::from_utf8_lossy(&chunk);
        print!("{text}");
    }

    Ok(())
}
