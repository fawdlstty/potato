#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
