#![cfg(feature = "std")]
//! 安全加固回归测试：覆盖请求体上限、multipart 空值、响应头注入、
//! 重复 Transfer-Encoding、chunked 元数据行洪水与多 Set-Cookie。
#[cfg(test)]
mod tests {
    use potato::{HttpRequest, HttpResponse, HttpServer};
    use std::time::Duration;
    use tokio::{io::AsyncWriteExt, time::sleep};

    fn get_test_port() -> u16 {
        static PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(21000);
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

    async fn start_server(
        configure: impl Fn(&mut potato::PipeContext),
    ) -> anyhow::Result<(u16, tokio::task::JoinHandle<()>)> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");
        let mut server = HttpServer::new(&server_addr);
        let _ = server.configure(configure);
        let handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;
        Ok((port, handle))
    }

    /// P0-2: multipart 的 name= 空值不再 panic，请求正常处理
    #[tokio::test]
    async fn test_multipart_empty_name_does_not_panic() -> anyhow::Result<()> {
        #[potato::http_post("/form")]
        async fn form(req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text(format!("fields: {}", req.body_pairs.len()))
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        let body = "--x\r\nContent-Disposition: form-data; name=\r\n\r\nvalue\r\n--x--\r\n";
        let request = format!(
            "POST /form HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Type: multipart/form-data; boundary=x\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("200 OK"), "got: {response_text}");

        server_handle.abort();
        Ok(())
    }

    /// P0-1: HTTP/1.1 默认也有请求体上限（100MB），巨大的 Content-Length 直接 413
    #[tokio::test]
    async fn test_default_body_limit_returns_413() -> anyhow::Result<()> {
        #[potato::http_post("/upload")]
        async fn upload(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("ok")
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        let request = format!(
            "POST /upload HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            200 * 1024 * 1024
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("413"), "got: {response_text}");

        server_handle.abort();
        Ok(())
    }

    /// P1-2: 重复的 Transfer-Encoding 请求头被拒绝（请求走私向量）
    #[tokio::test]
    async fn test_duplicate_transfer_encoding_rejected() -> anyhow::Result<()> {
        #[potato::http_post("/te")]
        async fn te(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        let request = concat!(
            "POST /te HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Transfer-Encoding: identity\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Connection: close\r\n",
            "\r\n",
            "0\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("400"), "got: {response_text}");
        assert!(response_text.contains("Connection: close"));

        server_handle.abort();
        Ok(())
    }

    /// P2-1: 超长 chunk-size 行被拒绝（16KB 上限）
    #[tokio::test]
    async fn test_chunk_size_line_flood_rejected() -> anyhow::Result<()> {
        #[potato::http_post("/chunked")]
        async fn chunked(_req: &mut HttpRequest) -> HttpResponse {
            HttpResponse::text("should not reach")
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        // 发送没有 CRLF 的超长"chunk size 行"
        let flood = vec![b'a'; 32 * 1024];
        let request = format!(
            "POST /chunked HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Transfer-Encoding: chunked\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            String::from_utf8_lossy(&flood)
        );
        stream.write_all(request.as_bytes()).await?;
        // 服务端可能在我们还在发送时就关闭连接（RST），读取时容忍该错误
        let mut response = Vec::new();
        let mut tmp = [0u8; 4096];
        use tokio::io::AsyncReadExt;
        loop {
            match stream.read(&mut tmp).await {
                Ok(0) => break,
                Ok(n) => response.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        let response_text = String::from_utf8_lossy(&response);
        // 安全目标：连接被终止（服务端读到超长行即断开，可能来不及送出 400，
        // 未读完的洪水数据触发 RST 也会丢弃已写入的响应）。只要不是无限
        // 读入、不是 panic 即视为防护生效。
        assert!(
            response.is_empty() || response_text.contains("400"),
            "got: {response_text}"
        );

        server_handle.abort();
        Ok(())
    }

    /// P1-1: 响应头中的 CR/LF 被过滤，响应拆分被阻止
    #[tokio::test]
    async fn test_response_header_injection_sanitized() -> anyhow::Result<()> {
        #[potato::http_get("/inject")]
        async fn inject() -> HttpResponse {
            let mut res = HttpResponse::text("ok");
            res.add_header("X-Custom".into(), "value\r\nEvil: injected".into());
            res
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        let request = concat!(
            "GET /inject HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("200 OK"));
        // CR/LF 被过滤后，注入内容不再是独立的响应头
        assert!(
            !response_text.contains("\r\nEvil:"),
            "header injection not sanitized: {response_text}"
        );
        // 被清洗后的值仍然存在（去掉了控制字符）
        assert!(response_text.contains("X-Custom: valueEvil"));

        server_handle.abort();
        Ok(())
    }

    /// P1-6: 多个 Set-Cookie 不再互相覆盖
    #[tokio::test]
    async fn test_multiple_set_cookie_preserved() -> anyhow::Result<()> {
        #[potato::http_get("/cookies")]
        async fn cookies() -> HttpResponse {
            let sess = potato::SessionCache::new();
            sess.set_cookie("a", "1");
            sess.set_cookie("b", "2");
            let mut res = HttpResponse::text("ok");
            sess.apply_cookies(&mut res);
            res
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        let request = concat!(
            "GET /cookies HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.contains("200 OK"), "got: {response_text}");
        assert!(response_text.contains("a=1"), "cookie a missing: {response_text}");
        assert!(response_text.contains("b=2"), "cookie b missing: {response_text}");

        server_handle.abort();
        Ok(())
    }

    /// P1-5/P3-1/P3-2: 二进制文件上传可用；内容中出现的 boundary 文本
    /// （不在行首）不会误切；带引号的 boundary 正常解析
    #[tokio::test]
    async fn test_multipart_binary_and_boundary_in_content() -> anyhow::Result<()> {
        #[potato::http_post("/upload-bin")]
        async fn upload_bin(req: &mut HttpRequest) -> HttpResponse {
            match req.body_files.get("file") {
                Some(f) => HttpResponse::text(format!(
                    "file={} len={} first={:02x}",
                    f.filename,
                    f.data.len(),
                    f.data[0]
                )),
                None => HttpResponse::text("no file"),
            }
        }

        let (_port, server_handle) = start_server(|ctx| {
            ctx.use_handlers();
        })
        .await?;

        let mut stream = connect_with_retry(&format!("127.0.0.1:{}", _port)).await?;
        // 文件内容包含非 UTF-8 字节和行中的 "xxBOUND" 文本（非行首，不应切分）
        let mut content: Vec<u8> = vec![0xFF, 0xFE, b'A'];
        content.extend_from_slice(b"xx--BOUNDxx middle text");
        content.push(0x80);

        let mut body: Vec<u8> = b"--BOUND\r\n".to_vec();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\r\n",
        );
        body.extend_from_slice(&content);
        body.extend_from_slice(b"\r\n--BOUND--\r\n");

        let request = format!(
            "POST /upload-bin HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Type: multipart/form-data; boundary=\"BOUND\"\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        );
        stream.write_all(request.as_bytes()).await?;
        stream.write_all(&body).await?;
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await?;
        let response_text = String::from_utf8_lossy(&response);
        let expected = format!(
            "file=a.bin len={} first=ff",
            content.len()
        );
        assert!(
            response_text.contains(&expected),
            "got: {response_text}, expected: {expected}"
        );

        server_handle.abort();
        Ok(())
    }
}
