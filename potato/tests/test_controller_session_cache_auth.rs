/// 测试 Controller 成员方法中 SessionCache 的鉴权验证
/// 确保当缺少 Authorization header 时返回 401 错误
use potato::{HttpServer, OnceCache, SessionCache};

// 定义 Controller 结构体
#[potato::controller]
pub struct AuthTestController<'a> {
    pub once_cache: &'a OnceCache,
    pub sess_cache: &'a SessionCache,
}

// 实现 Controller 方法
#[potato::controller("/api/auth-test")]
impl<'a> AuthTestController<'a> {
    // 带 &self 的 GET 方法 - 需要鉴权
    #[potato::http_get("/protected")]
    pub async fn get_protected(&self) -> anyhow::Result<&'static str> {
        Ok("Protected data accessed successfully")
    }

    // 带 &mut self 的 POST 方法 - 需要鉴权
    #[potato::http_post("/update")]
    pub async fn update_data(&mut self) -> anyhow::Result<&'static str> {
        Ok("Data updated successfully")
    }

    // 不带 receiver 的静态方法 - 不需要鉴权
    #[potato::http_get("/public")]
    pub async fn get_public() -> anyhow::Result<&'static str> {
        Ok("Public data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_controller_requires_auth_without_header() {
        let port = 18890;
        let server_addr = format!("127.0.0.1:{}", port);

        // 创建服务器
        let mut server = HttpServer::new(&server_addr);
        server.configure(|ctx| {
            ctx.use_handlers();
        });

        // 启动服务器
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        // 等待服务器启动
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 测试 1: 不带 Authorization header 访问受保护的方法应该返回 401
        let url = format!("http://{}/api/auth-test/protected", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(
                    res.http_code, 401,
                    "Should return 401 without Authorization header"
                );
                println!("✅ GET /api/auth-test/protected correctly returns 401 without auth");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        // 测试 2: 不带 Authorization header 访问 POST 方法应该返回 401
        let url = format!("http://{}/api/auth-test/update", server_addr);
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(
                    res.http_code, 401,
                    "Should return 401 without Authorization header"
                );
                println!("✅ POST /api/auth-test/update correctly returns 401 without auth");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        // 测试 3: 不带 receiver 的公开方法应该可以访问（返回 200）
        let url = format!("http://{}/api/auth-test/public", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200, "Public endpoint should return 200");
                println!("✅ GET /api/auth-test/public correctly returns 200 (no auth required)");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        // 测试 4: 带有效的 Authorization header 应该可以访问受保护的方法
        // 首先生成一个有效的 token
        SessionCache::set_jwt_secret(b"test-secret-key-for-testing").await;
        let token = SessionCache::generate_token(123, Duration::from_secs(3600))
            .await
            .unwrap();

        let url = format!("http://{}/api/auth-test/protected", server_addr);
        match potato::get!(&url, Authorization = format!("Bearer {}", token)).await {
            Ok(res) => {
                assert_eq!(
                    res.http_code, 200,
                    "Should return 200 with valid Authorization header"
                );
                println!("✅ GET /api/auth-test/protected correctly returns 200 with valid auth");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        server_handle.abort();
    }
}
