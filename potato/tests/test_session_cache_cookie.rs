/// SessionCache Cookie 功能测试
use potato::{HttpRequest, HttpResponse, HttpServer, SessionCache};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_session_cache_cookie_basic() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static HANDLER_CALLED: AtomicUsize = AtomicUsize::new(0);

    #[potato::http_get("/cookie/test")]
    async fn cookie_handler(_req: &mut HttpRequest, cache: &mut SessionCache) -> HttpResponse {
        HANDLER_CALLED.fetch_add(1, Ordering::Relaxed);

        // 读取请求中的cookie
        let session_id = cache.get_cookie("session_id");

        // 设置响应cookie
        cache.set_cookie("user_token", "abc123");
        cache.set_cookie("visited", "true");

        HttpResponse::text(format!("session_id: {:?}", session_id))
    }

    // 设置JWT密钥
    SessionCache::set_jwt_secret(b"test-secret-key").await;

    // 生成一个测试 token
    let test_token = SessionCache::generate_token(12345, Duration::from_secs(3600)).await?;

    let port = 18100;
    let server_addr = format!("127.0.0.1:{port}");
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    // 使用 potato HTTP 客户端测试
    let url = format!("http://{}/cookie/test", server_addr);

    let response = potato::get!(
        &url,
        Authorization = format!("Bearer {}", test_token),
        Custom("Cookie") = "session_id=xyz789; other_cookie=value"
    )
    .await?;

    let response_text = match &response.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };

    println!("Response: {}", response_text);

    // 验证响应内容
    assert!(response_text.contains("session_id: Some(\"xyz789\")"));

    assert_eq!(HANDLER_CALLED.load(Ordering::Relaxed), 1);
    println!("✅ SessionCache cookie basic test passed");

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_session_cache_cookie_remove() -> anyhow::Result<()> {
    #[potato::http_get("/cookie/remove")]
    async fn cookie_remove_handler(cache: &mut SessionCache) -> HttpResponse {
        // 移除cookie
        cache.remove_cookie("old_session");

        HttpResponse::text("cookie removed")
    }

    SessionCache::set_jwt_secret(b"test-secret-key").await;
    let test_token = SessionCache::generate_token(12345, Duration::from_secs(3600)).await?;

    let port = 18101;
    let server_addr = format!("127.0.0.1:{port}");
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    // 使用 potato HTTP 客户端测试
    let url = format!("http://{}/cookie/remove", server_addr);

    let response = potato::get!(&url, Authorization = format!("Bearer {}", test_token)).await?;

    let response_text = match &response.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };

    println!("Response: {}", response_text);

    // 验证响应成功
    assert!(response_text.contains("cookie removed"));

    println!("✅ SessionCache cookie remove test passed");

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_session_cache_cookie_no_cookie_header() -> anyhow::Result<()> {
    #[potato::http_get("/cookie/no_cookie")]
    async fn no_cookie_handler(cache: &mut SessionCache) -> HttpResponse {
        // 尝试读取不存在的cookie
        let token = cache.get_cookie("token");
        assert!(token.is_none());

        // 设置新cookie
        cache.set_cookie("new_token", "value123");

        HttpResponse::text("ok")
    }

    SessionCache::set_jwt_secret(b"test-secret-key").await;
    let test_token = SessionCache::generate_token(12345, Duration::from_secs(3600)).await?;

    let port = 18102;
    let server_addr = format!("127.0.0.1:{port}");
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    // 使用 potato HTTP 客户端测试
    let url = format!("http://{}/cookie/no_cookie", server_addr);

    let response = potato::get!(&url, Authorization = format!("Bearer {}", test_token)).await?;

    let response_text = match &response.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8(data.clone())?,
        _ => panic!("Expected data body"),
    };

    // 验证没有panic，并且响应成功
    assert!(response_text.contains("ok"));

    println!("✅ SessionCache cookie no cookie header test passed");

    server_handle.abort();
    Ok(())
}
