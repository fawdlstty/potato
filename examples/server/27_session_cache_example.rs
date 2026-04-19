/// SessionCache 使用示例
/// 展示如何在不同请求间使用 SessionCache 传递数据
/// 用户只需在函数参数中声明 `cache: &mut SessionCache`，宏系统自动处理token验证和session加载

use potato::{HttpRequest, HttpResponse, HttpServer, SessionCache};

// 用户登录接口，签发token
#[potato::http_post("/api/login")]
async fn login(req: &mut HttpRequest) -> HttpResponse {
    // 从请求中获取用户信息（简化示例）
    let user_id: i64 = req.body_pairs
        .get(&potato::hipstr::LocalHipStr::from("user_id"))
        .and_then(|v| v.to_string().parse().ok())
        .unwrap_or(12345);
    
    // 签发token（有效期1小时）
    match SessionCache::generate_token(user_id, std::time::Duration::from_secs(3600)).await {
        Ok(token) => HttpResponse::json(serde_json::json!({
            "token": token,
            "expires_in": 3600
        })),
        Err(e) => HttpResponse::error(format!("Failed to generate token: {e}")),
    }
}

// 获取用户资料 - 直接使用SessionCache参数，宏自动处理token验证
#[potato::http_get("/api/profile")]
async fn get_profile(cache: &mut SessionCache) -> HttpResponse {
    // 从session中获取或设置用户资料
    let profile = cache.with_get("profile", |p: &Option<serde_json::Value>| {
        p.clone().unwrap_or(serde_json::json!({
            "user_id": 12345,
            "name": "John Doe",
            "email": "john@example.com"
        }))
    });
    
    HttpResponse::json(profile)
}

// 更新用户资料 - 数据会自动保存在SessionCache中
#[potato::http_post("/api/profile")]
async fn update_profile(req: &mut HttpRequest, cache: &mut SessionCache) -> HttpResponse {
    // 解析请求体
    let new_profile: serde_json::Value = match serde_json::from_slice(&req.body) {
        Ok(val) => val,
        Err(e) => return HttpResponse::error(format!("Invalid JSON: {e}")),
    };
    
    // 保存到session
    cache.set("profile", new_profile.clone());
    
    HttpResponse::json(serde_json::json!({
        "message": "Profile updated",
        "profile": new_profile
    }))
}

// 获取请求计数 - 演示跨请求数据保持
#[potato::http_get("/api/request_count")]
async fn get_request_count(cache: &mut SessionCache) -> HttpResponse {
    // 增加请求计数
    let count: u32 = cache.get("request_count").unwrap_or(0);
    cache.set("request_count", count + 1);
    
    HttpResponse::json(serde_json::json!({
        "request_count": count + 1,
        "message": format!("This is your {} request", count + 1)
    }))
}

// 同时使用OnceCache和SessionCache
#[potato::http_get("/api/combined")]
#[potato::preprocess(auth_preprocess)]
async fn combined_handler(once_cache: &mut OnceCache, session_cache: &mut SessionCache) -> HttpResponse {
    // OnceCache用于单次请求内的数据传递
    let request_id: String = once_cache.get("request_id");
    
    // SessionCache用于跨请求数据保持
    let visit_count: u32 = session_cache.get("visit_count").unwrap_or(0);
    session_cache.set("visit_count", visit_count + 1);
    
    HttpResponse::json(serde_json::json!({
        "request_id": request_id,
        "visit_count": visit_count + 1,
        "message": "Combined cache example"
    }))
}

// 预处理示例：设置OnceCache数据
#[potato::preprocess]
async fn auth_preprocess(req: &mut HttpRequest, cache: &mut OnceCache) {
    // 生成请求ID用于跟踪
    let request_id = format!("req_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());
    cache.set("request_id", request_id);
}

// 登出接口 - 清理session数据并使token失效
#[potato::http_post("/api/logout")]
async fn logout(req: &mut HttpRequest, cache: &mut SessionCache) -> HttpResponse {
    // 从token中获取user_id并使session失效
    if let Some(auth_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
        let header_value = auth_header.to_str();
        if header_value.starts_with("Bearer ") {
            if let Ok((user_id, _)) = SessionCache::parse_token(&header_value[7..]).await {
                // 使session失效
                SessionCache::invalidate(user_id);
            }
        }
    }
    
    HttpResponse::text("Logged out successfully")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 设置JWT密钥（生产环境应该使用更安全的密钥）
    SessionCache::set_jwt_secret(b"your-secret-key-change-in-production").await;
    
    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!("\nAPI Endpoints:");
    println!("  POST /api/login          - Login and get token");
    println!("  GET  /api/profile        - Get user profile (requires token)");
    println!("  POST /api/profile        - Update user profile (requires token)");
    println!("  GET  /api/request_count  - Get request count (requires token)");
    println!("  GET  /api/combined       - Combined OnceCache + SessionCache (requires token)");
    println!("  POST /api/logout         - Logout (requires token)");
    println!("\nExample usage:");
    println!("  1. Login: curl -X POST http://127.0.0.1:8080/api/login -d 'user_id=12345'");
    println!("  2. Use token: curl -H 'Authorization: Bearer <token>' http://127.0.0.1:8080/api/profile");
    println!("\nNote: Just add `cache: &mut SessionCache` parameter to your handler,");
    println!("      the macro will automatically validate the token and load the session!");
    
    server.serve_http().await?;
    Ok(())
}
