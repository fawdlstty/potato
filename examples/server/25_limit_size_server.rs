/// 请求体大小限制示例
/// 演示如何使用 use_limit_size 中间件和 limit_size 注解

// 全局限制: 1MB header, 10MB body
#[potato::http_post("/upload")]
async fn upload_file(req: &mut potato::HttpRequest) -> potato::HttpResponse {
    let body_size = req.body.len();
    potato::HttpResponse::text(format!("upload success, body size: {} bytes", body_size))
}

// 使用注解覆盖全局限制: 允许 100MB body
#[potato::http_post("/large-upload")]
#[potato::limit_size(100 * 1024 * 1024)]
async fn large_upload(req: &mut potato::HttpRequest) -> potato::HttpResponse {
    let body_size = req.body.len();
    potato::HttpResponse::text(format!(
        "large upload success, body size: {} bytes",
        body_size
    ))
}

// 分别限制 header 和 body
#[potato::http_post("/medium-upload")]
#[potato::limit_size(header = 512 * 1024, body = 50 * 1024 * 1024)]
async fn medium_upload(req: &mut potato::HttpRequest) -> potato::HttpResponse {
    let body_size = req.body.len();
    potato::HttpResponse::text(format!(
        "medium upload success, body size: {} bytes",
        body_size
    ))
}

// 小文件上传: 限制 1MB
#[potato::http_post("/small-upload")]
#[potato::limit_size(1024 * 1024)]
async fn small_upload(req: &mut potato::HttpRequest) -> potato::HttpResponse {
    let body_size = req.body.len();
    potato::HttpResponse::text(format!(
        "small upload success, body size: {} bytes",
        body_size
    ))
}

// 获取状态的 handler (不需要 body 限制)
#[potato::http_get("/status")]
async fn status() -> potato::HttpResponse {
    potato::HttpResponse::json(r#"{"status": "ok", "limit": "configured"}"#)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");

    server.configure(|ctx| {
        // 设置全局限制: Header 1MB, Body 10MB
        ctx.use_limit_size(1024 * 1024, 10 * 1024 * 1024);

        // 注册 handlers
        ctx.use_handlers();
    });

    println!("服务器启动在 http://0.0.0.0:8080");
    println!();
    println!("测试端点:");
    println!("  POST /upload         - 使用全局限制 (10MB)");
    println!("  POST /large-upload   - 注解覆盖 (100MB)");
    println!("  POST /medium-upload  - 注解覆盖 (50MB)");
    println!("  POST /small-upload   - 注解覆盖 (1MB)");
    println!("  GET  /status         - 查看状态");
    println!();
    println!("示例请求:");
    println!("  curl -X POST http://127.0.0.1:8080/upload -d 'small data'");
    println!("  curl -X POST http://127.0.0.1:8080/large-upload -d @large_file.bin");

    server.serve_http().await
}
