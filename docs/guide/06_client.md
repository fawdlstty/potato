# 使用客户端

客户端宏支持直接传 URL，并可选追加请求头。示例代码：

```rust
let mut res = potato::get!("https://www.fawdlstty.com").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

附加参数用于指定 HTTP 头。示例修改 `User-Agent`：

```rust
let mut res = potato::get!("https://www.fawdlstty.com", User_Agent = "aaa").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

带请求体的方法（`post!`/`put!`）第二个参数是 body：

```rust
let body = vec![];
let mut res = potato::post!("https://www.fawdlstty.com", body, User_Agent = "aaa").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

其余方法同样支持该写法：`delete!`、`head!`、`options!`、`connect!`、`trace!`、`patch!`。

可通过会话形式发起请求，如果请求路径相同，则复用链接：

```rust
let mut sess = Session::new();
let mut res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let mut res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
println!("{}", String::from_utf8(res1.body.data().await.to_vec())?);
println!("{}", String::from_utf8(res2.body.data().await.to_vec())?);
```

SSE流式响应可通过 `stream_data()` 持续接收：

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

发起Websocket连接请求通过如下形式：

```rust
let mut ws = Websocket::connect("ws://127.0.0.1:8080/ws", vec![]).await?;
ws.send_ping().await?;
ws.send_text("aaa").await?;
let frame = ws.recv().await?;
```

另外。即使是纯客户端模式，也可以使用jemalloc获取详细内存分配报告。需要在程序入口点（main函数开始位置）加入如下代码：

```rust
potato::init_jemalloc()?;
```

然后在需要时，调用如下代码：

```rust
let pdf_data = crate::dump_jemalloc_profile()?;
```

此时`pdf_data`变量里就存了pdf内存分析报告原始内容，将其存储为文件即可查看。

## 反向代理与转发会话

可以使用 [TransferSession](file:///e:/GitHub_fa/potato/potato/src/client.rs#L224-L251) 来处理反向代理和正向代理场景。它支持HTTP和WebSocket请求的转发，并且可以修改转发的内容。

创建一个反向代理会话，将请求转发到指定的目标URL：

```rust
let mut transfer_session = potato::client::TransferSession::from_reverse_proxy(
    "/api".to_string(),      // 请求路径前缀
    "http://backend-server:8080".to_string()  // 后端目标服务器
);

// 在处理请求时使用transfer方法
// let response = transfer_session.transfer(&mut request, true /* 是否修改内容 */).await?;
```

创建一个正向代理会话，用于通用代理转发：

```rust
let mut transfer_session = potato::client::TransferSession::from_forward_proxy();

// 在处理请求时使用transfer方法
// let response = transfer_session.transfer(&mut request, false /* 是否修改内容 */).await?;
```
