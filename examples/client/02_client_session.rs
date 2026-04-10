#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut sess = potato::Session::new();
    let mut res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
    let mut res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
    println!(
        "res1: {}",
        String::from_utf8(res1.body.data().await.to_vec())?
    );
    println!(
        "res2: {}",
        String::from_utf8(res2.body.data().await.to_vec())?
    );
    Ok(())
}
