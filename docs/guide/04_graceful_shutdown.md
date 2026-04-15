# 优雅退出

服务对象提供 `shutdown_signal` 方法，请求即获取退出信号。触发此信号即退出服务。

话不多说看示例：

```rust
use std::sync::LazyLock;
use tokio::sync::{oneshot, Mutex};

static SHUTDOWN_SIGNAL: LazyLock<Mutex<Option<oneshot::Sender<()>>>> =
    LazyLock::new(|| Mutex::new(None));

#[potato::http_get("/shutdown")]
async fn shutdown() -> potato::HttpResponse {
    if let Some(signal) = SHUTDOWN_SIGNAL.lock().await.take() {
        _ = signal.send(());
    }
    potato::HttpResponse::html("shutdown!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    *SHUTDOWN_SIGNAL.lock().await = Some(server.shutdown_signal());
    println!("visit: http://127.0.0.1:8080/shutdown");
    server.serve_http().await
}
```