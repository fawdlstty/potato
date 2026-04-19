/// 集成测试：验证 serde_json::Value 返回类型在实际 HTTP 服务中的工作
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_json_value_return_integration() -> anyhow::Result<()> {
    use potato::HttpServer;

    // 测试 serde_json::Value 返回
    #[potato::http_get("/json-value")]
    async fn handler_json_value() -> serde_json::Value {
        serde_json::json!({
            "message": "Hello from JSON Value",
            "status": "success",
            "data": {
                "count": 42,
                "active": true
            }
        })
    }

    // 测试 anyhow::Result<serde_json::Value> 返回 - 成功情况
    #[potato::http_get("/result-json-value-success")]
    async fn handler_result_json_value_success() -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "message": "Success result",
            "code": 200
        }))
    }

    // 测试 anyhow::Result<serde_json::Value> 返回 - 错误情况
    #[potato::http_get("/result-json-value-error")]
    async fn handler_result_json_value_error(fail: bool) -> anyhow::Result<serde_json::Value> {
        if fail {
            Err(anyhow::anyhow!("Operation failed"))
        } else {
            Ok(serde_json::json!({
                "status": "success"
            }))
        }
    }

    // 测试带参数的 JSON Value 返回
    #[potato::http_post("/json-value-with-params")]
    async fn handler_json_value_with_params(name: String, age: u32) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "age": age,
            "message": "User created"
        })
    }

    let server_addr = "127.0.0.1:18090";
    let mut server = HttpServer::new(server_addr);
    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    // 等待服务器启动
    sleep(Duration::from_millis(300)).await;

    // 测试 1: 直接返回 serde_json::Value
    let url = format!("http://{}/json-value", server_addr);
    let res = potato::get(&url, vec![]).await?;
    assert_eq!(res.http_code, 200);
    let body = match &res.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };
    println!("Test 1 - JSON Value response: {}", body);

    // 验证返回的是有效的 JSON
    let json: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(json["message"], "Hello from JSON Value");
    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["count"], 42);
    assert_eq!(json["data"]["active"], true);

    // 测试 2: 返回 Result<serde_json::Value> - 成功
    let url = format!("http://{}/result-json-value-success", server_addr);
    let res = potato::get(&url, vec![]).await?;
    assert_eq!(res.http_code, 200);
    let body = match &res.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };
    println!("Test 2 - Result JSON Value success: {}", body);

    let json: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(json["message"], "Success result");
    assert_eq!(json["code"], 200);

    // 测试 3: 返回 Result<serde_json::Value> - 错误
    let url = format!("http://{}/result-json-value-error", server_addr);
    let res = potato::get(&url, vec![]).await?;
    // 错误应该返回 500
    assert_eq!(res.http_code, 500);
    println!("Test 3 - Result JSON Value error: HTTP {}", res.http_code);

    // 清理
    server_handle.abort();

    println!("All JSON Value return integration tests passed!");
    Ok(())
}

#[tokio::test]
async fn test_json_value_content_type() -> anyhow::Result<()> {
    use potato::HttpServer;

    #[potato::http_get("/json-content-type-test")]
    async fn handler_json_content_type() -> serde_json::Value {
        serde_json::json!({
            "test": "content type"
        })
    }

    let server_addr = "127.0.0.1:18091";
    let mut server = HttpServer::new(server_addr);
    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/json-content-type-test", server_addr);
    let res = potato::get(&url, vec![]).await?;
    assert_eq!(res.http_code, 200);

    // 验证 Content-Type 是 application/json
    let content_type = res.headers.get("Content-Type");
    assert!(
        content_type.is_some(),
        "Content-Type header should be present"
    );
    println!("Content-Type: {:?}", content_type);
    // Content-Type 应该包含 application/json
    assert!(
        content_type.unwrap().contains("application/json"),
        "Content-Type should be application/json, got: {}",
        content_type.unwrap()
    );

    let body = match &res.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };
    let json: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(json["test"], "content type");

    server_handle.abort();
    println!("Content-Type test passed!");
    Ok(())
}
