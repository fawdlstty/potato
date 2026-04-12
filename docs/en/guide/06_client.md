# Using the Client

Client macros support passing the URL directly, with optional headers. Example:

```rust
let mut res = potato::get!("https://www.fawdlstty.com").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

Additional parameters are used to specify HTTP headers. Example for `User-Agent`:

```rust
let mut res = potato::get!("https://www.fawdlstty.com", User_Agent = "aaa").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

For methods with a request body (`post!`/`put!`), the second argument is the body:

```rust
let body = vec![];
let mut res = potato::post!("https://www.fawdlstty.com", body, User_Agent = "aaa").await?;
println!("{}", String::from_utf8(res.body.data().await.to_vec())?);
```

Other methods follow the same style: `delete!`, `head!`, `options!`, `connect!`, `trace!`, `patch!`.

Requests can be made in session form. If the request paths are the same, the connection will be reused:

```rust
let mut sess = Session::new();
let mut res1 = sess.get("https://www.fawdlstty.com/1", vec![]).await?;
let mut res2 = sess.get("https://www.fawdlstty.com/2", vec![]).await?;
println!("{}", String::from_utf8(res1.body.data().await.to_vec())?);
println!("{}", String::from_utf8(res2.body.data().await.to_vec())?);
```

For SSE responses, keep reading with `stream_data()`:

```rust
let mut res = potato::get!("http://127.0.0.1:3000/api/v1/chat").await?;
let mut stream = res.body.stream_data();
while let Some(chunk) = stream.next().await {
    print!("{}", String::from_utf8_lossy(&chunk));
}
```

To initiate a WebSocket connection request, use the following form (macro with the same parameter format as `get!()`):

```rust
let mut ws = potato::websocket!("ws://127.0.0.1:8080/ws").await?;
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

## Reverse Proxy and Transfer Sessions

You can use [TransferSession](file:///e:/GitHub_fa/potato/potato/src/client.rs#L224-L251) to handle reverse proxy and forward proxy scenarios. It supports forwarding of both HTTP and WebSocket requests, and can modify the forwarded content.

Create a reverse proxy session that forwards requests to a specified target URL:

```rust
let mut transfer_session = potato::client::TransferSession::from_reverse_proxy(
    "/api".to_string(),      // Request path prefix
    "http://backend-server:8080".to_string()  // Backend target server
);

// Use the transfer method when processing requests
// let response = transfer_session.transfer(&mut request, true /* whether to modify content */).await?;
```

Create a forward proxy session for general proxy forwarding:

```rust
let mut transfer_session = potato::client::TransferSession::from_forward_proxy();

// Use the transfer method when processing requests
// let response = transfer_session.transfer(&mut request, false /* whether to modify content */).await?;
```
