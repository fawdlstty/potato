/// 测试 OnceCache 参数功能
use std::time::Duration;
use tokio::time::sleep;

use potato::{HttpRequest, HttpResponse, HttpServer, OnceCache};

#[tokio::test]
async fn test_once_cache_in_handler() -> anyhow::Result<()> {
    // 测试 handler 中使用 OnceCache
    #[potato::http_get("/cache_handler")]
    async fn cache_handler(cache: &mut OnceCache) -> HttpResponse {
        cache.set("message", "hello from cache".to_string());
        let msg: String = cache
            .get::<String>("message")
            .expect("message not found")
            .clone();
        HttpResponse::text(msg)
    }

    let port = 18090;
    let server_addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/cache_handler", server_addr);
    match potato::get(&url, vec![]).await {
        Ok(res) => {
            assert_eq!(res.http_code, 200);
            let body = match &res.body {
                potato::HttpResponseBody::Data(data) => {
                    String::from_utf8(data.clone()).unwrap_or_default()
                }
                potato::HttpResponseBody::Stream(_) => "stream response".to_string(),
            };
            assert_eq!(body, "hello from cache");
            println!("✅ Handler with OnceCache test passed");
        }
        Err(e) => {
            panic!("Handler with OnceCache test failed: {}", e);
        }
    }

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_once_cache_in_preprocess() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PREPROCESS_CALLED: AtomicUsize = AtomicUsize::new(0);
    static HANDLER_CALLED: AtomicUsize = AtomicUsize::new(0);

    PREPROCESS_CALLED.store(0, Ordering::Relaxed);
    HANDLER_CALLED.store(0, Ordering::Relaxed);

    // 测试 preprocess 中使用 OnceCache 传递数据给 handler
    #[potato::preprocess]
    async fn pre_with_cache(req: &mut HttpRequest, cache: &mut OnceCache) {
        PREPROCESS_CALLED.fetch_add(1, Ordering::Relaxed);
        cache.set("user_id", 12345u32);
        cache.set("username", "test_user".to_string());
    }

    #[potato::http_get("/cache_preprocess")]
    #[potato::preprocess(pre_with_cache)]
    async fn handler_with_cache(cache: &mut OnceCache) -> HttpResponse {
        HANDLER_CALLED.fetch_add(1, Ordering::Relaxed);
        let user_id: u32 = *cache.get::<u32>("user_id").expect("user_id not found");
        let username: String = cache
            .get::<String>("username")
            .expect("username not found")
            .clone();
        HttpResponse::text(format!("user: {} (id: {})", username, user_id))
    }

    let port = 18091;
    let server_addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/cache_preprocess", server_addr);
    match potato::get(&url, vec![]).await {
        Ok(res) => {
            assert_eq!(res.http_code, 200);
            let body = match &res.body {
                potato::HttpResponseBody::Data(data) => {
                    String::from_utf8(data.clone()).unwrap_or_default()
                }
                potato::HttpResponseBody::Stream(_) => "stream response".to_string(),
            };
            assert_eq!(body, "user: test_user (id: 12345)");
            assert_eq!(PREPROCESS_CALLED.load(Ordering::Relaxed), 1);
            assert_eq!(HANDLER_CALLED.load(Ordering::Relaxed), 1);
            println!("✅ Preprocess with OnceCache test passed");
        }
        Err(e) => {
            panic!("Preprocess with OnceCache test failed: {}", e);
        }
    }

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_once_cache_in_postprocess() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static HANDLER_CALLED: AtomicUsize = AtomicUsize::new(0);
    static POSTPROCESS_CALLED: AtomicUsize = AtomicUsize::new(0);

    HANDLER_CALLED.store(0, Ordering::Relaxed);
    POSTPROCESS_CALLED.store(0, Ordering::Relaxed);

    #[potato::postprocess]
    fn post_with_cache(_req: &mut HttpRequest, res: &mut HttpResponse, cache: &mut OnceCache) {
        POSTPROCESS_CALLED.fetch_add(1, Ordering::Relaxed);
        let process_time: String = cache
            .get::<String>("process_time")
            .expect("process_time not found")
            .clone();
        // 在响应中添加处理时间信息
        let body_str = match &res.body {
            potato::HttpResponseBody::Data(data) => {
                String::from_utf8(data.clone()).unwrap_or_default()
            }
            potato::HttpResponseBody::Stream(_) => "stream".to_string(),
        };
        res.body = potato::HttpResponseBody::Data(
            format!("{} | processed in: {}", body_str, process_time).into_bytes(),
        );
    }

    // 测试 handler 设置 cache,postprocess 读取并修改响应
    #[potato::http_get("/cache_postprocess")]
    #[potato::postprocess(post_with_cache)]
    async fn handler_for_post(cache: &mut OnceCache) -> HttpResponse {
        HANDLER_CALLED.fetch_add(1, Ordering::Relaxed);
        cache.set("process_time", "100ms".to_string());
        HttpResponse::text("original response")
    }

    let port = 18092;
    let server_addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/cache_postprocess", server_addr);
    match potato::get(&url, vec![]).await {
        Ok(res) => {
            assert_eq!(res.http_code, 200);
            let body = match &res.body {
                potato::HttpResponseBody::Data(data) => {
                    String::from_utf8(data.clone()).unwrap_or_default()
                }
                potato::HttpResponseBody::Stream(_) => "stream response".to_string(),
            };
            assert_eq!(body, "original response | processed in: 100ms");
            assert_eq!(HANDLER_CALLED.load(Ordering::Relaxed), 1);
            assert_eq!(POSTPROCESS_CALLED.load(Ordering::Relaxed), 1);
            println!("✅ Postprocess with OnceCache test passed");
        }
        Err(e) => {
            panic!("Postprocess with OnceCache test failed: {}", e);
        }
    }

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_once_cache_full_pipeline() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PRE_CALLED: AtomicUsize = AtomicUsize::new(0);
    static HANDLER_CALLED: AtomicUsize = AtomicUsize::new(0);
    static POST_CALLED: AtomicUsize = AtomicUsize::new(0);

    PRE_CALLED.store(0, Ordering::Relaxed);
    HANDLER_CALLED.store(0, Ordering::Relaxed);
    POST_CALLED.store(0, Ordering::Relaxed);

    // 完整流程:preprocess -> handler -> postprocess 都使用 cache
    #[potato::preprocess]
    fn pre_set_cache(req: &mut HttpRequest, cache: &mut OnceCache) {
        PRE_CALLED.fetch_add(1, Ordering::Relaxed);
        cache.set("step", "preprocess".to_string());
        cache.set("data_from_pre", "pre_data".to_string());
    }

    #[potato::http_get("/cache_full")]
    #[potato::preprocess(pre_set_cache)]
    #[potato::postprocess(post_modify_cache)]
    async fn handler_full_pipe(cache: &mut OnceCache) -> HttpResponse {
        HANDLER_CALLED.fetch_add(1, Ordering::Relaxed);
        let step: String = cache.get::<String>("step").expect("step not found").clone();
        assert_eq!(step, "preprocess");

        cache.set("step", "handler".to_string());
        let pre_data: String = cache
            .get::<String>("data_from_pre")
            .expect("data_from_pre not found")
            .clone();
        cache.set("data_from_handler", format!("handler_{}", pre_data));

        HttpResponse::text("handler response")
    }

    #[potato::postprocess]
    fn post_modify_cache(_req: &mut HttpRequest, res: &mut HttpResponse, cache: &mut OnceCache) {
        POST_CALLED.fetch_add(1, Ordering::Relaxed);
        let step: String = cache.get::<String>("step").expect("step not found").clone();
        assert_eq!(step, "handler");

        let handler_data: String = cache
            .get::<String>("data_from_handler")
            .expect("data_from_handler not found")
            .clone();
        let body_str = match &res.body {
            potato::HttpResponseBody::Data(data) => {
                String::from_utf8(data.clone()).unwrap_or_default()
            }
            potato::HttpResponseBody::Stream(_) => "stream".to_string(),
        };
        res.body =
            potato::HttpResponseBody::Data(format!("{} | {}", body_str, handler_data).into_bytes());
    }

    let port = 18093;
    let server_addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/cache_full", server_addr);
    match potato::get(&url, vec![]).await {
        Ok(res) => {
            assert_eq!(res.http_code, 200);
            let body = match &res.body {
                potato::HttpResponseBody::Data(data) => {
                    String::from_utf8(data.clone()).unwrap_or_default()
                }
                potato::HttpResponseBody::Stream(_) => "stream response".to_string(),
            };
            assert_eq!(body, "handler response | handler_pre_data");
            assert_eq!(PRE_CALLED.load(Ordering::Relaxed), 1);
            assert_eq!(HANDLER_CALLED.load(Ordering::Relaxed), 1);
            assert_eq!(POST_CALLED.load(Ordering::Relaxed), 1);
            println!("✅ Full pipeline with OnceCache test passed");
        }
        Err(e) => {
            panic!("Full pipeline with OnceCache test failed: {}", e);
        }
    }

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_once_cache_remove() -> anyhow::Result<()> {
    // 测试 remove 功能
    #[potato::http_get("/cache_remove")]
    async fn cache_remove_handler(cache: &mut OnceCache) -> HttpResponse {
        cache.set("temp", "temporary value".to_string());
        let removed: Option<String> = cache.remove("temp");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap(), "temporary value");

        // 再次获取应该返回None
        let result: Option<&String> = cache.get("temp");
        assert!(result.is_none());

        HttpResponse::text("remove test passed")
    }

    let port = 18094;
    let server_addr = format!("127.0.0.1:{}", port);
    let mut server = HttpServer::new(&server_addr);

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    sleep(Duration::from_millis(300)).await;

    let url = format!("http://{}/cache_remove", server_addr);
    match potato::get(&url, vec![]).await {
        Ok(res) => {
            assert_eq!(res.http_code, 200);
            println!("✅ OnceCache remove test passed");
        }
        Err(e) => {
            panic!("OnceCache remove test failed: {}", e);
        }
    }

    server_handle.abort();
    Ok(())
}
