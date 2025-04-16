#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut sess = Session::new();
    let res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
    let res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
    println!("res1: {}", String::from_utf8(res1.body)?);
    println!("res2: {}", String::from_utf8(res2.body)?);
    Ok(())
}
