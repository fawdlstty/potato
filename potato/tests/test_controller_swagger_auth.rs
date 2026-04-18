/// 测试 Controller Swagger 鉴权标记
use potato::{HttpRequest, HttpResponse, HttpServer, OnceCache, SessionCache};

#[potato::preprocess]
fn my_preprocess(_req: &mut HttpRequest) -> anyhow::Result<()> {
    Ok(())
}

#[potato::controller]
pub struct UsersController<'a> {
    pub once_cache: &'a OnceCache,
    pub sess_cache: &'a SessionCache,
}

#[potato::controller("/api/users")]
#[potato::preprocess(my_preprocess)]
impl<'a> UsersController<'a> {
    #[potato::http_get] // 地址为 "/api/users"，有 &self，应该标记为需要鉴权
    pub async fn get(&self) -> anyhow::Result<&'static str> {
        Ok("get users data")
    }

    #[potato::http_post] // 地址为 "/api/users"，有 &mut self，应该标记为需要鉴权
    pub async fn post(&mut self) -> anyhow::Result<&'static str> {
        Ok("post users data")
    }

    #[potato::http_get("/any")] // 地址为 "/api/users/any"，有 &self，应该标记为需要鉴权
    pub async fn get_any(&self) -> anyhow::Result<&'static str> {
        Ok("get users any data")
    }
}

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
    async fn test_controller_swagger_auth_marker() {
        use std::time::Duration;

        let port = 18889;
        let server_addr = format!("127.0.0.1:{}", port);

        // 创建服务器并启用 OpenAPI
        let mut server = HttpServer::new(&server_addr);
        server.configure(|ctx| {
            ctx.use_openapi("/doc/");
        });

        // 启动服务器
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        // 等待服务器启动
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 获取 Swagger JSON
        let url = format!("http://{}/doc/index.json", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                let swagger_json = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8_lossy(data).to_string()
                    }
                    potato::HttpResponseBody::Stream(_) => "{}".to_string(),
                };

                let swagger: serde_json::Value =
                    serde_json::from_str(&swagger_json).expect("Invalid JSON");

                println!(
                    "Swagger JSON:\n{}",
                    serde_json::to_string_pretty(&swagger).unwrap()
                );

                // 验证 UsersController 的方法都标记为需要鉴权
                let paths = swagger["paths"].as_object().unwrap();

                // /api/users GET 应该有 security 字段
                let users_get = &paths["/api/users"]["get"];
                assert!(
                    users_get.get("security").is_some(),
                    "/api/users GET should have security marker (has &self receiver)"
                );
                println!("✅ /api/users GET correctly has security requirement");

                // /api/users POST 应该有 security 字段
                let users_post = &paths["/api/users"]["post"];
                assert!(
                    users_post.get("security").is_some(),
                    "/api/users POST should have security marker (has &mut self receiver)"
                );
                println!("✅ /api/users POST correctly has security requirement");

                // /api/users/any GET 应该有 security 字段
                let users_any_get = &paths["/api/users/any"]["get"];
                assert!(
                    users_any_get.get("security").is_some(),
                    "/api/users/any GET should have security marker (has &self receiver)"
                );
                println!("✅ /api/users/any GET correctly has security requirement");

                // /api/public GET 不应该有 security 字段
                let public_get = &paths["/api/public"]["get"];
                assert!(
                    public_get.get("security").is_none(),
                    "/api/public GET should NOT have security marker"
                );
                println!("✅ /api/public GET correctly has NO security requirement");

                // /api/private GET 应该有 security 字段（因为有 SessionCache 参数）
                let private_get = &paths["/api/private"]["get"];
                assert!(
                    private_get.get("security").is_some(),
                    "/api/private GET should have security marker (has SessionCache parameter)"
                );
                println!("✅ /api/private GET correctly has security requirement");

                // 验证 components.securitySchemes 中有 bearerAuth 定义
                let security_schemes = &swagger["components"]["securitySchemes"]["bearerAuth"];
                assert!(
                    security_schemes.get("type").is_some(),
                    "bearerAuth security scheme should be defined"
                );
                assert_eq!(
                    security_schemes["type"].as_str().unwrap(),
                    "http",
                    "bearerAuth type should be 'http'"
                );
                println!("✅ bearerAuth security scheme correctly defined");

                // 清理
                server_handle.abort();
            }
            Err(e) => {
                panic!("Failed to get swagger json: {}", e);
            }
        }
    }
}
