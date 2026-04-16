/// OnceCache 使用示例
/// 展示如何在 preprocess、handler 和 postprocess 之间传递数据

use potato::{HttpRequest, HttpResponse, HttpServer, OnceCache};

// 1. 在 preprocess 中设置缓存数据
#[potato::preprocess]
async fn auth_preprocess(req: &mut HttpRequest, cache: &mut OnceCache) {
    // 模拟从请求中提取用户信息并缓存
    if let Some(auth_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
        let token = auth_header.to_str();
        // 在实际应用中,这里会验证 token 并提取用户信息
        cache.set("user_id", 12345u32);
        cache.set("username", "john_doe".to_string());
        cache.set("role", "admin".to_string());
    }
}

// 2. 在 handler 中使用缓存数据
#[potato::http_get("/api/profile")]
#[potato::preprocess(auth_preprocess)]
async fn get_profile(cache: &mut OnceCache) -> HttpResponse {
    // 直接从缓存中获取用户信息,无需重复解析
    // 正确使用方式：处理 Option 返回值
    let user_id = cache.get::<u32>("user_id").copied().unwrap_or(0);
    let username = cache.get::<String>("username").cloned().unwrap_or_default();
    let role = cache.get::<String>("role").cloned().unwrap_or_default();
    
    // 也可以使用 contains_key 检查
    if cache.contains_key::<u32>("user_id") {
        println!("User ID is cached");
    }
    
    HttpResponse::text(format!("Profile: {} (ID: {}, Role: {})", username, user_id, role))
}

// 3. 在 postprocess 中读取和修改缓存
#[potato::postprocess]
fn log_postprocess(_req: &mut HttpRequest, res: &mut HttpResponse, cache: &mut OnceCache) {
    // 记录处理信息到缓存
    cache.set("response_size", match &res.body {
        potato::HttpResponseBody::Data(data) => data.len(),
        potato::HttpResponseBody::Stream(_) => 0,
    });
    
    // 可以基于缓存数据修改响应
    let user_id: u32 = *cache.get::<u32>("user_id").expect("user_id not found");
    println!("Request processed for user {}", user_id);
}

// 4. 在 handler 和 postprocess 之间传递数据
#[potato::http_get("/api/data")]
#[potato::postprocess(log_postprocess)]
async fn get_data(req: &mut HttpRequest, cache: &mut OnceCache) -> HttpResponse {
    // handler 设置一些处理数据
    cache.set("processing_time_ms", 100u32);
    cache.set("data_source", "database".to_string());
    
    HttpResponse::text("Data response")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    server.serve_http().await?;
    Ok(())
}
