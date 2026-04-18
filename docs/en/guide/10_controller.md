# Controller

Controller organizes related routes with unified path prefix, middleware inheritance, and automatic Swagger grouping.

## Basic Structure

```rust
#[potato::controller]
pub struct UsersController<'a> {
    pub once_cache: &'a potato::OnceCache,
    pub sess_cache: &'a potato::SessionCache, // Optional
}

#[potato::controller("/api/users")]
impl<'a> UsersController<'a> {
    #[potato::http_get("/")]  // Route: /api/users/
    pub async fn get(&self) -> anyhow::Result<&'static str> {
        Ok("get users")
    }

    #[potato::http_post("/")] // Route: /api/users/
    pub async fn post(&mut self) -> anyhow::Result<&'static str> {
        Ok("post users")
    }

    #[potato::http_get("/any")] // Route: /api/users/any
    pub async fn get_any() -> anyhow::Result<&'static str> {
        Ok("get any")
    }
}
```

## Struct Definition

```rust
#[potato::controller]  // No path on struct
pub struct MyController<'a> {
    pub once_cache: &'a potato::OnceCache,   // Optional
    pub sess_cache: &'a potato::SessionCache, // Optional
}
```

**Field Rules**:
- Only `&'a OnceCache` and `&'a SessionCache` types allowed
- Fields are optional: define either, both, or none
- No quantity limits

## Impl Block Annotations

```rust
#[potato::controller("/api/v1")]  // Base path must be on impl block
#[potato::preprocess(fn1, fn2)]   // Optional: applies to all methods
#[potato::postprocess(fn1)]       // Optional: applies to all methods
impl<'a> MyController<'a> {
    // Method definitions
}
```

## Method Routes

```rust
#[potato::http_get("/path")]      // Full route: base_path + /path
#[potato::http_post("/path")]
#[potato::http_put("/path")]
#[potato::http_delete("/path")]
#[potato::http_patch("/path")]
#[potato::http_head("/path")]
#[potato::http_options("/path")]
pub async fn handler(&self) -> anyhow::Result<T> {
    // Implementation
}
```

**Path rules**:
- Method path must start with `/`
- Final route = struct base path + method path
- `http_get("/")` → base path itself

## Automatic Authentication

**Trigger**: Method has `&self` or `&mut self` **and** struct contains `SessionCache` field

**Auth Flow**:
- Extract token from `Authorization: Bearer <token>`
- Call `SessionCache::from_token` to load session
- Return 401 on failure

**No Auth Required**:
- Struct without `SessionCache`: auto-creates temporary SessionCache, skips auth
- Static methods (no receiver): called directly, no instantiation needed

## Swagger Integration

- Uses struct name as tag (e.g., `UsersController`)
- Auto-marks authenticated endpoints
- Auto-removes lifetime parameters (`UsersController<'a>` → `UsersController`)

## Route Registration

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers(true);      // Enable macro-registered routes
        ctx.use_openapi("/doc/");    // Optional: Swagger docs
    });
    server.serve_http().await
}
```
