/// Controller 返回类型完整性测试 - 集成测试

#[potato::controller]
pub struct ReturnTypeTestController<'a> {
    pub once_cache: &'a potato::OnceCache,
}

#[potato::controller("/test/return-types")]
impl<'a> ReturnTypeTestController<'a> {
    // 1. 返回 HttpResponse
    #[potato::http_get("/http-response")]
    pub async fn test_http_response(&self) -> potato::HttpResponse {
        potato::HttpResponse::html("HttpResponse")
    }

    // 2. 返回 Result<HttpResponse>
    #[potato::http_get("/result-http-response")]
    pub async fn test_result_http_response(&self) -> anyhow::Result<potato::HttpResponse> {
        Ok(potato::HttpResponse::html("Result<HttpResponse>"))
    }

    // 3. 返回 String
    #[potato::http_get("/string")]
    pub async fn test_string(&self) -> String {
        "String".to_string()
    }

    // 4. 返回 Result<String>
    #[potato::http_get("/result-string")]
    pub async fn test_result_string(&self) -> anyhow::Result<String> {
        Ok("Result<String>".to_string())
    }

    // 5. 返回 &'static str
    #[potato::http_get("/static-str")]
    pub async fn test_static_str(&self) -> &'static str {
        "&'static str"
    }

    // 6. 返回 Result<&'static str>
    #[potato::http_get("/result-static-str")]
    pub async fn test_result_static_str(&self) -> anyhow::Result<&'static str> {
        Ok("Result<&'static str>")
    }

    // 7. 返回 ()
    #[potato::http_get("/unit")]
    pub async fn test_unit(&self) {
        println!("Unit return");
    }

    // 8. 返回 Result<()>
    #[potato::http_get("/result-unit")]
    pub async fn test_result_unit(&self) -> anyhow::Result<()> {
        println!("Result<()> return");
        Ok(())
    }

    // 9. 返回 serde_json::Value
    #[potato::http_get("/json-value")]
    pub async fn test_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "serde_json::Value",
            "message": "JSON value return"
        })
    }

    // 10. 返回 Result<serde_json::Value>
    #[potato::http_get("/result-json-value")]
    pub async fn test_result_json_value(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "type": "Result<serde_json::Value>",
            "message": "Result JSON value return"
        }))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8081");
    server.configure(|ctx| {
        ctx.use_handlers();
    });

    println!("测试 Controller 返回类型:");
    println!("1.  http://127.0.0.1:8081/test/return-types/http-response");
    println!("2.  http://127.0.0.1:8081/test/return-types/result-http-response");
    println!("3.  http://127.0.0.1:8081/test/return-types/string");
    println!("4.  http://127.0.0.1:8081/test/return-types/result-string");
    println!("5.  http://127.0.0.1:8081/test/return-types/static-str");
    println!("6.  http://127.0.0.1:8081/test/return-types/result-static-str");
    println!("7.  http://127.0.0.1:8081/test/return-types/unit");
    println!("8.  http://127.0.0.1:8081/test/return-types/result-unit");
    println!("9.  http://127.0.0.1:8081/test/return-types/json-value");
    println!("10. http://127.0.0.1:8081/test/return-types/result-json-value");

    server.serve_http().await
}
