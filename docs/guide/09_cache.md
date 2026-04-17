# 缓存系统

Potato框架提供两种缓存系统：OnceCache（单次请求缓存）和SessionCache（会话级缓存）。

## OnceCache

用于单次HTTP请求生命周期内传递数据，在preprocess、handler和postprocess之间共享。

### 基本用法

```rust
#[potato::preprocess]
async fn auth_preprocess(req: &mut HttpRequest, cache: &mut OnceCache) {
    cache.set("user_id", 12345u32);
}

#[potato::http_get("/profile")]
#[potato::preprocess(auth_preprocess)]
async fn get_profile(cache: &mut OnceCache) -> HttpResponse {
    let user_id: u32 = cache.get("user_id").copied().unwrap_or(0);
    HttpResponse::text(format!("User: {}", user_id))
}
```

### 常用方法

- `cache.get::<T>(name)` - 获取值（返回`Option<&T>`）
- `cache.get_or_default::<T>(name, default)` - 获取值或默认值
- `cache.get_mut::<T>(name)` - 获取可变引用
- `cache.set::<T>(name, value)` - 设置值
- `cache.remove::<T>(name)` - 移除并返回值
- `cache.contains_key::<T>(name)` - 检查键是否存在

## SessionCache

用于跨请求保持用户会话数据，基于JWT token实现用户身份验证和数据隔离。

### 基本用法

```rust
// 登录接口签发token
#[potato::http_post("/login")]
async fn login(req: &mut HttpRequest) -> HttpResponse {
    let user_id = 12345i64;
    let token = SessionCache::generate_token(user_id, Duration::from_secs(3600)).unwrap();
    HttpResponse::json(serde_json::json!({ "token": token }))
}

// 直接使用SessionCache，宏自动处理token验证
#[potato::http_get("/profile")]
async fn get_profile(cache: &mut SessionCache) -> HttpResponse {
    let count: u32 = cache.get("visits").unwrap_or(0);
    cache.set("visits", count + 1);
    HttpResponse::text(format!("Visits: {}", count + 1))
}
```

### Cookie支持

SessionCache自动处理HTTP Cookie，支持完整的Cookie属性：
- 调用`get_cookie`自动读取请求的`Cookie` header
- 调用`set_cookie`/`set_cookie_with_builder`自动设置响应的`Set-Cookie` header
- 支持路径、域名、过期时间、Secure、HttpOnly、SameSite等所有标准属性

#### 简单用法

```rust
#[potato::http_get("/cookie/simple")]
async fn cookie_simple(cache: &mut SessionCache) -> HttpResponse {
    // 读取请求Cookie
    let session_id = cache.get_cookie("session_id");
    
    // 设置简单Cookie（默认路径为"/"）
    cache.set_cookie("theme", "dark");
    
    HttpResponse::text("ok")
}
```

#### 完整属性配置

```rust
use potato::CookieBuilder;
use chrono::Utc;

#[potato::http_get("/cookie/full")]
async fn cookie_full(cache: &mut SessionCache) -> HttpResponse {
    let expires = Utc::now().timestamp() + 3600; // 1小时后过期
    
    let cookie = CookieBuilder::new("user_token", "abc123")
        .path("/api")                    // 路径
        .domain(".example.com")          // 域名
        .expires(expires)                // 过期时间（Unix时间戳）
        .max_age(3600)                   // 最大存活时间（秒）
        .secure(true)                    // 仅HTTPS传输
        .http_only(true)                 // 禁止JavaScript访问
        .same_site("Strict");            // SameSite策略
    
    cache.set_cookie_with_builder(cookie);
    
    HttpResponse::text("ok")
}
```

#### 删除Cookie

```rust
#[potato::http_get("/cookie/delete")]
async fn cookie_delete(cache: &mut SessionCache) -> HttpResponse {
    // 简单删除
    cache.remove_cookie("session_id");
    
    // 带域名删除
    cache.remove_cookie_with_domain("user_token", ".example.com");
    
    HttpResponse::text("cookies deleted")
}
```

### 常用方法

#### 会话管理
- `SessionCache::set_jwt_secret(secret)` - 设置JWT密钥（全局）
- `SessionCache::generate_token(user_id, duration)` - 签发token
- `SessionCache::invalidate(user_id)` - 使session失效

#### 数据存储
- `cache.get::<T>(key)` - 获取值（需Clone）
- `cache.set::<T>(key, value)` - 设置值
- `cache.with_get::<T>(key, |v| ...)` - 读取并处理
- `cache.with_mut::<T>(key, |v| ...)` - 可变引用处理

#### Cookie操作
- `cache.get_cookie(name)` - 读取请求cookie（返回`Option<String>`）
- `cache.set_cookie(name, value)` - 设置简单cookie（默认路径"/"）
- `cache.set_cookie_with_builder(cookie)` - 使用CookieBuilder设置完整属性cookie
- `cache.remove_cookie(name)` - 移除cookie
- `cache.remove_cookie_with_domain(name, domain)` - 移除带域名的cookie

#### CookieBuilder方法
- `CookieBuilder::new(name, value)` - 创建cookie
- `.path(path)` - 设置路径
- `.domain(domain)` - 设置域名
- `.expires(timestamp)` - 设置过期时间（Unix时间戳，秒）
- `.max_age(seconds)` - 设置最大存活时间（秒）
- `.secure(bool)` - 设置Secure标志（仅HTTPS）
- `.http_only(bool)` - 设置HttpOnly标志（禁止JS访问）
- `.same_site(policy)` - 设置SameSite策略（"Strict"/"Lax"/"None"）

### 同时使用OnceCache和SessionCache

```rust
#[potato::http_get("/data")]
async fn get_data(once: &mut OnceCache, session: &mut SessionCache) -> HttpResponse {
    once.set("req_id", "abc");        // 单次请求
    session.set("user", "john");      // 跨请求保持
    HttpResponse::text("ok")
}
```
