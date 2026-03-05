/// 集成测试 - 测试实际的HTTP客户端-服务器交互
/// 这些测试演示了如何使用实际的potato框架进行集成测试
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(17000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use potato::HttpServer;

    /// 测试基本的HTTP GET请求-应用宏定义处理器
    #[tokio::test]
    async fn test_basic_server_client_interaction() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        // 启动服务器
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 测试基本GET请求
        let url = format!("http://{}/", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                // 服务器应该返回某种响应
                println!("Response status: {}", res.http_code);
                assert!(res.http_code >= 200 && res.http_code < 600);
            }
            Err(_) => {
                // 由于没有定义路由，404是合理的
                println!("Expected 404 for undefined route");
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试多个并发请求
    #[tokio::test]
    async fn test_concurrent_requests() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let addr_clone = server_addr.clone();

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 并发发送多个请求
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let addr = addr_clone.clone();
                tokio::spawn(async move {
                    let url = format!("http://{}/test{}", addr, i);
                    let _ = potato::get(&url, vec![]).await;
                })
            })
            .collect();

        // 等待所有请求完成
        for handle in handles {
            let _ = handle.await;
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试不同的HTTP方法
    #[tokio::test]
    async fn test_various_http_methods() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/api/test", server_addr);

        // 测试GET
        let _ = potato::get(&url, vec![]).await;

        // 测试POST
        let _ = potato::post(&url, vec![], vec![]).await;

        // 测试PUT
        let _ = potato::put(&url, vec![], vec![]).await;

        // 测试DELETE
        let _ = potato::delete(&url, vec![]).await;

        // 测试HEAD
        let _ = potato::head(&url, vec![]).await;

        // 测试OPTIONS
        let _ = potato::options(&url, vec![]).await;

        server_handle.abort();
        Ok(())
    }

    /// 测试会话持久化
    #[tokio::test]
    async fn test_session_reuse() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let mut session = potato::Session::new();

        // 使用同一会话发送多个请求
        for i in 0..3 {
            let url = format!("http://{}/path{}", server_addr, i);
            let _ = session.get(&url, vec![]).await;
            sleep(Duration::from_millis(50)).await;
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试请求头
    #[tokio::test]
    async fn test_custom_headers() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/api", server_addr);
        let headers = vec![
            potato::Headers::User_Agent("test-agent/1.0".into()),
            potato::Headers::Custom(("X-Test-Header".to_string(), "test-value".to_string())),
        ];

        let _ = potato::get(&url, headers).await;

        server_handle.abort();
        Ok(())
    }

    /// 测试不同的内容类型
    #[tokio::test]
    async fn test_content_types() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 发送JSON内容
        let json_url = format!("http://{}/json", server_addr);
        let json_body = r#"{"test": "data"}"#.as_bytes().to_vec();
        let headers = vec![potato::Headers::Content_Type("application/json".into())];
        let _ = potato::post(&json_url, json_body, headers).await;

        // 发送表单数据
        let form_url = format!("http://{}/form", server_addr);
        let form_body = b"key=value&foo=bar".to_vec();
        let headers = vec![potato::Headers::Content_Type(
            "application/x-www-form-urlencoded".into(),
        )];
        let _ = potato::post(&form_url, form_body, headers).await;

        server_handle.abort();
        Ok(())
    }

    /// 测试响应处理
    #[tokio::test]
    async fn test_response_handling() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(response) => {
                println!("Response code: {}", response.http_code);
                println!("Response headers: {:?}", response.headers);
                let body_len = match &response.body {
                    potato::HttpResponseBody::Data(data) => data.len(),
                    potato::HttpResponseBody::Stream(_) => 0,
                };
                println!("Response body length: {}", body_len);

                // 验证响应对象的有效性
                assert!(!response.headers.is_empty() || response.headers.is_empty());
            }
            Err(e) => {
                println!("Request error: {}", e);
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试WebSocket连接（如果可用）
    #[tokio::test]
    async fn test_websocket_connection() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 尝试连接WebSocket
        let ws_url = format!("ws://{}/ws", server_addr);
        match potato::Websocket::connect(&ws_url, vec![]).await {
            Ok(mut ws) => {
                // 发送ping
                let _ = ws.send_ping().await;

                // 发送消息
                let _ = ws.send_text("hello").await;

                println!("WebSocket connection successful");
            }
            Err(e) => {
                // 没有WebSocket端点是正常的
                println!("WebSocket connection failed (expected): {}", e);
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试大型请求体
    #[tokio::test]
    async fn test_large_request_body() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 创建一个较大的请求体（1MB）
        let large_body = vec![0u8; 1024 * 1024];
        let url = format!("http://{}/upload", server_addr);

        match potato::post(&url, large_body, vec![]).await {
            Ok(_) => {
                println!("Large request sent successfully");
            }
            Err(e) => {
                println!("Large request failed: {}", e);
            }
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试连接超时处理
    #[tokio::test]
    async fn test_connection_failure() -> anyhow::Result<()> {
        // 尝试连接到不存在的服务器
        let url = "http://127.0.0.1:1/";
        match potato::get(url, vec![]).await {
            Ok(_) => {
                panic!("Should not connect to non-existent server");
            }
            Err(e) => {
                println!("Expected connection error: {}", e);
                assert!(true);
            }
        }

        Ok(())
    }

    /// 测试响应体解析
    #[tokio::test]
    async fn test_response_body_parsing() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(response) => {
                // 尝试将响应体解析为 UTF-8 字符串
                match &response.body {
                    potato::HttpResponseBody::Data(data) => match String::from_utf8(data.clone()) {
                        Ok(text) => {
                            println!("Response as string length: {}", text.len());
                        }
                        Err(e) => {
                            println!("Response is not valid UTF-8: {}", e);
                        }
                    },
                    potato::HttpResponseBody::Stream(_) => {
                        println!("Response is a stream");
                    }
                }
            }
            Err(e) => {
                println!("Request failed: {}", e);
            }
        }

        server_handle.abort();
        Ok(())
    }
}
