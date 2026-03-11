#[potato::http_get("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, res) = potato::OpenAISender::new(
        "chatcmpl-openai",
        "chat.completion.chunk",
        "gpt-3.5-turbo",
        "assistant",
        100,
    )
    .await?;
    tokio::spawn(async move {
        async fn openai_chat_inner(sender: potato::OpenAISender) -> anyhow::Result<()> {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("Hello,").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("World!").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("hohohoho!").await?;
            sender.send_finish("stop").await?;
            Ok(())
        }
        if let Err(e) = openai_chat_inner(sender).await {
            eprintln!("OpenAI chat error: {e}");
        }
    });
    Ok(res)
}

#[potato::http_get("/api2/v1/chat")]
async fn claude_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, rx) =
        potato::ClaudeSender::new("chatclaude", "claude-3-sonnet-20240229", "assistant", 100)
            .await?;
    tokio::spawn(async move {
        async fn claude_chat_inner(sender: potato::ClaudeSender) -> anyhow::Result<()> {
            // 发送内容片段
            sender.send("Hello,").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("World!").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("hohohoho!").await?;
            // 发送结束事件（包含 content_block_stop, message_delta, message_stop）
            sender.send_finish().await?;
            Ok(())
        }
        if let Err(e) = claude_chat_inner(sender).await {
            eprintln!("Claude chat error: {e}");
        }
    });
    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.configure(|ctx| {
        ctx.use_handlers(true);
    });
    println!("OpenAI stream on http://127.0.0.1:3000/stream");
    println!("OpenAI custom stream on http://127.0.0.1:3000/stream-custom");
    println!("Claude stream on http://127.0.0.1:3000/claude-stream");
    server.serve_http().await
}
