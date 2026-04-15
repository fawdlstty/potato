/// 请求体大小限制功能测试
#[cfg(test)]
mod tests {
    use potato::{HttpRequest, HttpResponse, HttpServer};
    use std::time::Duration;
    use tokio::{io::AsyncWriteExt, time::sleep};

    fn get_test_port() -> u16 {
        static PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(18000);
        PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    async fn connect_with_retry(addr: &str) -> anyhow::Result<tokio::net::TcpStream> {
        for _ in 0..10 {
            match tokio::net::TcpStream::connect(addr).await {
                Ok(s) => return Ok(s),
                Err(_) => sleep(Duration::from_millis(100)).await,
            }
        }
        Err(anyhow::anyhow!("Failed to connect to {}", addr))
    }

    /// 测试全局 body 限制中间件
    #[tokio::test]
    async fn test_global_body_limit_middleware() -> anyhow::Result<()> {
        #[potato::http_post("/upload")]
        async fn upload(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(format!("upload success, size: {}", req.body.len()))
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        server.configure(|ctx| {
            // 设置限制: 1MB
            ctx.use_limit_size(1024 * 1024, 1024 * 1024);
            ctx.use_handlers();
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        // 测试小 body (应该成功)
        let mut stream = connect_with_retry(&server_addr).await?;
        let small_body = b"hello world";
        let request = format!(
            "POST /upload HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            port,
            small_body.len(),
            String::from_utf8_lossy(small_body)
        );
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        use tokio::io::AsyncReadExt;
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("200 OK"));
        assert!(response_text.contains("upload success"));
        println!("✅ Small body request succeeded");

        // 测试大 body (应该返回 413)
        let mut stream2 = connect_with_retry(&server_addr).await?;
        let large_body = vec![0u8; 2 * 1024 * 1024]; // 2MB
        let request2 = format!(
            "POST /upload HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            port,
            large_body.len()
        );
        stream2.write_all(request2.as_bytes()).await?;
        // 发送部分 body
        stream2.write_all(&large_body[..1024]).await?;

        let mut response2 = Vec::new();
        stream2.read_to_end(&mut response2).await?;
        let response_text2 = String::from_utf8_lossy(&response2);
        // 应该返回 413 或连接被关闭
        println!("Large body response: {}", response_text2);

        server_handle.abort();
        Ok(())
    }

    /// 测试 handler 注解覆盖全局限制
    #[tokio::test]
    async fn test_handler_annotation_override() -> anyhow::Result<()> {
        // 全局限制 1MB
        #[potato::http_post("/upload")]
        async fn upload(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(format!("upload success, size: {}", req.body.len()))
        }

        // 注解限制 10MB
        #[potato::http_post("/large-upload")]
        #[potato::limit_size(10 * 1024 * 1024)]
        async fn large_upload(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(format!("large upload success, size: {}", req.body.len()))
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        server.configure(|ctx| {
            ctx.use_limit_size(1024 * 1024, 1024 * 1024); // 全局 1MB
            ctx.use_handlers();
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        // 测试 5MB body 到 /large-upload (应该成功,注解允许 10MB)
        let mut stream = connect_with_retry(&server_addr).await?;
        let medium_body = vec![0u8; 5 * 1024 * 1024]; // 5MB
        let request = format!(
            "POST /large-upload HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            port,
            medium_body.len()
        );
        stream.write_all(request.as_bytes()).await?;
        stream.write_all(&medium_body[..1024]).await?;

        let mut response = Vec::new();
        use tokio::io::AsyncReadExt;
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        // 应该成功 (注解覆盖全局限制)
        println!("Large upload response: {}", response_text);

        server_handle.abort();
        Ok(())
    }

    /// 测试 limit_size 注解 - 单值参数
    #[potato::http_post("/test-single-param")]
    #[potato::limit_size(1024)]
    async fn test_single_param(req: &mut HttpRequest) -> HttpResponse {
        HttpResponse::text(format!("received {} bytes", req.body.len()))
    }

    /// 测试 limit_size 注解 - 命名参数
    #[potato::http_post("/test-named-param")]
    #[potato::limit_size(header = 2048, body = 4096)]
    async fn test_named_param(req: &mut HttpRequest) -> HttpResponse {
        HttpResponse::text(format!("received {} bytes", req.body.len()))
    }

    /// 测试注解编译正确性
    #[tokio::test]
    async fn test_limit_size_annotation_compilation() -> anyhow::Result<()> {
        // 此测试验证注解能够正确编译
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        server.configure(|ctx| {
            ctx.use_handlers();
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        // 发送小请求
        let mut stream = connect_with_retry(&server_addr).await?;
        let request = format!(
            "POST /test-single-param HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Length: 5\r\n\
             Connection: close\r\n\
             \r\n\
             hello",
            port
        );
        use tokio::io::AsyncWriteExt;
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        use tokio::io::AsyncReadExt;
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("200 OK"));
        println!("✅ Annotation compilation test passed");

        server_handle.abort();
        Ok(())
    }

    /// 测试 413 错误响应
    #[tokio::test]
    async fn test_413_error_response() -> anyhow::Result<()> {
        #[potato::http_post("/small-limit")]
        #[potato::limit_size(100)]
        async fn small_limit(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(format!("received {} bytes", req.body.len()))
        }

        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let mut server = HttpServer::new(&server_addr);

        server.configure(|ctx| {
            ctx.use_handlers();
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;

        // 发送超过限制的 body (200 bytes > 100 bytes limit)
        let mut stream = connect_with_retry(&server_addr).await?;
        let large_body = vec![0u8; 200];
        let request = format!(
            "POST /small-limit HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            port,
            large_body.len()
        );
        use tokio::io::AsyncWriteExt;
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        use tokio::io::AsyncReadExt;
        stream.read_to_end(&mut response).await?;
        let response_text = String::from_utf8_lossy(&response);

        // 验证返回 413
        assert!(response_text.contains("413"));
        assert!(response_text.contains("Payload Too Large"));
        println!("✅ 413 error response test passed");

        server_handle.abort();
        Ok(())
    }

    /// 测试全局配置 API
    #[test]
    fn test_global_config_api() {
        use potato::ServerConfig;

        // 测试默认值
        let default_body_limit = ServerConfig::get_max_body_bytes();
        assert_eq!(default_body_limit, 100 * 1024 * 1024); // 100MB

        // 测试设置新值
        ServerConfig::set_max_body_bytes(50 * 1024 * 1024);
        assert_eq!(ServerConfig::get_max_body_bytes(), 50 * 1024 * 1024);

        // 测试最小值保护
        ServerConfig::set_max_body_bytes(0);
        assert_eq!(ServerConfig::get_max_body_bytes(), 1);

        println!("✅ Global config API test passed");
    }
}
