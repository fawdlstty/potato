# 优雅退出

服务对象提供 `shutdown_signal` 方法，请求即获取退出信号。触发此信号即退出服务。

话不多说看示例：

```rust
use potato::*;
use std::sync::LazyLock;
use tokio::sync::{oneshot, Mutex};

static SHUTDOWN_SIGNAL: LazyLock<Mutex<Option<oneshot::Sender<()>>>> =
    LazyLock::new(|| Mutex::new(None));

#[http_get("/shutdown")]
async fn shutdown() -> HttpResponse {
    if let Some(signal) = SHUTDOWN_SIGNAL.lock().await.take() {
        _ = signal.send(());
    }
    HttpResponse::html("shutdown!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    *SHUTDOWN_SIGNAL.lock().await = Some(server.shutdown_signal());
    println!("visit: http://127.0.0.1:8080/shutdown");
    server.serve_http().await
}
```