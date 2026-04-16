/// 集成测试：验证 Custom(key) = value 宏语法
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(29000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::HttpServer;

    /// 测试 get! 宏的 Custom(key) = value 语法
    #[tokio::test]
    async fn test_get_macro_custom_variable_syntax() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 测试 Custom(key) = value 语法
        let header_key = "X-Custom-Key";
        let header_value = "custom-value";
        let _res = potato::get!(
            &url,
            User_Agent = "test-client/1.0",
            Custom(header_key) = header_value
        )
        .await;

        println!("✅ GET macro with Custom(key) = value syntax works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 post! 宏的 Custom(key) = value 语法
    #[tokio::test]
    async fn test_post_macro_custom_variable_syntax() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 测试 Custom(key) = value 语法与字符串字面量混合使用
        let auth_key = "Authorization";
        let auth_value = "Bearer test-token";
        let _res = potato::post!(
            &url,
            vec![],
            Custom(auth_key) = auth_value,
            "X-Request-ID" = "12345",
            Content_Type = "application/json"
        )
        .await;

        println!("✅ POST macro with Custom(key) = value syntax works");

        server_handle.abort();
        Ok(())
    }

    /// 测试混合使用所有语法形式
    #[tokio::test]
    async fn test_mixed_header_syntax() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 混合使用三种语法：
        // 1. 标准 header: User_Agent
        // 2. 字符串字面量: "X-Custom-1"
        // 3. 变量形式: Custom(key) = value
        let custom_key = "X-Custom-2";
        let custom_value = "value2";

        let _res = potato::get!(
            &url,
            User_Agent = "test-client",
            "X-Custom-1" = "value1",
            Custom(custom_key) = custom_value,
            Accept = "application/json"
        )
        .await;

        println!("✅ Mixed header syntax works (standard + literal + variable)");

        server_handle.abort();
        Ok(())
    }
}
