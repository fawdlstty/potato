#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 创建 OpenAI 会话
    let mut agent = potato::AgentClientSession::new(
        potato::LlmProvider::OpenAI,
        "https://api.openai.com",
        Some("sk-your-api-key".to_string()),
    );

    // 获取可用模型列表
    let models = agent.list_models().await?;
    for model in &models {
        println!("Model: {} ({})", model.id, model.name);
    }

    // 设置模型（会自动验证模型是否可用）
    agent.set_model("gpt-4o-mini").await?;

    // 设置思考等级（仅部分推理模型支持）
    agent.set_thinking_mode(potato::ThinkingMode::High);

    // 设置系统提示词
    agent.set_system_prompt("You are a helpful assistant.");

    // 非流式对话
    let reply = agent.chat("Hello, who are you?").await?;
    println!("Reply: {}", reply);

    // 流式对话
    let mut stream = agent.chat_stream("Tell me a short story.").await?;
    while let Some(chunk) = stream.recv().await {
        match chunk {
            potato::StreamChunk::Content(text) => print!("{}", text),
            potato::StreamChunk::Done => break,
        }
    }
    println!();

    // 获取当前会话历史
    for msg in agent.messages() {
        println!("[{:?}] {}", msg.role, msg.content);
    }

    // 清空历史（保留 system prompt）
    agent.clear_messages();

    // 序列化会话状态（可保存到文件）
    let state = agent.serialize()?;

    // 反序列化恢复会话
    let mut restored = potato::AgentClientSession::deserialize(&state)?;
    let reply2 = restored.chat("What did we talk about?").await?;
    println!("Restored reply: {}", reply2);

    Ok(())
}
