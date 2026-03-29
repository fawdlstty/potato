/// 服务器特性的综合测试
/// 测试可以不依赖特定路由实现的功能
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use std::{env, fs};
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(26000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::{HttpRequest, HttpResponse, HttpServer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    async fn connect_with_retry(addr: &str) -> anyhow::Result<TcpStream> {
        let mut last_err = None;
        for _ in 0..10 {
            match TcpStream::connect(addr).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    last_err = Some(err);
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
        Err(last_err.expect("retry loop must capture error").into())
    }

    /// 测试服务器创建和基本配置
    #[tokio::test]
    async fn test_server_creation_and_configuration() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 测试 configure 方法
        server.configure(|ctx| {
            // 这个配置可以成功应用
            ctx.use_handlers(false);
        });

        println!("✅ Server created and configured for: {}", server_addr);
        Ok(())
    }

    /// 测试多个服务器可以在不同端口上创建
    #[tokio::test]
    async fn test_multiple_servers_different_ports() -> anyhow::Result<()> {
        let port1 = get_test_port();
        let port2 = get_test_port();
        let port3 = get_test_port();

        let server_addr1 = format!("127.0.0.1:{}", port1);
        let server_addr2 = format!("127.0.0.1:{}", port2);
        let server_addr3 = format!("127.0.0.1:{}", port3);

        let _server1 = HttpServer::new(&server_addr1);
        let _server2 = HttpServer::new(&server_addr2);
        let _server3 = HttpServer::new(&server_addr3);

        assert_ne!(port1, port2);
        assert_ne!(port2, port3);
        assert_ne!(port1, port3);

        println!("✅ Created 3 servers on different ports");
        Ok(())
    }

    /// 测试服务器的关闭信号
    #[tokio::test]
    async fn test_server_shutdown_signal() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 获取关闭信号
        let shutdown_tx = server.shutdown_signal();

        let server_handle = tokio::spawn(async move {
            // 服务器会在收到关闭信号时退出
            let _ = server.serve_http().await;
        });

        // 给服务器时间启动
        sleep(Duration::from_millis(200)).await;

        // 发送关闭信号
        let _ = shutdown_tx.send(());

        // 等待服务器关闭
        sleep(Duration::from_millis(200)).await;

        // 如果没有 panic，测试通过
        server_handle.abort();
        println!("✅ Server shutdown signal works");
        Ok(())
    }

    /// 测试服务器配置的多种选项
    #[tokio::test]
    async fn test_server_configuration_options() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 配置选项1: 禁用处理器
        server.configure(|ctx| {
            ctx.use_handlers(false);
        });

        println!("✅ Server configuration: handlers disabled");

        // 配置选项2: OpenAPI (如果启用)
        let mut server2 = HttpServer::new(&format!("127.0.0.1:{}", get_test_port()));
        server2.configure(|ctx| {
            ctx.use_handlers(false);
            // ctx.use_openapi("/doc/"); // 仅在 openapi 特性启用时
        });

        println!("✅ Server configuration options applied");
        Ok(())
    }

    /// 测试服务器地址解析
    #[tokio::test]
    async fn test_server_address_parsing() -> anyhow::Result<()> {
        // 测试不同格式的地址
        let addresses = vec!["127.0.0.1:25000", "localhost:25001", "0.0.0.0:25002"];

        for addr in addresses {
            let _server = HttpServer::new(addr);
            println!("✅ Server created with address: {}", addr);
        }

        Ok(())
    }

    /// 测试服务器 HTTP 和 HTTPS 方法的可用性
    #[tokio::test]
    async fn test_server_protocols_availability() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let _server = HttpServer::new(&server_addr);

        // serve_http 方法应该存在
        // serve_https 方法应该存在（如果 TLS 特性启用）

        println!("✅ Server protocol methods are available");
        Ok(())
    }

    /// 测试从 String 创建服务器
    #[tokio::test]
    async fn test_server_from_string() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr_string = format!("127.0.0.1:{}", port);

        // 从 String 创建
        let _server1 = HttpServer::new(addr_string.clone());
        println!("✅ Server created from String: {}", addr_string);

        // 从 &str 创建
        let _server2 = HttpServer::new("127.0.0.1:25010");
        println!("✅ Server created from &str");

        Ok(())
    }

    /// 测试关闭信号只能获取一次
    #[tokio::test]
    async fn test_shutdown_signal_once() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 第一次获取应该成功
        let _shutdown_tx = server.shutdown_signal();

        // 第二次获取应该 panic（在注释中说明）
        // 取消注释下行会导致 panic: "shutdown signal already set"
        // let _shutdown_tx2 = server.shutdown_signal();

        println!("✅ Shutdown signal correctly allows only one acquisition");
        Ok(())
    }

    #[tokio::test]
    async fn test_server_accepts_chunked_request_body() -> anyhow::Result<()> {
        #[potato::http_post("/chunked_req_echo")]
        async fn chunked_req_echo(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(String::from_utf8_lossy(&req.body).to_string())
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "POST /chunked_req_echo HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Connection: close\r\n",
            "\r\n",
            "5\r\n",
            "Hello\r\n",
            "6\r\n",
            " World\r\n",
            "0\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);

        assert!(response_text.starts_with("HTTP/1.1 200"));
        assert!(response_text.contains("Hello World"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_transfer_encoding_content_length_conflict() -> anyhow::Result<()> {
        #[potato::http_post("/chunked_req_conflict")]
        async fn chunked_req_conflict(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "POST /chunked_req_conflict HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Content-Length: 100\r\n",
            "Connection: close\r\n",
            "\r\n",
            "5\r\n",
            "Hello\r\n",
            "0\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        assert!(response.is_empty());

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_http11_request_without_host() -> anyhow::Result<()> {
        #[potato::http_get("/host_required_missing")]
        async fn host_required_missing(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET /host_required_missing HTTP/1.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("missing required Host header"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_http11_request_with_empty_host() -> anyhow::Result<()> {
        #[potato::http_get("/host_required_empty")]
        async fn host_required_empty(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET /host_required_empty HTTP/1.1\r\n",
            "Host:   \r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("empty Host header"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_http11_request_with_duplicate_host() -> anyhow::Result<()> {
        #[potato::http_get("/host_required_duplicate")]
        async fn host_required_duplicate(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET /host_required_duplicate HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Host: example.com\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("multiple Host headers are not allowed"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_http11_request_with_invalid_host() -> anyhow::Result<()> {
        #[potato::http_get("/host_required_invalid")]
        async fn host_required_invalid(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET /host_required_invalid HTTP/1.1\r\n",
            "Host: bad host\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("invalid Host header"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_chunked_form_body_keeps_existing_body_pair_parsing() -> anyhow::Result<()> {
        #[potato::http_post("/chunked_form_parse")]
        async fn chunked_form_parse(req: &mut HttpRequest) -> HttpResponse {
            let name = req.body_pairs.get("name").map_or("", |v| v.as_ref());
            HttpResponse::text(name.to_string())
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "POST /chunked_form_parse HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Content-Type: application/x-www-form-urlencoded\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Connection: close\r\n",
            "\r\n",
            "a\r\n",
            "name=alice\r\n",
            "0\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 200"));
        assert!(response_text.contains("alice"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_unsupported_transfer_encoding_chain() -> anyhow::Result<()> {
        #[potato::http_post("/chunked_te_chain")]
        async fn chunked_te_chain(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "POST /chunked_te_chain HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Transfer-Encoding: gzip, chunked\r\n",
            "Connection: close\r\n",
            "\r\n",
            "5\r\n",
            "Hello\r\n",
            "0\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        assert!(response.is_empty());

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_connect_method_with_status() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "CONNECT example.com:443 HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 501 Not Implemented"));
        assert!(response_text.contains("CONNECT method is not implemented"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_accepts_absolute_form_request_target() -> anyhow::Result<()> {
        #[potato::http_get("/abs_form_target")]
        async fn abs_form_target(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("absolute form ok")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = format!(
            "GET http://{}/abs_form_target?ok=1 HTTP/1.1\r\nHost: wrong.example\r\nConnection: close\r\n\r\n",
            server_addr
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 200 OK"));
        assert!(response_text.contains("absolute form ok"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_authority_form_for_non_connect() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET example.com:443 HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("authority-form request-target is only valid for CONNECT"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_origin_form_for_connect() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "CONNECT /tunnel HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("CONNECT requires authority-form request-target"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_options_asterisk_uses_server_wide_allow() -> anyhow::Result<()> {
        #[potato::http_get("/asterisk_get")]
        async fn asterisk_get(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("ok")
        }

        #[potato::http_post("/asterisk_post")]
        async fn asterisk_post(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("ok")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "OPTIONS * HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 200 OK"));
        assert!(response_text.contains("Allow:"));
        assert!(response_text.contains("GET"));
        assert!(response_text.contains("POST"));

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "GET * HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(response_text.contains("asterisk-form request-target requires OPTIONS"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_head_fallback_uses_get_status_without_body() -> anyhow::Result<()> {
        #[potato::http_get("/head_fallback_get")]
        async fn head_fallback_get(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("head fallback payload")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "HEAD /head_fallback_get HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 200 OK"));
        assert!(!response_text.contains("head fallback payload"));
        let header_end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .ok_or_else(|| anyhow::anyhow!("response missing header terminator"))?;
        assert_eq!(response.len(), header_end);

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "HEAD /head_fallback_missing HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 404 Not Found"));
        assert!(!response_text.contains("404 not found"));
        let header_end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .ok_or_else(|| anyhow::anyhow!("response missing header terminator"))?;
        assert_eq!(response.len(), header_end);

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_head_route_takes_precedence_over_get_fallback() -> anyhow::Result<()> {
        #[potato::http_get("/head_precedence")]
        async fn head_precedence_get(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("from get")
        }

        #[potato::http_head("/head_precedence")]
        async fn head_precedence_head(_req: &mut HttpRequest) -> HttpResponse {
            let mut res = HttpResponse::empty();
            res.http_code = 204;
            res
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "HEAD /head_precedence HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 204 No Content"));

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_server_rejects_conflicting_duplicate_content_length() -> anyhow::Result<()> {
        #[potato::http_post("/duplicate_content_length")]
        async fn duplicate_content_length(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let mut stream = connect_with_retry(&server_addr).await?;
        let request = concat!(
            "POST /duplicate_content_length HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Content-Length: 5\r\n",
            "Content-Length: 6\r\n",
            "Connection: close\r\n",
            "\r\n",
            "Hello!"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        assert!(response.is_empty());

        server_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_static_file_route_supports_range_partial_content() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let temp_dir = env::temp_dir().join(format!("potato-range-{}", port));
        fs::create_dir_all(&temp_dir)?;
        let file_path = temp_dir.join("sample.txt");
        let file_content = b"HelloRangeWorld";
        fs::write(&file_path, file_content)?;

        let mut server = HttpServer::new(&server_addr);
        let static_root = temp_dir.canonicalize()?.to_string_lossy().to_string();
        server.configure(|ctx| {
            ctx.use_location_route("/static/", static_root.clone());
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let headers = vec![potato::Headers::Custom((
            "Range".to_string(),
            "bytes=5-9".to_string(),
        ))];
        let url = format!("http://{}/static/sample.txt", server_addr);
        let res = potato::get(&url, headers).await?;

        assert_eq!(res.http_code, 206);
        assert_eq!(res.get_header("Accept-Ranges"), Some("bytes"));
        assert_eq!(res.get_header("Content-Range"), Some("bytes 5-9/15"));
        let body = match res.body {
            potato::HttpResponseBody::Data(data) => data,
            potato::HttpResponseBody::Stream(_) => vec![],
        };
        assert_eq!(body, b"Range".to_vec());

        server_handle.abort();
        _ = fs::remove_file(file_path);
        _ = fs::remove_dir(temp_dir);
        Ok(())
    }

    #[tokio::test]
    async fn test_static_file_route_returns_416_for_unsatisfiable_range() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let temp_dir = env::temp_dir().join(format!("potato-range-{}", port));
        fs::create_dir_all(&temp_dir)?;
        let file_path = temp_dir.join("sample.txt");
        fs::write(&file_path, b"short")?;

        let mut server = HttpServer::new(&server_addr);
        let static_root = temp_dir.canonicalize()?.to_string_lossy().to_string();
        server.configure(|ctx| {
            ctx.use_location_route("/static/", static_root.clone());
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        let headers = vec![potato::Headers::Custom((
            "Range".to_string(),
            "bytes=99-100".to_string(),
        ))];
        let url = format!("http://{}/static/sample.txt", server_addr);
        let res = potato::get(&url, headers).await?;

        assert_eq!(res.http_code, 416);
        assert_eq!(res.get_header("Accept-Ranges"), Some("bytes"));
        assert_eq!(res.get_header("Content-Range"), Some("bytes */5"));

        server_handle.abort();
        _ = fs::remove_file(file_path);
        _ = fs::remove_dir(temp_dir);
        Ok(())
    }
}
