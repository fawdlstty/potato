# Server-Sent Events

## Overview

Potato framework supports SSE (Server-Sent Events) for streaming, suitable for scenarios like chat messages, progress updates, and real-time data that require progressive content delivery. Use `HttpResponse::sse()` to create a streaming response.

## Basic SSE Response

### Basic Usage

```rust
#[potato::http_get("/api/v1/chat")]
async fn sse_chat() -> anyhow::Result<potato::HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        async fn sse_chat_inner(tx: tokio::sync::mpsc::Sender<Vec<u8>>) -> anyhow::Result<()> {
            tx.send("Hello!".as_bytes().to_vec()).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            tx.send("I am a streaming assistant.".as_bytes().to_vec()).await?;
            Ok(())
        }
        if let Err(e) = sse_chat_inner(tx).await {
            eprintln!("SSE chat error: {e}");
        }
    });

    Ok(potato::HttpResponse::sse(rx))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("127.0.0.1:3000");
    server.serve_http().await
}
```

### Explanation

1. **Create Channel**: Use `tokio::sync::mpsc::channel::<Vec<u8>>(buffer_size)` to create a sender and receiver, where `buffer_size` is the channel buffer size (e.g., 100).

2. **Send Data**: Send byte chunks incrementally via `tx.send(data)` in a background task. Each send is pushed to the client as a chunk.

3. **Finish Streaming**: The streaming response ends automatically when the background task finishes (i.e., `tx` is dropped).

4. **Response Type**: Handler returns `anyhow::Result<HttpResponse>`, where `HttpResponse::sse(rx)` automatically configures SSE-related headers.

## Custom SSE Events

For scenarios requiring custom SSE event formats, set appropriate headers:

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

## Consuming SSE on the Client

Clients read streaming responses chunk by chunk with `stream_data()`:

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

Full example: `examples/client/04_sse_client.rs`
