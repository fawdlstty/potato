/// 示例：演示 HTTP handler 返回 String 和 &'static str 类型
/// 
/// 本示例展示了如何使用 potato 框架的 HTTP handler 宏（如 #[http_get]）
/// 来创建直接返回字符串的处理函数。返回的字符串会自动通过 HttpResponse::html() 包装。

// 测试返回 String 类型
#[potato::http_get("/test-string")]
async fn handler_string() -> String {
    "<html><body><h1>Hello from String</h1></body></html>".to_string()
}

// 测试返回 &'static str 类型
#[potato::http_get("/test-static-str")]
async fn handler_static_str() -> &'static str {
    "<html><body><h1>Hello from &'static str</h1></body></html>"
}

// 测试返回 anyhow::Result<String> 类型
#[potato::http_get("/test-result-string")]
async fn handler_result_string(success: bool) -> anyhow::Result<String> {
    if success {
        Ok("<html><body><h1>Hello from Result&lt;String&gt;</h1></body></html>".to_string())
    } else {
        anyhow::bail!("Failed to generate response")
    }
}

// 测试返回 anyhow::Result<&'static str> 类型
#[potato::http_get("/test-result-static-str")]
async fn handler_result_static_str(success: bool) -> anyhow::Result<&'static str> {
    if success {
        Ok("<html><body><h1>Hello from Result&lt;&'static str&gt;</h1></body></html>")
    } else {
        anyhow::bail!("Failed to generate response")
    }
}

// 测试同步函数返回 String
#[potato::http_get("/test-string-sync")]
fn handler_string_sync() -> String {
    "<html><body><h1>Hello from sync String</h1></body></html>".to_string()
}

// 测试同步函数返回 &'static str
#[potato::http_get("/test-static-str-sync")]
fn handler_static_str_sync() -> &'static str {
    "<html><body><h1>Hello from sync &'static str</h1></body></html>"
}

#[tokio::main]
async fn main() {
    println!("Starting server with string return type tests...");
    println!("Available endpoints:");
    println!("  - http://127.0.0.1:8080/test-string");
    println!("  - http://127.0.0.1:8080/test-static-str");
    println!("  - http://127.0.0.1:8080/test-result-string?success=true");
    println!("  - http://127.0.0.1:8080/test-result-static-str?success=true");
    println!("  - http://127.0.0.1:8080/test-string-sync");
    println!("  - http://127.0.0.1:8080/test-static-str-sync");
    
    let server = potato::HttpServer::new("127.0.0.1:8080")
        .serve_http()
        .await;
    
    if let Err(e) = server {
        eprintln!("Server error: {:?}", e);
    }
}
