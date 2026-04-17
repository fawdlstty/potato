/// SessionCache Cookie 功能简单测试
use potato::SessionCache;

#[test]
fn test_session_cache_cookie_methods() {
    // 创建SessionCache实例
    let cache = SessionCache::new();

    // 测试set_cookie和get_cookie
    cache.set_cookie("user_token", "abc123");
    cache.set_cookie("theme", "dark");

    // 注意：get_cookie读取的是请求cookie，这里没有请求所以返回None
    let token = cache.get_cookie("user_token");
    assert!(token.is_none()); // 因为没有解析请求cookie

    // 测试remove_cookie
    cache.remove_cookie("old_session");

    println!("✅ SessionCache cookie methods test passed");
}

#[test]
fn test_session_cache_cookie_parse_and_apply() {
    use potato::HttpResponse;

    let mut cache = SessionCache::new();

    // 模拟解析请求cookie
    cache.parse_request_cookies("session_id=xyz789; user_pref=light");

    // 验证可以读取到cookie
    let session_id = cache.get_cookie("session_id");
    assert_eq!(session_id, Some("xyz789".to_string()));

    let user_pref = cache.get_cookie("user_pref");
    assert_eq!(user_pref, Some("light".to_string()));

    // 设置响应cookie
    cache.set_cookie("new_token", "value123");
    cache.remove_cookie("old_cookie");

    // 创建HttpResponse并应用cookies
    let mut response = HttpResponse::text("test");
    cache.apply_cookies(&mut response);

    // 验证response headers中包含Set-Cookie
    assert!(response.headers.contains_key("Set-Cookie"));

    println!("✅ SessionCache cookie parse and apply test passed");
}
