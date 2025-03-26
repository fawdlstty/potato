#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let r = potato::get("https://www.fawdlstty.com").await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
