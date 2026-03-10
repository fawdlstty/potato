//! OpenAI SSE 流式传输示例
//!
//! 本示例展示如何使用标准的 OpenAI Chat Completion Stream 协议
//! 实现流式 AI 响应。

use potato::HttpServer;

/// OpenAI 风格的聊天接口
/// 使用 #[potato::openai] 宏自动设置正确的路由和响应头
#[potato::openai("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    // 创建 OpenAI SSE 发送器
    // 参数：id, object 类型，model 名称，助手角色
    let (sender, rx) = potato::OpenAISender::new(
        "chatcmpl-123456",       // 响应 ID
        "chat.completion.chunk", // 对象类型（固定）
        "gpt-3.5-turbo",         // 模型名称
        "assistant",             // 角色
    )
    .await?;

    // 在后台任务中生成响应内容
    tokio::spawn(async move {
        async fn chat_inner(sender: potato::OpenAISender) -> anyhow::Result<()> {
            // 模拟思考延迟
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 流式发送内容片段
            sender.send("你好！").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("我是").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("AI ").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("助手。").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("有什么").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

            sender.send("可以帮你的吗？").await?;

            // 发送结束标记（包含 finish_reason）
            sender.send_finish("stop").await?;

            Ok(())
        }

        if let Err(e) = chat_inner(sender).await {
            eprintln!("OpenAI chat error: {e}");
        }
    });

    Ok(rx)
}

/// 自定义路径的 OpenAI 风格接口
#[potato::openai("/custom/chat")]
async fn custom_openai_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) = potato::OpenAISender::new(
        "chatcmpl-custom",
        "chat.completion.chunk",
        "gpt-4",
        "assistant",
    )
    .await?;

    tokio::spawn(async move {
        // 快速流式响应示例
        let messages = vec!["Rust ", "是", "一门", "系统", "编程", "语言。"];

        for msg in messages {
            sender.send(msg).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        sender.send_finish("stop").await?;
    });

    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("127.0.0.1:3000");
    server.configure(|ctx| {
        ctx.use_handlers(true);
    });

    println!("=== OpenAI SSE 流式传输示例 ===");
    println!("标准接口：http://127.0.0.1:3000/api/v1/chat");
    println!("自定义接口：http://127.0.0.1:3000/custom/chat");
    println!("\n使用 curl 测试:");
    println!("curl -N http://127.0.0.1:3000/api/v1/chat");

    server.serve_http().await
}
