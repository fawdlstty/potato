/// SessionCache Cookie 使用示例
/// 展示如何使用 SessionCache 的 cookie 功能
/// 用户调用 get_cookie 自动读取请求的 Cookie header
/// 用户调用 set_cookie/remove_cookie 自动设置响应的 Set-Cookie header

use potato::{HttpRequest, HttpResponse, HttpServer, SessionCache};

// 读取请求cookie并设置响应cookie
#[potato::http_get("/cookie/demo")]
async fn cookie_demo(cache: &mut SessionCache) -> HttpResponse {
    // 自动从请求的 Cookie header 中读取
    let session_id = cache.get_cookie("session_id");
    let user_pref = cache.get_cookie("user_pref");
    
    // 自动设置到响应的 Set-Cookie header
    cache.set_cookie("last_visit", "2024-01-01");
    cache.set_cookie("theme", "dark");
    
    HttpResponse::json(serde_json::json!({
        "session_id": session_id,
        "user_pref": user_pref,
        "message": "Cookies set successfully"
    }))
}

// 移除cookie示例
#[potato::http_get("/cookie/logout")]
async fn cookie_logout(cache: &mut SessionCache) -> HttpResponse {
    // 移除cookie（设置过期时间为过去）
    cache.remove_cookie("session_id");
    cache.remove_cookie("user_token");
    
    HttpResponse::text("Logged out, cookies removed")
}

// 同时使用SessionCache数据和Cookie
#[potato::http_get("/cookie/combined")]
async fn cookie_combined(cache: &mut SessionCache) -> HttpResponse {
    // 使用SessionCache存储会话数据
    let visit_count: u32 = cache.get("visit_count").unwrap_or(0);
    cache.set("visit_count", visit_count + 1);
    
    // 使用Cookie存储客户端数据
    let theme = cache.get_cookie("theme").unwrap_or("light".to_string());
    cache.set_cookie("last_count", &visit_count.to_string());
    
    HttpResponse::json(serde_json::json!({
        "visit_count": visit_count + 1,
        "theme": theme,
        "message": "Combined session and cookie data"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!("\nCookie API Endpoints:");
    println!("  GET /cookie/demo      - Read and set cookies");
    println!("  GET /cookie/logout    - Remove cookies");
    println!("  GET /cookie/combined  - Combine session data and cookies");
    println!("\nExample usage:");
    println!("  curl -H 'Cookie: session_id=abc123' http://127.0.0.1:8080/cookie/demo");
    println!("\nNote: Just use cache.get_cookie/set_cookie/remove_cookie,");
    println!("      the macro will automatically handle request/response cookies!");
    
    server.serve_http().await?;
    Ok(())
}
