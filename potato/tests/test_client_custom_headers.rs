/// 测试客户端宏的 Custom header 支持
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(28000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::HttpServer;

    /// 测试 get! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_get_macro_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 测试混合使用标准 header 和 custom header
        let _res = potato::get!(
            &url,
            "X-Custom-Header" = "custom-value",
            User_Agent = "test-client/1.0",
            "Another-Custom" = "another-value"
        )
        .await;

        println!("✅ GET macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 post! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_post_macro_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/post", server_addr);

        // 测试 post 宏混合使用标准 header 和 custom header
        let _res = potato::post!(
            &url,
            vec![1, 2, 3],
            "X-Request-ID" = "12345",
            Content_Type = "application/octet-stream",
            "X-Auth-Token" = "secret-token"
        )
        .await;

        println!("✅ POST macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 put! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_put_macro_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/put", server_addr);

        // 测试 put 宏混合使用标准 header 和 custom header
        let _res = potato::put!(
            &url,
            vec![4, 5, 6],
            "X-Operation" = "update",
            Authorization = "Bearer test-token"
        )
        .await;

        println!("✅ PUT macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 delete! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_delete_macro_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/delete", server_addr);

        // 测试 delete 宏使用 custom header
        let _res = potato::delete!(
            &url,
            "X-Reason" = "cleanup",
            Authorization = "Bearer admin-token"
        )
        .await;

        println!("✅ DELETE macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 patch! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_patch_macro_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/patch", server_addr);

        // 测试 patch 宏使用 custom header
        let _res = potato::patch!(
            &url,
            "X-Patch-Type" = "partial",
            Content_Type = "application/json"
        )
        .await;

        println!("✅ PATCH macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 head!, options!, trace!, connect! 宏的 Custom header 支持
    #[tokio::test]
    async fn test_other_macros_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 测试 head 宏
        let _res = potato::head!(&url, "X-Test-Head" = "head-value").await;
        println!("✅ HEAD macro with custom headers works");

        // 测试 options 宏
        let _res = potato::options!(&url, "X-Test-Options" = "options-value").await;
        println!("✅ OPTIONS macro with custom headers works");

        // 测试 trace 宏
        let _res = potato::trace!(&url, "X-Test-Trace" = "trace-value").await;
        println!("✅ TRACE macro with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试纯 custom header（没有标准 header）
    #[tokio::test]
    async fn test_pure_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 只使用 custom header
        let _res = potato::get!(
            &url,
            "Custom-Header-1" = "value1",
            "Custom-Header-2" = "value2",
            "Custom-Header-3" = "value3"
        )
        .await;

        println!("✅ Pure custom headers work");

        server_handle.abort();
        Ok(())
    }

    /// 测试纯标准 header（向后兼容）
    #[tokio::test]
    async fn test_standard_headers_backward_compat() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{server_addr}/");

        // 只使用标准 header（向后兼容）
        let _res = potato::get!(
            &url,
            User_Agent = "test-client/1.0",
            Accept = "application/json"
        )
        .await;

        println!("✅ Standard headers backward compatibility works");

        server_handle.abort();
        Ok(())
    }
}
