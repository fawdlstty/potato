# Streaming

## Overview

Potato framework supports SSE (Server-Sent Events) for streaming, particularly suitable for scenarios like AI chat that require progressive content delivery. The framework provides standard support for two mainstream AI protocols: OpenAI and Claude.

## OpenAI Style Streaming

### Basic Usage

```rust
#[potato::openai("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) = potato::OpenAISender::new(
        "chatcmpl-123456",           // Response ID
        "chat.completion.chunk",     // Object type
        "gpt-3.5-turbo",             // Model name
        "assistant",                 // Role
    )
    .await?;
    tokio::spawn(async move {
        sender.send("Hello!").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        sender.send("I am an AI assistant.").await?;
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

### API Reference

**`#[potato::openai(path)]` Macro**
- Automatically configures routing and SSE response headers
- `path`: Route path, such as `"/api/v1/chat"`

**`potato::OpenAISender::new()`**
- Creates OpenAI SSE sender
- Parameters: id, object type, model name, assistant role
- Returns: Sender instance and receiver channel

**`sender.send(message)`**
- Sends a content chunk
- Parameter: Text content to send

**`sender.send_finish(finish_reason)`**
- Sends completion marker
- Parameter: Finish reason (e.g., `"stop"`, `"length"`)

## Claude Style Streaming

### Basic Usage

```rust
#[potato::claude("/api/v1/chat")]
async fn claude_chat() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (sender, rx) = potato::ClaudeSender::new(
        "msg_claude_123456",         // Message ID
        "claude-3-sonnet-20240229",  // Model name
        "assistant",                 // Role
    )
    .await?;
    tokio::spawn(async move {
        sender.send("Hello!").await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        sender.send("I am Claude AI assistant.").await?;
        sender.send_finish().await?;
    });
    Ok(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.serve_http().await
}
```

### API Reference

**`#[potato::claude(path)]` Macro**
- Automatically configures routing and SSE response headers
- `path`: Route path, such as `"/api/v1/chat"`

**`potato::ClaudeSender::new()`**
- Creates Claude SSE sender
- Parameters: id, model name, role
- Returns: Sender instance and receiver channel

**`sender.send(message)`**
- Sends a content chunk
- Parameter: Text content to send

**`sender.send_finish()`**
- Sends all completion events (automatically in order)
- No parameters required
