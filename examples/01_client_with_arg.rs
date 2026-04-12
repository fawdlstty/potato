#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut res = potato::get!(
        "https://www.fawdlstty.com",
        User_Agent = "aaa",
        Custom("X-API-Key") = "your-api-key"
    ).await?;
    println!("response: {}", str::from_utf8(res.body.data().await)?);
    Ok(())
}
