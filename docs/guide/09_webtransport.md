# WebTransport 支持

WebTransport 是基于 HTTP/3 (QUIC) 的现代 Web API，提供低延迟双向通信。

## 功能特性

- **低延迟双向通信**：比 WebSocket 更低的连接建立延迟（0-RTT）
- **多路复用流**：在单个连接上支持多个独立的双向流
- **不可靠数据报**：支持 UDP-like 的数据报传输（适合实时游戏、音视频）
- **有序/无序传输**：可根据场景选择可靠流或不可靠数据报

## 启用方式

WebTransport 功能已集成到 `http3` 特性中，无需单独启用：

```toml
[dependencies]
potato = { version = "0.3.6", features = ["http3"] }
```

## 服务器端

### 基础用法

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:4433");
    
    server.configure(|ctx| {
        // 一行代码启用 WebTransport
        ctx.use_webtransport("/wt", |session| async move {
            println!("新 WebTransport 会话: {:?}", session.remote_addr());
            
            // 处理双向流
            while let Ok(Some(stream)) = session.accept_bi().await {
                tokio::spawn(handle_stream(stream));
            }
        });
    });
    
    println!("WebTransport 服务器启动");
    // WebTransport 需要 HTTP/3 模式
    server.serve_http3("cert.pem", "key.pem").await
}

async fn handle_stream(mut stream: potato::WebTransportStream) {
    // 处理流数据
    while let Ok(data) = stream.recv().await {
        println!("收到: {:?}", String::from_utf8_lossy(&data));
        // 回显数据
        let _ = stream.send(&data).await;
    }
}
```

### 处理数据报

```rust
ctx.use_webtransport("/wt", |session| async move {
    // 处理数据报（不可靠但低延迟）
    while let Ok(datagram) = session.recv_datagram().await {
        println!("数据报: {:?}", String::from_utf8_lossy(&datagram));
    }
});
```

### 完整示例

```rust
ctx.use_webtransport("/wt", |session| async move {
    println!("新会话: {:?}", session.remote_addr());
    
    // 同时处理流和数据报
    loop {
        tokio::select! {
            // 接受新的双向流
            result = session.accept_bi() => {
                match result {
                    Ok(Some(stream)) => {
                        tokio::spawn(handle_bi_stream(stream));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("接受流失败: {}", e);
                        break;
                    }
                }
            }
            // 接收数据报
            result = session.recv_datagram() => {
                match result {
                    Ok(datagram) => {
                        println!("数据报: {:?}", datagram);
                    }
                    Err(e) => {
                        eprintln!("接收数据报失败: {}", e);
                    }
                }
            }
        }
    }
});
```

## 客户端

### 使用宏连接（推荐）

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 最简洁的连接方式
    let mut wt = potato::webtransport!("https://server.com/wt").await?;
    
    // 发送数据报
    wt.send_datagram(b"hello").await?;
    
    // 打开双向流
    let mut stream = wt.open_bi().await?;
    stream.send(b"request").await?;
    let response = stream.recv().await?;
    println!("响应: {:?}", String::from_utf8_lossy(&response));
    
    Ok(())
}
```

### 带自定义头

```rust
let mut wt = potato::webtransport!(
    "https://server.com/wt",
    "Authorization" = "Bearer token"
).await?;
```

### 传统方式

```rust
use potato::{WebTransport, Headers};

let mut wt = WebTransport::connect("https://server.com/wt", vec![
    Headers::Custom(("Authorization".to_string(), "Bearer token".to_string())),
]).await?;
```

## API 参考

### WebTransportSession (服务器端)

- `accept_bi()` - 接受新的双向流
- `accept_uni()` - 接受新的单向接收流
- `open_bi()` - 打开新的双向流
- `open_uni()` - 打开新的单向发送流
- `recv_datagram()` - 接收数据报
- `send_datagram()` - 发送数据报
- `remote_addr()` - 获取远程地址
- `close()` - 关闭会话

### WebTransportStream

- `send()` - 发送数据
- `recv()` - 接收数据
- `split()` - 拆分为发送和接收流
- `finish()` - 完成发送

### WebTransport (客户端)

- `connect()` - 连接到服务器
- `open_bi()` - 打开双向流
- `open_uni()` - 打开单向发送流
- `accept_uni()` - 接受单向接收流
- `send_datagram()` - 发送数据报
- `recv_datagram()` - 接收数据报

## 与 WebSocket 对比

| 特性 | WebSocket | WebTransport |
|------|-----------|--------------|
| 传输协议 | TCP | QUIC (UDP) |
| 多路复用 | ❌ | ✅ |
| 不可靠传输 | ❌ | ✅ |
| 0-RTT 连接 | ❌ | ✅ |
| 浏览器支持 | ✅ | ✅ (Chrome/Edge 104+) |
| 服务器复杂度 | 低 | 中 |

## 使用场景

- **实时游戏状态同步**：使用数据报传输（低延迟，允许丢包）
- **文件传输**：使用可靠流传输
- **实时音视频**：结合 WebRTC 使用
- **低延迟聊天**：替代 WebSocket

## 浏览器端使用

```javascript
const wt = new WebTransport('https://server.com/wt');

await wt.ready;
console.log('WebTransport 连接已建立');

// 发送数据报
const writer = wt.datagrams.writable.getWriter();
await writer.write(new Uint8Array([1, 2, 3, 4]));

// 接收数据报
const reader = wt.datagrams.readable.getReader();
while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    console.log('收到数据报:', value);
}

// 关闭
wt.close();
```

## 注意事项

1. **必须使用 serve_http3**：WebTransport 基于 HTTP/3 (QUIC)，不能使用 serve_http 或 serve_https
2. **需要 TLS 证书**：QUIC 强制要求 TLS，即使是本地测试也需要证书
3. **UDP 端口**：确保防火墙允许 UDP 流量
4. **浏览器兼容性**：仅支持 Chrome/Edge 104+

## 示例代码

完整示例请参考：
- [32_webtransport_server.rs](https://github.com/fawdlstty/potato/blob/master/examples/32_webtransport_server.rs)
- [33_webtransport_client.rs](https://github.com/fawdlstty/potato/blob/master/examples/33_webtransport_client.rs)
