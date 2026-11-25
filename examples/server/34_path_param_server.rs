// 路径参数示例：路由模板里的 {name} 段按 handler 同名参数的类型自动解析
// 固定段精确匹配优先于参数段（/user/admin 命中静态路由，/user/其他 命中 /user/{id}）
#[potato::http_get("/user/{id}")]
async fn user(id: i64) -> potato::HttpResponse {
    potato::HttpResponse::text(format!("user id = {id}"))
}

// 多参数：/post/2024/path-param 支持
#[potato::http_get("/post/{year}/title/{slug}")]
async fn post_title(year: u32, slug: String) -> potato::HttpResponse {
    potato::HttpResponse::text(format!("post year={year} slug={slug}"))
}

// 静态路由优先级高于同形动态路由
#[potato::http_get("/user/admin")]
async fn admin() -> potato::HttpResponse {
    potato::HttpResponse::text("admin page")
}

// 类型不匹配返回 400（/file/not-a-number 触发）
#[potato::http_get("/file/{size}")]
async fn file_size(size: u64) -> potato::HttpResponse {
    potato::HttpResponse::text(format!("file size = {size}"))
}

// 控制器/无宏路由可通过 req.path_param("name") 手动取值
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers();
        ctx.use_openapi("/doc/");
    });
    println!("try: http://127.0.0.1:8080/user/123");
    println!("try: http://127.0.0.1:8080/user/admin");
    println!("try: http://127.0.0.1:8080/post/2024/title/hello-world");
    println!("try: http://127.0.0.1:8080/file/4096  (or /file/abc for 400)");
    println!("doc: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
