use potato::OllamaSender;

#[tokio::test]
async fn test_ollama_sender_basic() -> anyhow::Result<()> {
    // 创建 OllamaSender
    let (sender, _rx) = OllamaSender::new("llama3", 100).await?;

    // 发送消息
    sender.send("Hello").await?;
    sender.send(" World").await?;

    // 结束流
    sender.send_finish().await?;

    Ok(())
}

#[tokio::test]
async fn test_ollama_sender_content_type() -> anyhow::Result<()> {
    // 创建 OllamaSender 并验证 Content-Type
    let (_sender, rx) = OllamaSender::new("llama3", 100).await?;

    // 验证返回的 HttpResponse 具有正确的 Content-Type
    let content_type = rx.get_header("Content-Type");
    assert!(
        content_type.is_some(),
        "Content-Type header should be present"
    );

    let content_type = content_type.unwrap();
    assert!(
        content_type.contains("application/x-ndjson"),
        "Content-Type should be application/x-ndjson, but got: {}",
        content_type
    );

    Ok(())
}

#[tokio::test]
async fn test_ollama_sender_multiple_models() -> anyhow::Result<()> {
    // 测试不同的模型名称
    let models = vec!["llama3", "gemma:2b", "mistral", "codellama"];

    for model in models {
        let (sender, _rx) = OllamaSender::new(model, 50).await?;
        sender.send("Test message").await?;
        sender.send_finish().await?;
    }

    Ok(())
}

#[tokio::test]
async fn test_ollama_sender_empty_message() -> anyhow::Result<()> {
    let (sender, _rx) = OllamaSender::new("llama3", 100).await?;

    // 发送空消息
    sender.send("").await?;
    sender.send_finish().await?;

    Ok(())
}

#[tokio::test]
async fn test_ollama_sender_unicode() -> anyhow::Result<()> {
    let (sender, _rx) = OllamaSender::new("llama3", 100).await?;

    // 发送 Unicode 字符
    sender.send("你好").await?;
    sender.send("🚀").await?;
    sender.send("Hello 世界").await?;
    sender.send_finish().await?;

    Ok(())
}
