use potato::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut ws = Websocket::connect("ws://127.0.0.1:8080/ws", vec![]).await?;
    ws.send_ping().await?;
    ws.send_text("aaa").await?;
    let frame = ws.recv().await?;
    println!("{frame:?}");
    Ok(())
}
