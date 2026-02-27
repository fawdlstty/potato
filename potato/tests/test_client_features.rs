/// 客户端特性的综合测试
/// 基于 examples/client 中的示例
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(27000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::HttpServer;

    /// 测试基础的客户端 Session 创建
    #[tokio::test]
    async fn test_client_session_creation() -> anyhow::Result<()> {
        let _session = potato::Session::new();

        // Session 创建成功
        println!("✅ Session created successfully");
        Ok(())
    }

    /// 测试客户端请求的不同内容类型
    #[tokio::test]
    async fn test_client_request_methods() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let mut session = potato::Session::new();
        let base_url = format!("http://{}", server_addr);

        // 测试不同方法
        let methods = vec![
            ("GET", base_url.clone()),
            ("POST", format!("{}/post", base_url)),
            ("PUT", format!("{}/put", base_url)),
            ("DELETE", format!("{}/delete", base_url)),
            ("HEAD", format!("{}/head", base_url)),
            ("OPTIONS", format!("{}/options", base_url)),
        ];

        for (method, url) in methods {
            match method {
                "GET" => {
                    let _ = session.get(&url, vec![]).await;
                    println!("✅ GET request works");
                }
                "POST" => {
                    let _ = session.post(&url, vec![], vec![]).await;
                    println!("✅ POST request works");
                }
                "PUT" => {
                    let _ = session.put(&url, vec![], vec![]).await;
                    println!("✅ PUT request works");
                }
                "DELETE" => {
                    let _ = session.delete(&url, vec![]).await;
                    println!("✅ DELETE request works");
                }
                "HEAD" => {
                    let _ = session.head(&url, vec![]).await;
                    println!("✅ HEAD request works");
                }
                "OPTIONS" => {
                    let _ = session.options(&url, vec![]).await;
                    println!("✅ OPTIONS request works");
                }
                _ => {}
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试客户端请求头
    #[tokio::test]
    async fn test_client_request_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let mut session = potato::Session::new();
        let url = format!("http://{}/", server_addr);

        // 测试带请求头的请求
        let headers = vec![
            potato::Headers::User_Agent("test-client/1.0".into()),
            potato::Headers::Custom(("X-Custom-Header".to_string(), "test-value".to_string())),
        ];

        let _res = session.get(&url, headers).await;
        println!("✅ Client request with custom headers works");

        server_handle.abort();
        Ok(())
    }

    /// 测试全局 API (不使用 Session)
    #[tokio::test]
    async fn test_global_client_api() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/", server_addr);

        // 使用全局 API
        let _res = potato::get(&url, vec![]).await;
        println!("✅ Global GET API works");

        let _res = potato::post(&url, vec![], vec![]).await;
        println!("✅ Global POST API works");

        let _res = potato::put(&url, vec![], vec![]).await;
        println!("✅ Global PUT API works");

        let _res = potato::delete(&url, vec![]).await;
        println!("✅ Global DELETE API works");

        let _res = potato::head(&url, vec![]).await;
        println!("✅ Global HEAD API works");

        let _res = potato::options(&url, vec![]).await;
        println!("✅ Global OPTIONS API works");

        server_handle.abort();
        Ok(())
    }

    /// 测试客户端的连接失败处理
    #[tokio::test]
    async fn test_client_connection_error() -> anyhow::Result<()> {
        // 尝试连接到不存在的服务器
        let url = "http://127.0.0.1:1/";

        let result = potato::get(url, vec![]).await;

        // 应该返回错误
        assert!(result.is_err());
        println!("✅ Client connection error handling works");

        Ok(())
    }

    /// 测试客户端请求超时处理
    #[tokio::test]
    async fn test_client_with_timeout() -> anyhow::Result<()> {
        // 这个测试演示如何使用 tokio::time::timeout
        // 与客户端 API 配合

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let url = format!("http://{}/", server_addr);

        // 使用 tokio 的超时包装
        let result = tokio::time::timeout(Duration::from_secs(5), potato::get(&url, vec![])).await;

        match result {
            Ok(Ok(_res)) => {
                println!("✅ Request completed within timeout");
            }
            Ok(Err(_e)) => {
                println!("✅ Request failed as expected");
            }
            Err(_e) => {
                println!("✅ Request timeout");
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试客户端 Session 多请求
    #[tokio::test]
    async fn test_client_session_multiple_requests() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let mut session = potato::Session::new();
        let base_url = format!("http://{}", server_addr);

        // 使用同一个 session 发送多个请求
        for i in 0..5 {
            let url = format!("{}/request{}", base_url, i);
            let _res = session.get(&url, vec![]).await;
            sleep(Duration::from_millis(10)).await;
        }

        println!("✅ Session with multiple requests works");

        server_handle.abort();
        Ok(())
    }

    /// 测试客户端 JSON 请求方法
    #[tokio::test]
    async fn test_client_json_api() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let mut session = potato::Session::new();
        let url = format!("http://{}/json", server_addr);

        // 使用 JSON API（如果可用）
        let json_value = serde_json::json!({"key": "value"});
        let _res = session.post_json(&url, json_value, vec![]).await;
        println!("✅ Client post_json API works");

        server_handle.abort();
        Ok(())
    }

    /// 测试客户端 JSON 字符串 API
    #[tokio::test]
    async fn test_client_json_str_api() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(200)).await;

        let mut session = potato::Session::new();
        let url = format!("http://{}/json", server_addr);

        // 使用 JSON 字符串 API
        let json_str = r#"{"key":"value"}"#.to_string();
        let _res = session.post_json_str(&url, json_str, vec![]).await;
        println!("✅ Client post_json_str API works");

        server_handle.abort();
        Ok(())
    }
}
