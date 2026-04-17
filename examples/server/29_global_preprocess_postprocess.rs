use potato::{get, HttpResponse, HttpServer};

// 使用宏标注预处理函数
#[potato::preprocess]
async fn global_preprocess(req: &mut HttpRequest) -> Option<HttpResponse> {
    println!("[Preprocess] Request: {} {}", req.method, req.url_path);
    // 可以在这里进行认证检查、日志记录等
    // 如果返回 Some(response)，请求将被短路返回
    None
}

// 使用宏标注后处理函数
#[potato::postprocess]
async fn global_postprocess(req: &mut HttpRequest, res: &mut HttpResponse) {
    println!(
        "[Postprocess] Response: {} - Status: {}",
        req.url_path, res.http_code
    );
    // 可以在这里添加通用响应头、记录响应时间等
    res.add_header("X-Processed-By".into(), "global-postprocess".into());
}

#[get("/")]
async fn index() -> HttpResponse {
    HttpResponse::html("<h1>Hello from Global Preprocess/Postprocess!</h1>")
}

#[get("/api/data")]
async fn get_data() -> HttpResponse {
    HttpResponse::json(r#"{"message": "This is API data"}"#)
}

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("127.0.0.1:8080");

    server.configure(|ctx| {
        // 注册全局预处理函数（必须是 #[potato::preprocess] 标注的函数）
        ctx.use_preprocess(global_preprocess);

        // 注册全局后处理函数（必须是 #[potato::postprocess] 标注的函数）
        ctx.use_postprocess(global_postprocess);

        // 其他路由配置
        ctx.use_handlers();
    });

    println!("Server running at http://127.0.0.1:8080");
    server.run().await.unwrap();
}
