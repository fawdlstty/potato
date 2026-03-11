# 流式传输

## 概述

Potato 框架支持使用 SSE（Server-Sent Events）实现流式传输，特别适用于 AI 聊天等需要逐步返回内容的场景。框架提供了对 OpenAI 和 Claude 两种主流 AI 协议的标准支持。

## OpenAI 风格流式传输

### 基本用法

```rust
#[potato::http_get("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, res) = potato::OpenAISender::new(
        "chatcmpl-123456",           // 响应 ID
        "chat.completion.chunk",     // 对象类型
        "gpt-3.5-turbo",             // 模型名称
        "assistant",                 // 角色
        100,                         // 缓冲区大小
    )
    .await?;
    
    tokio::spawn(async move {
        async fn openai_chat_inner(sender: potato::OpenAISender) -> anyhow::Result<()> {
            sender.send("你好！").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            sender.send("我是 AI 助手。").await?;
            sender.send_finish("stop").await?;
            Ok(())
        }
        if let Err(e) = openai_chat_inner(sender).await {
            eprintln!("OpenAI chat error: {e}");
        }
    });
    
    Ok(res)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.serve_http().await
}
```

### 说明

1. **创建 OpenAISender**: 使用 `OpenAISender::new()` 创建发送器和响应对象，参数包括：
   - `id`: 响应 ID（如 "chatcmpl-123456"）
   - `object`: 对象类型（通常为 "chat.completion.chunk"）
   - `model`: 模型名称（如 "gpt-3.5-turbo"）
   - `role`: 助手角色（通常为 "assistant"）
   - `buffer_size`: 通道缓冲区大小（如 100）

2. **发送消息**: 使用 `sender.send()` 发送消息片段，每次调用会添加一个内容增量。

3. **结束流式**: 使用 `sender.send_finish(finish_reason)` 结束流，需要提供结束原因（如 "stop"、"length"、"content_filter"）。

4. **响应类型**: Handler 返回 `anyhow::Result<HttpResponse>`，响应会自动配置 SSE 相关的头部。

## Claude 风格流式传输

### 基本用法

```rust
#[potato::http_get("/api/v1/chat")]
async fn claude_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, rx) =
        potato::ClaudeSender::new("msg_claude_123456", "claude-3-sonnet-20240229", "assistant", 100).await?;
    
    tokio::spawn(async move {
        async fn claude_chat_inner(sender: potato::ClaudeSender) -> anyhow::Result<()> {
            // 发送内容片段
            sender.send("你好！").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            sender.send("我是 Claude AI 助手。").await?;
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
    server.serve_http().await
}
```

### 说明

1. **创建 ClaudeSender**: 使用 `ClaudeSender::new()` 创建发送器和响应对象，参数包括：
   - `id`: 消息 ID（如 "msg_claude_123456"）
   - `model`: 模型名称（如 "claude-3-sonnet-20240229"）
   - `role`: 助手角色（通常为 "assistant"）
   - `buffer_size`: 通道缓冲区大小（如 100）

2. **发送消息**: 使用 `sender.send()` 发送文本内容块。

3. **结束流式**: 使用 `sender.send_finish()` 结束流，此方法会自动发送：
   - `content_block_stop`: 表示内容块完成
   - `message_delta`: 包含停止原因和使用统计
   - `message_stop`: 表示消息完成

4. **响应类型**: Handler 返回 `anyhow::Result<HttpResponse>`，自动为 Claude 协议配置适当的 SSE 头部。

## 通用流式传输

对于不需要 AI 协议的简单流式场景，使用 `HttpResponse::stream()`：

```rust
#[potato::http_get("/stream")]
async fn stream_handler() -> anyhow::Result<HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        for i in 0..10 {
            let data = format!("消息 {}\n", i).into_bytes();
            tx.send(data).await.ok();
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    });

    Ok(HttpResponse::stream(rx))
}
```

对于 SSE（Server-Sent Events），设置适当的头部：

```rust
#[potato::http_get("/sse")]
async fn sse_handler() -> anyhow::Result<HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        for i in 0..5 {
            let sse_data = format!("data: 事件 {}\n\n", i);
            tx.send(sse_data.into_bytes()).await.ok();
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    let mut resp = HttpResponse::stream(rx);
    resp.add_header("Content-Type", "text/event-stream");
    resp.add_header("Cache-Control", "no-cache");
    Ok(resp)
}
```
