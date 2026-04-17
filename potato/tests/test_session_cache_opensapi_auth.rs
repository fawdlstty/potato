/// 测试 SessionCache 自动标记为需要鉴权的功能
use potato::{HttpResponse, HttpServer, SessionCache};

// 不使用 SessionCache 的接口 - 不应该标记为需要鉴权
#[potato::http_get("/api/public")]
async fn public_handler() -> HttpResponse {
    HttpResponse::text("Public endpoint")
}

// 使用 SessionCache 的接口 - 应该标记为需要鉴权
#[potato::http_get("/api/private")]
async fn private_handler(cache: &mut SessionCache) -> HttpResponse {
    let count: u32 = cache.get("count").unwrap_or(0);
    let body = serde_json::json!({ "count": count });
    HttpResponse::json(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "openapi")]
    #[tokio::test]
    async fn test_session_cache_opensapi_auth_marker() {
        use std::time::Duration;

        let port = 18888;
        let server_addr = format!("127.0.0.1:{}", port);

        // 创建服务器并启用 OpenAPI
        let mut server = HttpServer::new(&server_addr);
        server.configure(|ctx| {
            // 不调用 use_handlers，只注册 OpenAPI 路由
            ctx.use_openapi("/doc/");
        });

        // 启动服务器
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        // 等待服务器启动
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 获取 OpenAPI JSON
        let url = format!("http://{}/doc/index.json", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                let openapi_json = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8_lossy(data).to_string()
                    }
                    potato::HttpResponseBody::Stream(_) => "{}".to_string(),
                };

                let openapi: serde_json::Value =
                    serde_json::from_str(&openapi_json).expect("Invalid JSON");

                println!(
                    "OpenAPI JSON:\n{}",
                    serde_json::to_string_pretty(&openapi).unwrap()
                );

                // 验证 /api/public 没有 security 标记
                let public_path = &openapi["paths"]["/api/public"]["get"];
                assert!(
                    public_path.get("security").is_none(),
                    "/api/public should NOT have security标记"
                );
                println!("✅ /api/public correctly has NO security requirement");

                // 验证 /api/private 有 security 标记
                let private_path = &openapi["paths"]["/api/private"]["get"];
                assert!(
                    private_path.get("security").is_some(),
                    "/api/private should have security标记"
                );

                let security = &private_path["security"];
                assert!(
                    security.as_array().unwrap().len() > 0,
                    "security array should not be empty"
                );

                let bearer_auth = &security[0]["bearerAuth"];
                assert!(
                    bearer_auth.is_array() && bearer_auth.as_array().unwrap().is_empty(),
                    "bearerAuth should be an empty array"
                );
                println!("✅ /api/private correctly has security requirement with bearerAuth");

                // 验证 components.securitySchemes 中有 bearerAuth 定义
                let security_schemes = &openapi["components"]["securitySchemes"]["bearerAuth"];
                assert!(
                    security_schemes.get("type").is_some(),
                    "bearerAuth security scheme should be defined"
                );
                assert_eq!(
                    security_schemes["type"].as_str().unwrap(),
                    "http",
                    "bearerAuth type should be 'http'"
                );
                assert_eq!(
                    security_schemes["scheme"].as_str().unwrap(),
                    "Bearer",
                    "bearerAuth scheme should be 'Bearer'"
                );
                println!("✅ bearerAuth security scheme correctly defined");

                // 验证 /api/private 有 401 响应码
                let responses = &private_path["responses"];
                assert!(
                    responses.get("401").is_some(),
                    "/api/private should have 401 response"
                );
                println!("✅ /api/private correctly has 401 response");

                println!("\n🎉 All tests passed! SessionCache correctly triggers auth marking in OpenAPI");
            }
            Err(e) => {
                eprintln!("Failed to get OpenAPI JSON: {}", e);
                panic!("Failed to get OpenAPI JSON");
            }
        }

        server_handle.abort();
    }
}
