# Cache System

Potato framework provides two cache systems: OnceCache (single request cache) and SessionCache (session-level cache).

## OnceCache

Used to pass data within a single HTTP request lifecycle, shared between preprocess, handler, and postprocess.

### Basic Usage

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

### Common Methods

- `cache.get::<T>(name)` - Get value (returns `Option<&T>`)
- `cache.get_or_default::<T>(name, default)` - Get value or default
- `cache.get_mut::<T>(name)` - Get mutable reference
- `cache.set::<T>(name, value)` - Set value
- `cache.remove::<T>(name)` - Remove and return value
- `cache.contains_key::<T>(name)` - Check if key exists

## SessionCache

Used to maintain user session data across requests, based on JWT token for user authentication and data isolation.

### Basic Usage

```rust
// Login endpoint to issue token
#[potato::http_post("/login")]
async fn login(req: &mut HttpRequest) -> HttpResponse {
    let user_id = 12345i64;
    let token = SessionCache::generate_token(user_id, Duration::from_secs(3600)).unwrap();
    HttpResponse::json(serde_json::json!({ "token": token }))
}

// Use SessionCache directly, macro automatically handles token validation
#[potato::http_get("/profile")]
async fn get_profile(cache: &mut SessionCache) -> HttpResponse {
    let count: u32 = cache.get("visits").unwrap_or(0);
    cache.set("visits", count + 1);
    HttpResponse::text(format!("Visits: {}", count + 1))
}
```

### Cookie Support

SessionCache automatically handles HTTP Cookies with full attribute support:
- Calling `get_cookie` automatically reads the request's `Cookie` header
- Calling `set_cookie`/`set_cookie_with_builder` automatically sets the response's `Set-Cookie` header
- Supports all standard attributes: path, domain, expires, Secure, HttpOnly, SameSite, etc.

#### Simple Usage

```rust
#[potato::http_get("/cookie/simple")]
async fn cookie_simple(cache: &mut SessionCache) -> HttpResponse {
    // Read request Cookie
    let session_id = cache.get_cookie("session_id");
    
    // Set simple Cookie (default path is "/")
    cache.set_cookie("theme", "dark");
    
    HttpResponse::text("ok")
}
```

#### Full Attribute Configuration

```rust
use potato::CookieBuilder;
use chrono::Utc;

#[potato::http_get("/cookie/full")]
async fn cookie_full(cache: &mut SessionCache) -> HttpResponse {
    let expires = Utc::now().timestamp() + 3600; // Expires in 1 hour
    
    let cookie = CookieBuilder::new("user_token", "abc123")
        .path("/api")                    // Path
        .domain(".example.com")          // Domain
        .expires(expires)                // Expiration time (Unix timestamp)
        .max_age(3600)                   // Max age in seconds
        .secure(true)                    // HTTPS only
        .http_only(true)                 // Prevent JavaScript access
        .same_site("Strict");            // SameSite policy
    
    cache.set_cookie_with_builder(cookie);
    
    HttpResponse::text("ok")
}
```

#### Deleting Cookies

```rust
#[potato::http_get("/cookie/delete")]
async fn cookie_delete(cache: &mut SessionCache) -> HttpResponse {
    // Simple delete
    cache.remove_cookie("session_id");
    
    // Delete with domain
    cache.remove_cookie_with_domain("user_token", ".example.com");
    
    HttpResponse::text("cookies deleted")
}
```

### Common Methods

#### Session Management
- `SessionCache::set_jwt_secret(secret)` - Set JWT secret (global)
- `SessionCache::generate_token(user_id, duration)` - Generate token
- `SessionCache::invalidate(user_id)` - Invalidate session

#### Data Storage
- `cache.get::<T>(key)` - Get value (requires Clone)
- `cache.set::<T>(key, value)` - Set value
- `cache.with_get::<T>(key, |v| ...)` - Read and process
- `cache.with_mut::<T>(key, |v| ...)` - Mutable reference processing

#### Cookie Operations
- `cache.get_cookie(name)` - Read request cookie (returns `Option<String>`)
- `cache.set_cookie(name, value)` - Set simple cookie (default path "/")
- `cache.set_cookie_with_builder(cookie)` - Set full attribute cookie using CookieBuilder
- `cache.remove_cookie(name)` - Remove cookie
- `cache.remove_cookie_with_domain(name, domain)` - Remove cookie with domain

#### CookieBuilder Methods
- `CookieBuilder::new(name, value)` - Create cookie
- `.path(path)` - Set path
- `.domain(domain)` - Set domain
- `.expires(timestamp)` - Set expiration time (Unix timestamp, seconds)
- `.max_age(seconds)` - Set max age in seconds
- `.secure(bool)` - Set Secure flag (HTTPS only)
- `.http_only(bool)` - Set HttpOnly flag (prevent JS access)
- `.same_site(policy)` - Set SameSite policy ("Strict"/"Lax"/"None")

### Using OnceCache and SessionCache Together

```rust
#[potato::http_get("/data")]
async fn get_data(once: &mut OnceCache, session: &mut SessionCache) -> HttpResponse {
    once.set("req_id", "abc");        // Single request
    session.set("user", "john");      // Cross-request persistence
    HttpResponse::text("ok")
}
```
