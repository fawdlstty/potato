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
