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
