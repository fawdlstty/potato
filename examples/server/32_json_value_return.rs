/// 示例：HTTP handler 返回 serde_json::Value 和 anyhow::Result<serde_json::Value>

use potato::HttpServer;
use serde_json::json;

// 示例1：直接返回 serde_json::Value
#[potato::http_get("/json-value")]
async fn handler_json_value() -> serde_json::Value {
    json!({
        "message": "Hello from JSON Value",
        "status": "success",
        "data": {
            "count": 42,
            "active": true,
            "tags": ["rust", "web", "json"]
        }
    })
}

// 示例2：返回 anyhow::Result<serde_json::Value> - 成功情况
#[potato::http_get("/result-json-success")]
async fn handler_result_json_success() -> anyhow::Result<serde_json::Value> {
    Ok(json!({
        "message": "Success result",
        "code": 200,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

// 示例3：返回 anyhow::Result<serde_json::Value> - 可能失败的情况
#[potato::http_get("/result-json-maybe-error")]
async fn handler_result_json_maybe_error(fail: bool) -> anyhow::Result<serde_json::Value> {
    if fail {
        Err(anyhow::anyhow!("Operation failed as requested"))
    } else {
        Ok(json!({
            "message": "Success",
            "fail_requested": false
        }))
    }
}

// 示例4：带 HttpRequest 参数的 JSON Value 返回
#[potato::http_get("/json-with-request")]
async fn handler_json_with_request(req: &mut potato::HttpRequest) -> serde_json::Value {
    let path = req.url_path.to_string();
    let method = req.method.to_string();
    
    json!({
        "path": path,
        "method": method,
        "message": "Request info returned as JSON"
    })
}

// 示例5：复杂嵌套 JSON 结构
#[potato::http_get("/complex-json")]
async fn handler_complex_json() -> serde_json::Value {
    json!({
        "users": [
            {
                "id": 1,
                "name": "Alice",
                "email": "alice@example.com",
                "roles": ["admin", "user"]
            },
            {
                "id": 2,
                "name": "Bob",
                "email": "bob@example.com",
                "roles": ["user"]
            }
        ],
        "metadata": {
            "total": 2,
            "page": 1,
            "per_page": 10
        }
    })
}

// 示例6：使用 serde_json::json! 宏构建动态 JSON
#[potato::http_get("/dynamic-json")]
async fn handler_dynamic_json(items_count: u32) -> serde_json::Value {
    let mut items = Vec::new();
    for i in 1..=items_count {
        items.push(json!({
            "id": i,
            "value": format!("item-{}", i)
        }));
    }
    
    json!({
        "count": items_count,
        "items": items
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("JSON Value Return Type Examples");
    println!("================================");
    println!();
    println!("Available endpoints:");
    println!("  GET /json-value              - Direct JSON Value return");
    println!("  GET /result-json-success     - Result<JSON Value> success case");
    println!("  GET /result-json-maybe-error - Result<JSON Value> with error handling");
    println!("  GET /json-with-request       - JSON with HttpRequest parameter");
    println!("  GET /complex-json            - Complex nested JSON structure");
    println!("  GET /dynamic-json            - Dynamic JSON building");
    println!();
    println!("Example URLs:");
    println!("  http://127.0.0.1:8080/json-value");
    println!("  http://127.0.0.1:8080/result-json-maybe-error?fail=true");
    println!("  http://127.0.0.1:8080/dynamic-json?items_count=5");
    println!();
    
    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!();
    
    server.serve_http().await?;
    Ok(())
}
