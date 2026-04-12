#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut res = potato::get!("https://www.fawdlstty.com", User_Agent = "aaa").await?;
    println!("{}", str::from_utf8(res.body.data().await)?);
    Ok(())
}
