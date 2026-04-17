use potato::*;

/// 自定义错误处理器 - 异步版本
/// 所有handler中的异常都会通过此函数处理
#[potato::handle_error]
async fn handle_error(req: &mut HttpRequest, err: anyhow::Error) -> HttpResponse {
    // 记录错误日志
    eprintln!("[Error Handler] Caught error: {:?}", err);
    
    // 返回JSON格式的错误响应
    HttpResponse::json(serde_json::json!({
        "success": false,
        "error": format!("{}", err),
        "path": req.url_path
    }))
}

/// 测试handler - 返回错误
#[potato::http_get("/test_error")]
async fn test_error() -> anyhow::Result<HttpResponse> {
    anyhow::bail!("This is a test error from handler");
}

/// 测试handler - 正常返回
#[potato::http_get("/test_ok")]
async fn test_ok() -> HttpResponse {
    HttpResponse::json(serde_json::json!({
        "success": true,
        "message": "This request succeeded"
    }))
}

/// 测试handler - 同步函数
#[potato::http_get("/test_sync_error")]
fn test_sync_error() -> anyhow::Result<HttpResponse> {
    anyhow::bail!("Sync handler error");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Error handler example server starting on http://127.0.0.1:8080");
    
    let mut server = HttpServer::new("127.0.0.1:8080");
    server.configure(|ctx| {
        ctx.use_handlers();
    });
    
    println!("Test endpoints:");
    println!("  - http://127.0.0.1:8080/test_error (returns error)");
    println!("  - http://127.0.0.1:8080/test_ok (returns success)");
    println!("  - http://127.0.0.1:8080/test_sync_error (sync handler error)");
    
    server.serve_http().await
}
