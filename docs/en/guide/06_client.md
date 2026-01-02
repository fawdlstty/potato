# Using the Client

Specify two parameters: the request address and additional parameters. Example code:

```rust
let res = potato::get("https://www.fawdlstty.com", vec![]).await?;
println!("{}", String::from_utf8(res.body)?);
```

Additional parameters are used to specify HTTP headers. Example of modifying `User-Agent`:

```rust
let res = potato::get("https://www.fawdlstty.com", vec![Headers::User_Agent("aaa".into())]).await?;
println!("{}", String::from_utf8(res.body)?);
```

Requests can be made in session form. If the request paths are the same, the connection will be reused:

```rust
let mut sess = Session::new();
let res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
```

To initiate a WebSocket connection request, use the following form:

```rust
let mut ws = Websocket::connect("ws://127.0.0.1:8080/ws", vec![]).await?;
ws.send_ping().await?;
ws.send_text("aaa").await?;
let frame = ws.recv().await?;
```

Additionally, even in pure client mode, you can use jemalloc to get detailed memory allocation reports. You need to add the following code at the program entry point (at the beginning of the main function):

```rust
potato::init_jemalloc()?;
```

Then when needed, call the following code:

```rust
let pdf_data = crate::dump_jemalloc_profile()?;
```

At this point, the `pdf_data` variable contains the raw content of the PDF memory analysis report. Store it as a file to view it.