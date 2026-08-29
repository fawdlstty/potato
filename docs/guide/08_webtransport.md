# WebTransport 支持（实验性）

WebTransport API 处于实验状态，会话协议尚未完整实现。不要在生产环境依赖它，也不保证与所有 WebTransport 客户端互操作。

## 启用方式

必须显式启用实验特性，`http3` 和 `full` 都不会包含它：

```toml
[dependencies]
potato = { version = "0.3.25", features = ["webtransport"] }
```

## 服务器端

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:4433");

    server.configure(|ctx| {
        ctx.use_webtransport("/wt", |session| async move {
            println!("新 WebTransport 会话: {:?}", session.remote_addr());

            while let Ok(Some(mut stream)) = session.accept_bi().await {
                if stream.send(b"ok").await.is_ok() {
                    let _ = stream.finish().await;
                }
            }
        });
    });

    server.serve_http3("cert.pem", "key.pem").await
}
```

## 客户端

客户端连接只接受 URL，不支持自定义 header：

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wt = potato::webtransport!("https://server.com/wt").await?;
    let mut stream = wt.open_bi().await?;

    stream.send(b"request").await?;
    if let Some(response) = stream.recv().await? {
        println!("响应: {:?}", String::from_utf8_lossy(&response));
    }

    Ok(())
}
```

## 当前 API

`WebTransportSession` 提供 `accept_bi`、`accept_uni`、`open_bi`、`open_uni`、`send_datagram`、`recv_datagram`、`remote_addr` 和 `close`。

`WebTransportStream` 提供 `send`、`recv` 和 `finish`；它被丢弃时会重置底层 QUIC 流。

`WebTransport` 客户端提供 `connect`、`open_bi`、`open_uni`、`accept_uni`、`send_datagram` 和 `recv_datagram`。

## 注意事项

1. 必须使用 `serve_http3`，并且提供有效的 TLS 证书。
2. 确保防火墙放行对应的 UDP 端口。
3. 当前实现只覆盖最小 HTTP/3 CONNECT 和 QUIC 流/数据报路径，未完整实现 WebTransport capsule、session ID 和关闭语义。

## 示例代码

完整示例请参考：

- [20_webtransport_server.rs](https://github.com/fawdlstty/potato/blob/master/examples/server/20_webtransport_server.rs)
- [08_webtransport_client.rs](https://github.com/fawdlstty/potato/blob/master/examples/client/08_webtransport_client.rs)
