# 流式传输

## 概述

Potato 框架支持使用 SSE（Server-Sent Events）实现流式传输，特别适用于 AI 聊天等需要逐步返回内容的场景。框架提供了对 OpenAI 和 Claude 两种主流 AI 协议的标准支持。

## OpenAI 风格流式传输

### 基本用法

```rust
#[potato::openai("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) = potato::OpenAISender::new(
        "chatcmpl-123456",           // 响应 ID
        "chat.completion.chunk",     // 对象类型
        "gpt-3.5-turbo",             // 模型名称
        "assistant",                 // 角色
    )
    .await?;
    tokio::spawn(async move {
        sender.send("你好！").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        sender.send("我是 AI 助手。").await?;
        sender.send_finish("stop").await?;
    });
    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.serve_http().await
}
```

### API 说明

**`#[potato::openai(path)]` 宏**
- 自动配置路由和 SSE 响应头
- `path`: 路由路径，如 `"/api/v1/chat"`

**`potato::OpenAISender::new()`**
- 创建 OpenAI SSE 发送器
- 参数：id, object 类型，model 名称，助手角色
- 返回：发送器实例和接收通道

**`sender.send(message)`**
- 发送内容片段
- 参数：要发送的文本内容

**`sender.send_finish(finish_reason)`**
- 发送结束标记
- 参数：结束原因（如 `"stop"`, `"length"` 等）

## Claude 风格流式传输

### 基本用法

```rust
use potato::HttpServer;

#[potato::claude("/api/v1/chat")]
async fn claude_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) = potato::ClaudeSender::new(
        "msg_claude_123456",         // 消息 ID
        "claude-3-sonnet-20240229",  // 模型名称
        "assistant",                 // 角色
    )
    .await?;
    tokio::spawn(async move {
        sender.send("你好！").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        sender.send("我是 Claude AI 助手。").await?;
        sender.send_finish().await?;
    });
    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server= potato::HttpServer::new("127.0.0.1:3000");
    server.configure(|ctx| {
        ctx.use_handlers(true);
    });
    server.serve_http().await
}
```

### API 说明

**`#[potato::claude(path)]` 宏**
- 自动配置路由和 SSE 响应头
- `path`: 路由路径，如 `"/api/v1/chat"`

**`potato::ClaudeSender::new()`**
- 创建 Claude SSE 发送器
- 参数：id, model 名称，role 角色
- 返回：发送器实例和接收通道

**`sender.send(message)`**
- 发送内容片段
- 参数：要发送的文本内容

**`sender.send_finish()`**
- 发送所有结束事件（自动按顺序发送）
- 无需参数
