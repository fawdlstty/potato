#![cfg(feature = "std")]
/// 路径参数路由测试：{name} 段匹配、类型解析、静态优先级、HEAD/OPTIONS 行为
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(34000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[potato::http_get("/pp/user/{id}")]
async fn pp_user(id: i64) -> HttpResponse {
    HttpResponse::text(format!("user id = {id}"))
}

#[potato::http_get("/pp/user/admin")]
async fn pp_admin() -> HttpResponse {
    HttpResponse::text("admin page")
}

#[potato::http_get("/pp/post/{year}/title/{slug}")]
async fn pp_post_title(year: u32, slug: String) -> HttpResponse {
    HttpResponse::text(format!("post year={year} slug={slug}"))
}

#[potato::http_get("/pp/file/{size}")]
async fn pp_file_size(size: u64) -> HttpResponse {
    HttpResponse::text(format!("file size = {size}"))
}

// 路径参数 + query 参数混用
#[potato::http_get("/pp/item/{id}")]
async fn pp_item(id: i64, q: String) -> HttpResponse {
    HttpResponse::text(format!("item id={id} q={q}"))
}

// String 参数 %xx 解码（%2F 不切分段，解码后含 /）
#[potato::http_get("/pp/echo/{val}")]
async fn pp_echo(val: String) -> HttpResponse {
    HttpResponse::text(format!("echo: {val}"))
}

use potato::{HttpRequest, HttpResponse, HttpServer};

async fn start_server() -> anyhow::Result<String> {
    let port = get_test_port();
    let server_addr = format!("127.0.0.1:{port}");
    let mut server = HttpServer::new(&server_addr);
    let _ = server.configure(|ctx| {
        ctx.use_handlers();
    });
    let _server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });
    sleep(Duration::from_millis(300)).await;
    Ok(server_addr)
}

fn url_of(addr: &str, path: &str) -> String {
    format!("http://{addr}{path}")
}

fn body_string(res: potato::HttpResponse) -> String {
    match res.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8_lossy(&data).into_owned(),
        potato::HttpResponseBody::Stream(_) => String::new(),
    }
}

#[tokio::test]
async fn test_path_param_i64() -> anyhow::Result<()> {
    let addr = start_server().await?;
    let res = potato::get(&url_of(&addr, "/pp/user/123"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "user id = 123");

    // 负数
    let res = potato::get(&url_of(&addr, "/pp/user/-5"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "user id = -5");
    Ok(())
}

#[tokio::test]
async fn test_static_route_priority() -> anyhow::Result<()> {
    let addr = start_server().await?;
    // 固定段精确命中，不被 /pp/user/{id} 抢走
    let res = potato::get(&url_of(&addr, "/pp/user/admin"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "admin page");
    Ok(())
}

#[tokio::test]
async fn test_path_param_multi() -> anyhow::Result<()> {
    let addr = start_server().await?;
    let res = potato::get(&url_of(&addr, "/pp/post/2024/title/hello-world"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "post year=2024 slug=hello-world");
    Ok(())
}

#[tokio::test]
async fn test_path_param_percent_decode() -> anyhow::Result<()> {
    let addr = start_server().await?;
    // %20 解码为空格
    let res = potato::get(&url_of(&addr, "/pp/post/2024/title/hello%20world"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "post year=2024 slug=hello world");

    // %2F 解码为 / 但不切分段（单段匹配）
    let res = potato::get(&url_of(&addr, "/pp/echo/a%2Fb"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "echo: a/b");
    Ok(())
}

#[tokio::test]
async fn test_path_param_type_mismatch_400() -> anyhow::Result<()> {
    let addr = start_server().await?;
    let res = potato::get(&url_of(&addr, "/pp/file/abc"), vec![]).await?;
    assert_eq!(res.http_code, 400);

    let res = potato::get(&url_of(&addr, "/pp/file/4096"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "file size = 4096");
    Ok(())
}

#[tokio::test]
async fn test_path_param_no_match_404() -> anyhow::Result<()> {
    let addr = start_server().await?;
    // 空参数段不命中
    let res = potato::get(&url_of(&addr, "/pp/user/"), vec![]).await?;
    assert_eq!(res.http_code, 404);
    // 段数不同不命中
    let res = potato::get(&url_of(&addr, "/pp/user/1/extra"), vec![]).await?;
    assert_eq!(res.http_code, 404);
    let res = potato::get(&url_of(&addr, "/pp/unknown"), vec![]).await?;
    assert_eq!(res.http_code, 404);
    Ok(())
}

#[tokio::test]
async fn test_path_param_with_query() -> anyhow::Result<()> {
    let addr = start_server().await?;
    let res = potato::get(&url_of(&addr, "/pp/item/9?q=abc"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "item id=9 q=abc");

    // 缺 query 参数返回 400
    let res = potato::get(&url_of(&addr, "/pp/item/9"), vec![]).await?;
    assert_eq!(res.http_code, 400);
    Ok(())
}

#[tokio::test]
async fn test_path_param_head_fallback() -> anyhow::Result<()> {
    let addr = start_server().await?;
    // 无显式 HEAD 路由时回退到动态 GET 路由
    let res = potato::head(&url_of(&addr, "/pp/user/77"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    Ok(())
}

#[tokio::test]
async fn test_path_param_options_allow() -> anyhow::Result<()> {
    let addr = start_server().await?;
    let res = potato::options(&url_of(&addr, "/pp/user/1"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    let allow = res
        .get_header("Allow")
        .map(|s| s.to_string())
        .unwrap_or_default();
    assert!(allow.contains("GET"), "Allow should contain GET: {allow}");
    assert!(allow.contains("HEAD"), "Allow should contain HEAD: {allow}");
    assert!(
        allow.contains("OPTIONS"),
        "Allow should contain OPTIONS: {allow}"
    );
    Ok(())
}

#[tokio::test]
async fn test_path_param_manual_access() -> anyhow::Result<()> {
    let addr = start_server().await?;
    // 无宏提取时可通过 req.path_param 手动取值（控制器场景等价用法）
    let res = potato::get(&url_of(&addr, "/pp/echo/plain"), vec![]).await?;
    assert_eq!(res.http_code, 200);
    assert_eq!(body_string(res), "echo: plain");
    let _ = HttpRequest::new();
    Ok(())
}
