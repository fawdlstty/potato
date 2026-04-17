use chrono::Utc;
/// SessionCache 完整Cookie功能测试
use potato::{CookieBuilder, HttpResponse, SessionCache};

#[test]
fn test_cookie_builder_basic() {
    // 测试基本的Cookie创建
    let cookie = CookieBuilder::new("session_id", "abc123");
    let cookie_str = cookie.to_set_cookie_string();

    assert!(cookie_str.contains("session_id=abc123"));
    assert!(cookie_str.contains("Path=/"));

    println!("✅ Cookie builder basic test passed");
    println!("Cookie: {}", cookie_str);
}

#[test]
fn test_cookie_builder_full_attributes() {
    // 测试完整属性的Cookie
    let expires_timestamp = Utc::now().timestamp() + 3600; // 1小时后过期

    let cookie = CookieBuilder::new("user_token", "xyz789")
        .path("/api")
        .domain(".example.com")
        .expires(expires_timestamp)
        .max_age(3600)
        .secure(true)
        .http_only(true)
        .same_site("Strict");

    let cookie_str = cookie.to_set_cookie_string();

    println!("Full attribute cookie: {}", cookie_str);

    assert!(cookie_str.contains("user_token=xyz789"));
    assert!(cookie_str.contains("Path=/api"));
    assert!(cookie_str.contains("Domain=.example.com"));
    assert!(cookie_str.contains("Max-Age=3600"));
    assert!(cookie_str.contains("Secure"));
    assert!(cookie_str.contains("HttpOnly"));
    assert!(cookie_str.contains("SameSite=Strict"));
    assert!(cookie_str.contains("Expires="));

    println!("✅ Cookie builder full attributes test passed");
}

#[test]
fn test_session_cache_cookie_with_builder() {
    let mut cache = SessionCache::new();

    // 解析请求cookie
    cache.parse_request_cookies("session_id=abc123; theme=dark");

    // 验证读取
    assert_eq!(cache.get_cookie("session_id"), Some("abc123".to_string()));
    assert_eq!(cache.get_cookie("theme"), Some("dark".to_string()));

    // 使用简单方法设置cookie
    cache.set_cookie("simple_cookie", "value1");

    // 使用builder设置完整cookie
    let full_cookie = CookieBuilder::new("full_cookie", "value2")
        .path("/admin")
        .secure(true)
        .http_only(true)
        .same_site("Lax");
    cache.set_cookie_with_builder(full_cookie);

    // 应用到response
    let mut response = HttpResponse::text("test");
    cache.apply_cookies(&mut response);

    // 验证response headers中包含Set-Cookie
    assert!(response.headers.contains_key("Set-Cookie"));

    // 打印所有Set-Cookie header
    if let Some(cookie_header) = response.headers.get("Set-Cookie") {
        println!("Set-Cookie headers: {}", cookie_header);
        // 验证包含我们设置的cookie
        assert!(
            cookie_header.contains("simple_cookie=value1")
                || cookie_header.contains("full_cookie=value2")
        );
    }

    println!("✅ SessionCache cookie with builder test passed");
}

#[test]
fn test_cookie_delete_with_domain() {
    let cache = SessionCache::new();

    // 删除带域名的cookie
    cache.remove_cookie_with_domain("old_session", ".example.com");

    let mut response = HttpResponse::text("test");
    cache.apply_cookies(&mut response);

    let set_cookie = response.headers.get("Set-Cookie");
    assert!(set_cookie.is_some());

    let cookie_str = set_cookie.unwrap();
    println!("Delete cookie: {}", cookie_str);

    assert!(cookie_str.contains("old_session="));
    assert!(cookie_str.contains("Domain=.example.com"));
    assert!(cookie_str.contains("Expires=Thu, 01 Jan 1970 00:00:00 GMT"));

    println!("✅ Cookie delete with domain test passed");
}

#[test]
fn test_cookie_secure_and_httponly() {
    // 测试安全相关的cookie属性
    let secure_cookie = CookieBuilder::new("secure_token", "secret")
        .secure(true)
        .http_only(true)
        .same_site("None");

    let cookie_str = secure_cookie.to_set_cookie_string();
    println!("Secure cookie: {}", cookie_str);

    assert!(cookie_str.contains("Secure"));
    assert!(cookie_str.contains("HttpOnly"));
    assert!(cookie_str.contains("SameSite=None"));

    println!("✅ Cookie secure and httponly test passed");
}

#[test]
fn test_cookie_max_age_vs_expires() {
    // 测试Max-Age和Expires的区别
    let expires_timestamp = Utc::now().timestamp() + 7200;

    let cookie_with_max_age = CookieBuilder::new("max_age_cookie", "value1").max_age(3600);

    let cookie_with_expires =
        CookieBuilder::new("expires_cookie", "value2").expires(expires_timestamp);

    let max_age_str = cookie_with_max_age.to_set_cookie_string();
    let expires_str = cookie_with_expires.to_set_cookie_string();

    println!("Max-Age cookie: {}", max_age_str);
    println!("Expires cookie: {}", expires_str);

    assert!(max_age_str.contains("Max-Age=3600"));
    assert!(!max_age_str.contains("Expires="));

    assert!(expires_str.contains("Expires="));
    assert!(!expires_str.contains("Max-Age="));

    println!("✅ Cookie max_age vs expires test passed");
}
