/// 集成测试：验证 String 和 &'static str 返回类型在实际 HTTP 服务器中的工作
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(19000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试异步 handler 返回 String 类型
    #[tokio::test]
    async fn test_async_string_return() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/test-string")]
        async fn handler_string() -> String {
            "<html><body><h1>Hello from String</h1></body></html>".to_string()
        }

        let mut server = potato::HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/test-string", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = match &res.body {
            potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
            _ => panic!("Expected data body"),
        };
        assert!(body.contains("<h1>Hello from String</h1>"));

        server_handle.abort();
        Ok(())
    }

    /// 测试异步 handler 返回 &'static str 类型
    #[tokio::test]
    async fn test_async_static_str_return() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/test-static-str")]
        async fn handler_static_str() -> &'static str {
            "<html><body><h1>Hello from &'static str</h1></body></html>"
        }

        let mut server = potato::HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/test-static-str", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = match &res.body {
            potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
            _ => panic!("Expected data body"),
        };
        assert!(body.contains("<h1>Hello from &'static str</h1>"));

        server_handle.abort();
        Ok(())
    }

    /// 测试异步 handler 返回 anyhow::Result<String> 类型
    #[tokio::test]
    async fn test_async_result_string_return() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/test-result-string")]
        async fn handler_result_string(success: bool) -> anyhow::Result<String> {
            if success {
                Ok("<html><body><h1>Success</h1></body></html>".to_string())
            } else {
                anyhow::bail!("Operation failed")
            }
        }

        let mut server = potato::HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 测试成功情况
        let url = format!("http://{}/test-result-string?success=true", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 200);

        // 测试失败情况
        let url = format!("http://{}/test-result-string?success=false", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 500);

        server_handle.abort();
        Ok(())
    }

    /// 测试同步 handler 返回 String 类型
    #[tokio::test]
    async fn test_sync_string_return() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/test-string-sync")]
        fn handler_string_sync() -> String {
            "<html><body><h1>Hello from sync String</h1></body></html>".to_string()
        }

        let mut server = potato::HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/test-string-sync", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = match &res.body {
            potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
            _ => panic!("Expected data body"),
        };
        assert!(body.contains("<h1>Hello from sync String</h1>"));

        server_handle.abort();
        Ok(())
    }

    /// 测试同步 handler 返回 &'static str 类型
    #[tokio::test]
    async fn test_sync_static_str_return() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/test-static-str-sync")]
        fn handler_static_str_sync() -> &'static str {
            "<html><body><h1>Hello from sync &'static str</h1></body></html>"
        }

        let mut server = potato::HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/test-static-str-sync", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);
        let body = match &res.body {
            potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
            _ => panic!("Expected data body"),
        };
        assert!(body.contains("<h1>Hello from sync &'static str</h1>"));

        server_handle.abort();
        Ok(())
    }
}
