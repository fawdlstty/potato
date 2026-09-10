# SSE（流式传输）

## 概述

Potato 框架支持使用 SSE（Server-Sent Events）实现流式传输，适用于聊天消息、进度推送、实时数据等需要逐步返回内容的场景。通过 `HttpResponse::sse()` 即可快速创建流式响应。

## 基本 SSE 响应

### 基本用法

```rust
#[potato::http_get("/api/v1/chat")]
async fn sse_chat() -> anyhow::Result<potato::HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        async fn sse_chat_inner(tx: tokio::sync::mpsc::Sender<Vec<u8>>) -> anyhow::Result<()> {
            tx.send("你好！".as_bytes().to_vec()).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            tx.send("我是流式助手。".as_bytes().to_vec()).await?;
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

### 说明

1. **创建通道**: 使用 `tokio::sync::mpsc::channel::<Vec<u8>>(buffer_size)` 创建发送端和接收端，`buffer_size` 为通道缓冲区大小（如 100）。

2. **发送数据**: 在后台任务中通过 `tx.send(data)` 逐块发送字节流，每次发送的内容会作为一个分块推送给客户端。

3. **结束流式**: 后台任务结束（`tx` 被丢弃）后，流式响应自动结束。

4. **响应类型**: Handler 返回 `anyhow::Result<HttpResponse>`，`HttpResponse::sse(rx)` 会自动配置 SSE 相关的头部。

## 自定义 SSE 事件

对于需要自定义 SSE 事件格式的场景，设置适当的头部：

```rust
#[potato::http_get("/sse-custom")]
async fn sse_custom_handler() -> anyhow::Result<HttpResponse> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);

    tokio::spawn(async move {
        for i in 0..5 {
            let sse_data = format!("data: 事件 {}\n\n", i);
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

## 客户端消费 SSE

客户端使用 `stream_data()` 逐块读取流式响应：

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

完整示例：`examples/client/04_sse_client.rs`
