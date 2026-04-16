/// 示例：演示如何使用 #[potato::max_concurrency(N)] 标注限制HTTP handler的并发请求数
///
/// max_concurrency 标注会自动为handler创建一个信号量，限制同时处理的请求数量。
/// 当并发请求数达到上限时，新的请求会等待直到有请求完成。
use potato::{HttpRequest, HttpResponse, HttpServer};
use std::time::Duration;
use tokio::time::sleep;

// 示例1：限制最多3个并发请求
#[potato::http_get("/limited-3")]
#[potato::max_concurrency(3)]
async fn limited_3_handler() -> String {
    // 模拟耗时操作
    sleep(Duration::from_secs(1)).await;
    "<h1>Max 3 concurrent requests</h1>".to_string()
}

// 示例2：限制最多10个并发请求
#[potato::http_post("/limited-10")]
#[potato::max_concurrency(10)]
async fn limited_10_handler(name: String) -> String {
    format!("<h1>Hello {name}, max 10 concurrent</h1>")
}

// 示例3：与其他标注一起使用 - CORS + max_concurrency
#[potato::http_put("/api/update")]
#[potato::cors(origin = "https://example.com")]
#[potato::max_concurrency(5)]
async fn update_handler(data: String) -> String {
    format!("Updated: {data}")
}

// 示例4：与preprocess/postprocess一起使用
#[potato::preprocess]
async fn log_request(req: &mut HttpRequest) -> anyhow::Result<Option<HttpResponse>> {
    println!("Request received: {:?}", req.url_path);
    Ok(None)
}

#[potato::http_get("/api/process")]
#[potato::preprocess(log_request)]
#[potato::max_concurrency(2)]
async fn process_handler() -> HttpResponse {
    sleep(Duration::from_millis(500)).await;
    HttpResponse::text("Processing complete")
}

// 示例5：同步handler也支持max_concurrency
#[potato::http_get("/sync-limited")]
#[potato::max_concurrency(4)]
fn sync_limited_handler() -> &'static str {
    "Synchronous handler with concurrency limit"
}

// 示例6：使用Result返回类型
#[potato::http_delete("/api/resource")]
#[potato::max_concurrency(1)]
async fn delete_resource(id: String) -> anyhow::Result<String> {
    // 限制为1，确保删除操作串行执行
    if id.is_empty() {
        anyhow::bail!("ID cannot be empty");
    }
    Ok(format!("Resource {id} deleted"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Max Concurrency Examples");
    println!("=========================");
    println!();
    println!("Available endpoints:");
    println!("  GET  /limited-3      - Max 3 concurrent requests (async)");
    println!("  POST /limited-10     - Max 10 concurrent requests with param");
    println!("  PUT  /api/update     - CORS + Max 5 concurrent");
    println!("  GET  /api/process    - Preprocess hook + Max 2 concurrent");
    println!("  GET  /sync-limited   - Max 4 concurrent (sync handler)");
    println!("  DELETE /api/resource - Max 1 concurrent (serial execution)");
    println!();
    println!("All handlers use semaphore-based concurrency limiting.");
    println!("Requests exceeding the limit will wait automatically.");
    println!();

    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!();

    server.serve_http().await?;
    Ok(())
}
