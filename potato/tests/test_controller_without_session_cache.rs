/// 测试不带 SessionCache 成员的 Controller 结构体
/// 确保不需要鉴权也能正常工作
use potato::{HttpServer, OnceCache};

// 定义不带 SessionCache 的 Controller 结构体
#[potato::controller]
pub struct NoAuthController<'a> {
    pub once_cache: &'a OnceCache,
    // 注意：没有 sess_cache 字段
}

// 实现 Controller 方法
#[potato::controller("/api/no-auth")]
impl<'a> NoAuthController<'a> {
    // 带 &self 的 GET 方法 - 不需要鉴权
    #[potato::http_get("/data")]
    pub async fn get_data(&self) -> anyhow::Result<&'static str> {
        Ok("Data accessed without auth")
    }

    // 带 &mut self 的 POST 方法 - 不需要鉴权
    #[potato::http_post("/update")]
    pub async fn update_data(&mut self) -> anyhow::Result<&'static str> {
        Ok("Data updated without auth")
    }

    // 不带 receiver 的静态方法
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
    async fn test_controller_without_session_cache() {
        let port = 18891;
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

        // 测试 1: 不带 Authorization header 访问带 &self 的方法应该成功（返回 200）
        let url = format!("http://{}/api/no-auth/data", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200, "Should return 200 without Authorization header");
                println!("✅ GET /api/no-auth/data correctly returns 200 without auth");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        // 测试 2: 不带 Authorization header 访问带 &mut self 的方法应该成功
        let url = format!("http://{}/api/no-auth/update", server_addr);
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200, "Should return 200 without Authorization header");
                println!("✅ POST /api/no-auth/update correctly returns 200 without auth");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        // 测试 3: 不带 receiver 的公开方法应该可以访问
        let url = format!("http://{}/api/no-auth/public", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200, "Public endpoint should return 200");
                println!("✅ GET /api/no-auth/public correctly returns 200");
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }

        server_handle.abort();
    }
}
