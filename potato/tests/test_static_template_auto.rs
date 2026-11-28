//! 静态响应模板自动登记（方案C 自动化）行为测试：
//! 1. 零参同步 `-> &'static str` 字面量体 GET handler 经路由宏自动登记
//!    （inventory 运行时收集进模板表）；
//! 2. 默认管线形态实例命中快路径：响应 body 与正常路径一致（语义无损）；
//! 3. 带附加管线项（CORS）的实例被连接级守卫拦截，不被模板截胡
//!    （响应带 Access-Control-Allow-Origin 头，证明走了正常管线）。

use std::sync::atomic::{AtomicU16, Ordering};

use potato::{CorsConfig, HttpServer};

static PORT: AtomicU16 = AtomicU16::new(18610);

fn get_test_port() -> u16 {
    PORT.fetch_add(1, Ordering::Relaxed)
}

/// 自动登记来源 handler：唯一路径防与其他测试的路由/模板冲突
#[potato::http_get("/__potato_tpl_auto_pin__")]
fn tpl_auto_pin() -> &'static str {
    "auto-template-body-pin"
}

/// const 别名体 handler（bare path 表达式）：加宽后的自动登记同样命中
const TPL_ALIAS_BODY: &str = "auto-template-body-alias";

#[potato::http_get("/__potato_tpl_auto_alias__")]
fn tpl_auto_alias() -> &'static str {
    TPL_ALIAS_BODY
}

/// 模板表已含宏自动登记条目（inventory 收集生效）
#[test]
fn auto_template_flag_collected() {
    assert!(
        potato::static_template_count() >= 1,
        "macro-registered static template flag must be collected into the table"
    );
}

/// 请求直到服务器就绪（连接被拒重试），超时返回 None。
/// 带 Origin 头：CORS 实例对无 Origin 请求不回 ACAO 头（守卫断言依赖）。
async fn get_when_ready(url: &str) -> Option<potato::HttpResponse> {
    for _ in 0..50 {
        let url_ref: &str = url;
        if let Ok(res) = potato::get!(url_ref, "Origin" = "http://localhost").await {
            return Some(res);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    None
}

#[tokio::test]
async fn default_pipeline_serves_registered_body() -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", get_test_port());
    let mut server = HttpServer::new(&addr);
    let _ = server.configure(|ctx| ctx.use_handlers());
    let handle = tokio::spawn(async move { server.serve_http().await });

    let url = format!("http://{addr}/__potato_tpl_auto_pin__");
    let res = get_when_ready(&url).await.expect("server should answer");
    assert_eq!(res.http_code, 200);
    match &res.body {
        potato::HttpResponseBody::Data(data) => {
            assert_eq!(String::from_utf8_lossy(data), "auto-template-body-pin")
        }
        _ => panic!("expected Data body"),
    }
    handle.abort();
    Ok(())
}

/// const 别名体 handler 同样自动登记并命中（默认管线形态，语义无损）
#[tokio::test]
async fn default_pipeline_serves_const_alias_body() -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", get_test_port());
    let mut server = HttpServer::new(&addr);
    let _ = server.configure(|ctx| ctx.use_handlers());
    let handle = tokio::spawn(async move { server.serve_http().await });

    let url = format!("http://{addr}/__potato_tpl_auto_alias__");
    let res = get_when_ready(&url).await.expect("server should answer");
    assert_eq!(res.http_code, 200);
    match &res.body {
        potato::HttpResponseBody::Data(data) => {
            assert_eq!(String::from_utf8_lossy(data), "auto-template-body-alias")
        }
        _ => panic!("expected Data body"),
    }
    handle.abort();
    Ok(())
}

#[tokio::test]
async fn cors_pipeline_bypasses_template() -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", get_test_port());
    let mut server = HttpServer::new(&addr);
    let _ = server.configure(|ctx| {
        ctx.use_handlers();
        ctx.use_cors(CorsConfig::default_minimal());
    });
    let handle = tokio::spawn(async move { server.serve_http().await });

    let url = format!("http://{addr}/__potato_tpl_auto_pin__");
    let res = get_when_ready(&url).await.expect("server should answer");
    assert_eq!(res.http_code, 200);
    match &res.body {
        potato::HttpResponseBody::Data(data) => {
            assert_eq!(String::from_utf8_lossy(data), "auto-template-body-pin")
        }
        _ => panic!("expected Data body"),
    }
    // 守卫断言：CORS 实例走正常管线（模板字节无该头，命中即失败）
    let bytes = res.as_bytes(potato::CompressMode::None, None);
    assert!(
        bytes
            .windows(27)
            .any(|w| w == b"Access-Control-Allow-Origin"),
        "CORS pipeline instance must bypass the static template fast path"
    );
    handle.abort();
    Ok(())
}
