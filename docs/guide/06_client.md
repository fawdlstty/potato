# 使用客户端

指定两个参数，请求地址与附加参数。示例代码：

```rust
let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
println!("{}", String::from_utf8(res.body)?);
```

附加参数用于指定HTTP头。示例修改 `User-Agent`：

```rust
let res = potato::get("https://www.fawdlstty.com", vec![Headers::User_Agent("aaa".into())]).await?;
println!("{}", String::from_utf8(res.body)?);
```

可通过会话形式发起请求，如果请求路径相同，则复用链接：

```rust
let mut sess = Session::new();
let res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
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
