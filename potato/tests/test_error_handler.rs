use potato::*;

/// 测试自定义错误处理器（异步版本）
mod test_async_error_handler {
    use super::*;

    #[potato::handle_error]
    async fn custom_error_handler(req: &mut HttpRequest, err: anyhow::Error) -> HttpResponse {
        let path = req.url_path.clone();
        HttpResponse::text(format!("Custom error handler: {} at {}", err, path))
    }

    #[potato::http_get("/test_async_handler_error")]
    async fn handler_with_error() -> anyhow::Result<HttpResponse> {
        anyhow::bail!("Async handler error");
    }
}

/// 测试默认错误处理器（无自定义handler）
mod test_default_error_handler {
    use super::*;

    #[potato::http_get("/test_default_error")]
    async fn handler_with_default_error() -> anyhow::Result<HttpResponse> {
        anyhow::bail!("Default handler error");
    }
}

#[tokio::test]
async fn test_error_handler_compilation() {
    // 这个测试主要验证代码能够编译通过
    // 由于inventory的注册机制，实际运行时需要单独的二进制

    println!("Error handler macros compiled successfully");

    // 验证HttpResponse::error仍然存在
    let resp = HttpResponse::error("test error");
    assert_eq!(resp.http_code, 500);

    println!("Default error response works correctly");
}

#[tokio::test]
async fn test_error_handler_basic() {
    // 测试基本的错误响应格式
    let resp = HttpResponse::error("test error message");
    assert_eq!(resp.http_code, 500);

    // 验证文本错误响应
    let text_resp = HttpResponse::text("error test");
    assert_eq!(text_resp.http_code, 200);

    println!("Error response format test passed");
}
