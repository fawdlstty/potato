# Server-Sent Events

## Overview

Potato framework supports SSE (Server-Sent Events) for streaming, particularly suitable for scenarios like AI chat that require progressive content delivery. The framework provides standard support for three mainstream AI protocols: OpenAI, Claude, and Ollama.

## OpenAI Style Streaming

### Basic Usage

```rust
#[potato::http_get("/api/v1/chat")]
async fn openai_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, res) = potato::OpenAISender::new(
        "chatcmpl-123456",           // Response ID
        "chat.completion.chunk",     // Object type
        "gpt-3.5-turbo",             // Model name
        "assistant",                 // Role
        100,                         // Buffer size
    )
    .await?;
    
    tokio::spawn(async move {
        async fn openai_chat_inner(sender: potato::OpenAISender) -> anyhow::Result<()> {
            sender.send("Hello!").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            sender.send("I am an AI assistant.").await?;
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

### Explanation

1. **Create OpenAISender**: Use `OpenAISender::new()` to create a sender and response object. Parameters include:
   - `id`: Response ID (e.g., "chatcmpl-123456")
   - `object`: Object type (typically "chat.completion.chunk")
   - `model`: Model name (e.g., "gpt-3.5-turbo")
   - `role`: Assistant role (typically "assistant")
   - `buffer_size`: Channel buffer size (e.g., 100)

2. **Send Messages**: Use `sender.send()` to send message chunks. Each call adds a content delta.

3. **Finish Streaming**: Use `sender.send_finish(finish_reason)` to end the stream with a finish reason (e.g., "stop", "length", "content_filter").

4. **Response Type**: Handler returns `anyhow::Result<HttpResponse>`, where the response is automatically configured with SSE headers.

## Claude Style Streaming

### Basic Usage

```rust
#[potato::http_get("/api/v1/chat")]
async fn claude_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, rx) =
        potato::ClaudeSender::new("msg_claude_123456", "claude-3-sonnet-20240229", "assistant", 100).await?;
    
    tokio::spawn(async move {
        async fn claude_chat_inner(sender: potato::ClaudeSender) -> anyhow::Result<()> {
            // Send content chunks
            sender.send("Hello!").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            sender.send("I am Claude AI assistant.").await?;
            // Send finish event (includes content_block_stop, message_delta, message_stop)
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

### Explanation

1. **Create ClaudeSender**: Use `ClaudeSender::new()` to create a sender and response object. Parameters include:
   - `id`: Message ID (e.g., "msg_claude_123456")
   - `model`: Model name (e.g., "claude-3-sonnet-20240229")
   - `role`: Assistant role (typically "assistant")
   - `buffer_size`: Channel buffer size (e.g., 100)

2. **Send Messages**: Use `sender.send()` to send text content blocks.

3. **Finish Streaming**: Use `sender.send_finish()` to end the stream. This automatically sends:
   - `content_block_stop`: Indicates content block completion
   - `message_delta`: Contains stop reason and usage statistics
   - `message_stop`: Indicates message completion

4. **Response Type**: Handler returns `anyhow::Result<HttpResponse>`, automatically configured with appropriate SSE headers for Claude protocol.

## Ollama Style Streaming

### Basic Usage

```rust
#[potato::http_get("/api/v1/chat")]
async fn ollama_chat() -> anyhow::Result<potato::HttpResponse> {
    let (sender, rx) = potato::OllamaSender::new("llama3", 100).await?;
    
    tokio::spawn(async move {
        async fn ollama_chat_inner(sender: potato::OllamaSender) -> anyhow::Result<()> {
            // Send content chunks
            sender.send("Hello!").await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            sender.send("I am Ollama AI assistant.").await?;
            // Send finish event
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
    server.serve_http().await
}
```

### Explanation

1. **Create OllamaSender**: Use `OllamaSender::new()` to create a sender and response object. Parameters include:
   - `model`: Model name (e.g., "llama3", "gemma", "mistral", etc.)
   - `buffer_size`: Channel buffer size (e.g., 100)

2. **Send Messages**: Use `sender.send()` to send text content blocks.

3. **Finish Streaming**: Use `sender.send_finish()` to end the stream. This automatically sends:
   - `done: true`: Indicates generation is complete
   - `done_reason`: Completion reason (e.g., "stop")

4. **Response Type**: Handler returns `anyhow::Result<HttpResponse>`, automatically configured with appropriate SSE headers for Ollama protocol.

5. **Data Format**: Ollama uses NDJSON (newline-delimited JSON) format, which differs from OpenAI/Claude's SSE format, but Potato internally uses a unified SSE channel for transmission.

## Generic SSE Transmission

For simple SSE scenarios without AI protocols, use `HttpResponse::sse()`:

```rust
#[potato::http_get("/sse")]
async fn sse_handler() -> anyhow::Result<HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        for i in 0..10 {
            let data = format!("Message {}\n", i).into_bytes();
            tx.send(data).await.ok();
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    });

    Ok(HttpResponse::sse(rx))
}
```

For custom SSE events, set appropriate headers:

```rust
#[potato::http_get("/sse-custom")]
async fn sse_custom_handler() -> anyhow::Result<HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        for i in 0..5 {
            let sse_data = format!("data: Event {}\n\n", i);
            tx.send(sse_data.into_bytes()).await.ok();
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    let mut resp = HttpResponse::sse(rx);
    resp.add_header("Content-Type", "text/event-stream");
    resp.add_header("Cache-Control", "no-cache");
    Ok(resp)
}
```
