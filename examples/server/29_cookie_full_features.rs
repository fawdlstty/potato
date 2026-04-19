/// SessionCache 完整Cookie功能示例
/// 展示如何使用CookieBuilder配置完整的cookie属性

use potato::{HttpResponse, HttpServer, SessionCache, CookieBuilder};
use chrono::Utc;

// 简单cookie示例
#[potato::http_get("/cookie/simple")]
async fn cookie_simple(cache: &mut SessionCache) -> HttpResponse {
    // 读取请求cookie
    let session_id = cache.get_cookie("session_id");
    let theme = cache.get_cookie("theme");
    
    // 设置简单cookie
    cache.set_cookie("last_visit", &Utc::now().timestamp().to_string());
    
    HttpResponse::json(serde_json::json!({
        "session_id": session_id,
        "theme": theme,
        "message": "Simple cookie example"
    }))
}

// 完整属性cookie示例
#[potato::http_get("/cookie/full")]
async fn cookie_full(cache: &mut SessionCache) -> HttpResponse {
    let expires = Utc::now().timestamp() + 3600; // 1小时后过期
    
    // 使用CookieBuilder创建完整配置的cookie
    let auth_cookie = CookieBuilder::new("auth_token", "secure_token_123")
        .path("/api")                    // 仅在/api路径下有效
        .domain(".example.com")          // 域名
        .expires(expires)                // 过期时间
        .max_age(3600)                   // 最大存活时间（秒）
        .secure(true)                    // 仅HTTPS传输
        .http_only(true)                 // 禁止JavaScript访问
        .same_site("Strict");            // 严格的SameSite策略
    
    cache.set_cookie_with_builder(auth_cookie);
    
    // 设置另一个cookie用于偏好设置
    let pref_cookie = CookieBuilder::new("user_prefs", "dark_mode")
        .path("/")
        .max_age(86400 * 30)             // 30天
        .same_site("Lax");               // 宽松的SameSite策略
    
    cache.set_cookie_with_builder(pref_cookie);
    
    HttpResponse::json(serde_json::json!({
        "message": "Full attribute cookies set",
        "auth_cookie": "auth_token (Secure, HttpOnly, SameSite=Strict)",
        "pref_cookie": "user_prefs (SameSite=Lax, 30 days)"
    }))
}

// 删除cookie示例
#[potato::http_get("/cookie/delete")]
async fn cookie_delete(cache: &mut SessionCache) -> HttpResponse {
    // 简单删除
    cache.remove_cookie("session_id");
    
    // 带域名删除（如果cookie设置了域名，删除时也需要指定）
    cache.remove_cookie_with_domain("auth_token", ".example.com");
    
    HttpResponse::json(serde_json::json!({
        "message": "Cookies deleted",
        "deleted": ["session_id", "auth_token"]
    }))
}

// 安全cookie示例（用于敏感信息）
#[potato::http_get("/cookie/secure")]
async fn cookie_secure(cache: &mut SessionCache) -> HttpResponse {
    let secure_cookie = CookieBuilder::new("csrf_token", "random_token_value")
        .path("/")
        .secure(true)                    // 必须HTTPS
        .http_only(false)                // 允许JS访问（CSRF token需要被JS读取）
        .same_site("Strict");            // 严格模式防止CSRF
    
    cache.set_cookie_with_builder(secure_cookie);
    
    HttpResponse::json(serde_json::json!({
        "message": "Secure CSRF token set",
        "properties": {
            "secure": true,
            "http_only": false,
            "same_site": "Strict"
        }
    }))
}

// 跟踪cookie示例（用于分析）
#[potato::http_get("/cookie/tracking")]
async fn cookie_tracking(cache: &mut SessionCache) -> HttpResponse {
    let tracking_cookie = CookieBuilder::new("tracking_id", "analytics_123")
        .path("/")
        .max_age(86400 * 365)            // 1年
        .secure(true)
        .http_only(true)
        .same_site("None");              // None允许跨站发送（用于分析）
    
    cache.set_cookie_with_builder(tracking_cookie);
    
    HttpResponse::json(serde_json::json!({
        "message": "Tracking cookie set",
        "properties": {
            "max_age": "1 year",
            "same_site": "None (allows cross-site)"
        }
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!("\nCookie API Endpoints:");
    println!("  GET /cookie/simple   - Simple cookie example");
    println!("  GET /cookie/full     - Full attribute cookie example");
    println!("  GET /cookie/delete   - Delete cookies example");
    println!("  GET /cookie/secure   - Secure cookie (CSRF) example");
    println!("  GET /cookie/tracking - Tracking cookie example");
    println!("\nExample usage:");
    println!("  curl http://127.0.0.1:8080/cookie/simple");
    println!("  curl -H 'Cookie: session_id=abc123' http://127.0.0.1:8080/cookie/full");
    println!("\nCookieBuilder supports:");
    println!("  - path: Cookie path");
    println!("  - domain: Cookie domain");
    println!("  - expires: Expiration timestamp");
    println!("  - max_age: Max age in seconds");
    println!("  - secure: HTTPS only flag");
    println!("  - http_only: Prevent JavaScript access");
    println!("  - same_site: Strict/Lax/None policy");
    
    server.serve_http().await?;
    Ok(())
}
