/// Ollama 流式传输示例
/// 
/// 本示例展示如何使用 Potato 框架实现兼容 Ollama API 的流式传输服务。
/// Ollama 使用 NDJSON (newline-delimited JSON) 格式进行流式传输。

#[potato::http_get("/api/v1/chat")]
async fn ollama_chat() -> anyhow::Result<potato::HttpResponse> {
    // 创建 OllamaSender，指定模型名称和缓冲区大小
    let (sender, rx) = potato::OllamaSender::new("llama3", 100).await?;
    
    // 在后台任务中发送流式数据
    tokio::spawn(async move {
        async fn ollama_chat_inner(sender: potato::OllamaSender) -> anyhow::Result<()> {
            // 模拟 AI 逐步生成回复
            sender.send("你好！").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            
            sender.send("我是 Ollama AI 助手。").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            
            sender.send("很高兴为你服务！").await?;
            
            // 发送结束事件
            sender.send_finish().await?;
            Ok(())
        }
        
        if let Err(e) = ollama_chat_inner(sender).await {
            eprintln!("Ollama chat error: {e}");
        }
    });
    
    Ok(rx)
}

#[potato::http_get("/api/v1/generate")]
async fn ollama_generate() -> anyhow::Result<potato::HttpResponse> {
    // 使用不同的模型
    let (sender, rx) = potato::OllamaSender::new("gemma:2b", 100).await?;
    
    tokio::spawn(async move {
        async fn ollama_generate_inner(sender: potato::OllamaSender) -> anyhow::Result<()> {
            // 模拟文本生成
            let words = vec!["在", "一个", "遥远", "的", "星系", "中", "，", "存在", "着", "无数", "的", "奥秘", "。"];
            
            for word in words {
                sender.send(word).await?;
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }
            
            sender.send_finish().await?;
            Ok(())
        }
        
        if let Err(e) = ollama_generate_inner(sender).await {
            eprintln!("Ollama generate error: {e}");
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
    
    println!("Ollama Chat API on http://127.0.0.1:3000/api/v1/chat");
    println!("Ollama Generate API on http://127.0.0.1:3000/api/v1/generate");
    println!();
    println!("测试示例:");
    println!("curl http://127.0.0.1:3000/api/v1/chat");
    println!("curl http://127.0.0.1:3000/api/v1/generate");
    
    server.serve_http().await
}
