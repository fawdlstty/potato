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
            sender.send("Hello,").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("World!").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("hohohoho!").await?;
            sender.send_finish().await?;
            Ok(())
        }
        if let Err(e) = claude_chat_inner(sender).await {
            eprintln!("Claude chat error: {e}");
        }
    });
    Ok(rx)
}

#[potato::http_get("/api3/v1/chat")]
async fn ollama_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, rx) = potato::OllamaSender::new("llama3", 100).await?;
    tokio::spawn(async move {
        async fn ollama_chat_inner(sender: potato::OllamaSender) -> anyhow::Result<()> {
            sender.send("Hello,").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("World!").await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            sender.send("hohohoho!").await?;
            sender.send_finish().await?;
            Ok(())
        }
        if let Err(e) = ollama_chat_inner(sender).await {
            eprintln!("Ollama chat error: {e}");
        }
    });
    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.configure(|ctx| {
        ctx.use_handlers();
    });
    println!("OpenAI SSE on http://127.0.0.1:3000/api/v1/chat");
    println!("Claude SSE on http://127.0.0.1:3000/api2/v1/chat");
    println!("Ollama SSE on http://127.0.0.1:3000/api3/v1/chat");
    server.serve_http().await
}
