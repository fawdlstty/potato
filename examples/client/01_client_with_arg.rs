#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let r = potato::get("https://www.fawdlstty.com", vec![Headers::User_Agent("aaa".into())]).await?;
    println!("{}", String::from_utf8(res.body)?);
    Ok(())
}
