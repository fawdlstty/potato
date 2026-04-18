# Controller

Controller用于组织相关路由，提供统一路径前缀、中间件继承和Swagger自动分组。

## 基本结构

```rust
#[potato::controller]
pub struct UsersController<'a> {
    pub once_cache: &'a potato::OnceCache,
    pub sess_cache: &'a potato::SessionCache, // 可选
}

#[potato::controller("/api/users")]
impl<'a> UsersController<'a> {
    #[potato::http_get("/")]  // 路由: /api/users/
    pub async fn get(&self) -> anyhow::Result<&'static str> {
        Ok("get users")
    }

    #[potato::http_post("/")] // 路由: /api/users/
    pub async fn post(&mut self) -> anyhow::Result<&'static str> {
        Ok("post users")
    }

    #[potato::http_get("/any")] // 路由: /api/users/any
    pub async fn get_any() -> anyhow::Result<&'static str> {
        Ok("get any")
    }
}
```

## 结构体定义

```rust
#[potato::controller]  // 结构体无需path
pub struct MyController<'a> {
    pub once_cache: &'a potato::OnceCache,   // 可选
    pub sess_cache: &'a potato::SessionCache, // 可选
}
```

**字段说明**：
- 仅支持 `&'a OnceCache` 和 `&'a SessionCache` 类型
- 字段可选，可只定义其中一个或两个都定义
- 无数量限制

## impl块标注

```rust
#[potato::controller("/api/v1")]  // base path必须写在impl块
#[potato::preprocess(fn1, fn2)]   // 可选：应用到所有方法
#[potato::postprocess(fn1)]       // 可选：应用到所有方法
impl<'a> MyController<'a> {
    // 方法定义
}
```

## 方法路由

```rust
#[potato::http_get("/path")]      // 完整路由: base_path + /path
#[potato::http_post("/path")]
#[potato::http_put("/path")]
#[potato::http_delete("/path")]
#[potato::http_patch("/path")]
#[potato::http_head("/path")]
#[potato::http_options("/path")]
pub async fn handler(&self) -> anyhow::Result<T> {
    // 实现
}
```

**路径规则**：
- 方法path必须以`/`开头
- 最终路由 = struct的base path + 方法path
- `http_get("/")` → base path本身

## 自动鉴权

**触发条件**：方法有 `&self` 或 `&mut self` 参数 **且** 结构体包含 `SessionCache` 字段

**鉴权流程**：
- 从 `Authorization: Bearer <token>` 提取 token
- 调用 `SessionCache::from_token` 加载会话
- 验证失败返回 401

**无需鉴权的情况**：
- 结构体不包含 `SessionCache` 字段：自动创建临时 SessionCache，跳过鉴权
- 方法无 receiver（静态方法）：直接调用，无需实例化

## Swagger集成

- 使用结构体名作为 tag（例：`UsersController`）
- 自动标记需要鉴权的接口
- 自动去除生命周期参数（`UsersController<'a>` → `UsersController`）

## 注册路由

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers(true);      // 启用宏注册的路由
        ctx.use_openapi("/doc/");    // 可选：Swagger文档
    });
    server.serve_http().await
}
```
