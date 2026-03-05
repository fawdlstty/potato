# 流式传输支持

potato 库现在支持 HTTP 流式传输 (Streaming),允许服务器向客户端连续发送数据块。

## 主要特性

### 1. `HttpResponseBody` 枚举

```rust
#[derive(Debug)]
pub enum HttpResponseBody {
    Data(Vec<u8>),           // 传统的一次性响应体
    Stream(Receiver<Vec<u8>>), // 新的流式响应体
}
```

### 2. 创建流式响应的 API

#### `HttpResponse::stream(rx: Receiver<Vec<u8>>)`
创建一个使用 chunked transfer encoding 的流式响应。

#### `HttpResponse::stream_with_content_type(rx: Receiver<Vec<u8>>, content_type: impl Into<String>)`
创建一个带有自定义 Content-Type 的流式响应。

### 3. `write_to_stream` 方法

```rust
pub async fn write_to_stream(
    &self,
    stream: &mut HttpStream,
    cmode: CompressMode,
) -> anyhow::Result<()>
```

这个方法会处理 `Data` 和 `Stream` 两种类型的响应体，自动使用 chunked transfer encoding 发送流式数据。

## 使用示例

### 方式一：手动创建流式响应 (使用 `use_custom`)

```rust
use potato::{HttpServer, HttpResponse};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

let mut server = HttpServer::new("127.0.0.1:3000");

server.configure(|ctx| {
    ctx.use_custom(|req| {
        Box::pin(async move {
            if req.url_path == "/stream" {
                let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
                
                // 启动任务生成数据
                tokio::spawn(async move {
                    let mut interval = interval(Duration::from_millis(500));
                    for i in 0..10 {
                        interval.tick().await;
                        let data = format!("Message {}\n", i).into_bytes();
                        if tx.send(data).await.is_err() {
                            break;
                        }
                    }
                });
                
                Ok(Some(HttpResponse::stream(rx)))
            } else {
                Ok(None)
            }
        })
    });
});

server.serve_http().await?;
```

### 方式二：使用宏自动转换 (推荐)

现在 `#[http_get]` 等宏支持直接返回 `Receiver<Vec<u8>>` 或 `Result<Receiver<Vec<u8>>>`:

```rust
use potato::{http_get, HttpServer};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

/// 流式传输示例
#[http_get("/stream")]
async fn stream_handler() -> tokio::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
    
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(500));
        for i in 0..10 {
            interval.tick().await;
            let data = format!("Stream message {}\n", i).into_bytes();
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });
    
    rx  // 直接返回 Receiver，宏会自动转换为 HttpResponse::stream(rx)
}

/// 返回 Result 类型
#[http_get("/stream-result")]
async fn stream_result_handler() -> anyhow::Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
    
    tokio::spawn(async move {
        // ... 生成数据
    });
    
    Ok(rx)  // 返回 Ok(rx)，宏会自动处理
}

let mut server = HttpServer::new("127.0.0.1:3000");
server.configure(|ctx| {
    ctx.use_handlers(true);
});
server.serve_http().await?;
```

### Server-Sent Events (SSE)

```rust
if req.url_path == "/sse" {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
    
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(1000));
        for i in 0..5 {
            interval.tick().await;
            // SSE 格式：data: <message>\n\n
            let sse_data = format!("data: Event {}\n\n", i);
            if tx.send(sse_data.into_bytes()).await.is_err() {
                break;
            }
        }
    });
    
    let mut resp = HttpResponse::stream(rx);
    resp.add_header("Content-Type", "text/event-stream");
    resp.add_header("Cache-Control", "no-cache");
    Ok(Some(resp))
}
```

## 技术细节

### Chunked Transfer Encoding

流式响应使用 HTTP/1.1 的 chunked transfer encoding:

1. 首先发送响应头 (包含 `Transfer-Encoding: chunked`)
2. 对于每个数据块:
   - 发送块长度 (十六进制) + `\r\n`
   - 发送块数据
   - 发送 `\r\n`
3. 发送结束块 `0\r\n\r\n`

### 注意事项

- **不要克隆流式响应**: `HttpResponseBody::Stream` 包含一个 `Receiver`,不能安全克隆
- **及时发送数据**: 确保通过 channel 持续发送数据，否则客户端可能会等待
- **错误处理**: 如果 channel 被关闭 (接收端断开),发送任务应该退出

## 完整示例

查看 `examples/server/15_streaming_server.rs` 获取完整的可运行示例。

## 兼容性

- 向后兼容: 所有现有的 `Data` 类型响应仍然正常工作
- 自动检测: 服务器会自动检测响应类型并使用正确的发送方式
- 压缩支持: `Data` 类型响应仍然支持 gzip 压缩
