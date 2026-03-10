//! Claude SSE 流式传输示例
//!
//! 本示例展示如何使用标准的 Claude Messages Stream 协议
//! 实现流式 AI 响应。

use potato::HttpServer;

/// Claude 风格的聊天接口
/// 使用 #[potato::claude] 宏自动设置正确的路由和响应头
#[potato::claude("/api/v1/chat")]
async fn claude_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    // 创建 Claude SSE 发送器
    // 参数：消息 ID，模型名称，角色
    let (sender, rx) = potato::ClaudeSender::new(
        "msg_claude_123456",        // 消息 ID
        "claude-3-sonnet-20240229", // 模型名称
        "assistant",                // 角色
    )
    .await?;

    // 在后台任务中生成响应内容
    tokio::spawn(async move {
        async fn chat_inner(sender: potato::ClaudeSender) -> anyhow::Result<()> {
            // 模拟思考延迟
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 流式发送内容片段
            sender.send("你好！").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("我是").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("Claude ").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("AI ").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("助手。").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("有什么").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("可以帮你的吗？").await?;

            // 发送结束标记（自动包含 content_block_stop, message_delta, message_stop）
            sender.send_finish().await?;

            Ok(())
        }

        if let Err(e) = chat_inner(sender).await {
            eprintln!("Claude chat error: {e}");
        }
    });

    Ok(rx)
}

/// 自定义路径的 Claude 风格接口
#[potato::claude("/custom/claude")]
async fn custom_claude_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) =
        potato::ClaudeSender::new("msg_claude_custom", "claude-3-opus-20240229", "assistant")
            .await?;

    tokio::spawn(async move {
        // 快速流式响应示例
        let messages = vec!["Rust ", "是", "一门", "现代", "系统", "编程", "语言", "。"];

        for msg in messages {
            sender.send(msg).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        sender.send_finish().await?;
    });

    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("127.0.0.1:3000");
    server.configure(|ctx| {
        ctx.use_handlers(true);
    });

    println!("=== Claude SSE 流式传输示例 ===");
    println!("标准接口：http://127.0.0.1:3000/api/v1/chat");
    println!("自定义接口：http://127.0.0.1:3000/custom/claude");
    println!("\n使用 curl 测试:");
    println!("curl -N http://127.0.0.1:3000/api/v1/chat");

    server.serve_http().await
}
