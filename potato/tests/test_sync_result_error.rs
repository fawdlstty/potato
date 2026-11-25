#![cfg(feature = "std")]
/// 集成测试：同步 handler 返回 Result 时的 Err 处理
///
/// sync 快路径（非 async、无缓存/无钩子/无适配器）此前对 handler 返回的
/// Err 直接 unwrap，恶意请求可让 handler 走 Err 分支打死进程（回归保护）。
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(22000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn body_string(res: &potato::HttpResponse) -> String {
    match &res.body {
        potato::HttpResponseBody::Data(data) => String::from_utf8_lossy(data).to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::{HttpRequest, HttpResponse, HttpServer};

    async fn start_server() -> anyhow::Result<(String, tokio::task::JoinHandle<()>)> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");
        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });
        sleep(Duration::from_millis(300)).await;
        Ok((server_addr, server_handle))
    }

    /// sync handler 返回 Err 时应转 500，而不是 panic 打死进程
    #[tokio::test]
    async fn test_sync_result_err_returns_500() -> anyhow::Result<()> {
        let (server_addr, server_handle) = start_server().await?;

        #[potato::http_get("/sync-error")]
        fn sync_error_handler() -> anyhow::Result<HttpResponse> {
            anyhow::bail!("sync handler error");
        }

        #[potato::http_post("/sync-error-arg")]
        fn sync_error_handler_with_arg(req: &mut HttpRequest) -> anyhow::Result<HttpResponse> {
            let _ = req;
            anyhow::bail!("sync handler error with arg");
        }

        let url = format!("http://{}/sync-error", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 500, "sync handler Err 应返回 500 而非 panic");
        assert!(
            body_string(&res).contains("Internal Server Error"),
            "对外应是通用文案，不泄露错误链"
        );

        let url = format!("http://{}/sync-error-arg", server_addr);
        let res = potato::post(&url, vec![], vec![]).await?;
        assert_eq!(res.http_code, 500, "带参数的 sync handler Err 同样应返回 500");

        println!("✅ sync Result Err 转 500 works");
        server_handle.abort();
        Ok(())
    }

    /// sync handler 返回 Ok 时行为不变
    #[tokio::test]
    async fn test_sync_result_ok_still_works() -> anyhow::Result<()> {
        let (server_addr, server_handle) = start_server().await?;

        #[potato::http_get("/sync-ok")]
        fn sync_ok_handler() -> anyhow::Result<HttpResponse> {
            Ok(HttpResponse::text("sync ok"))
        }

        let url = format!("http://{}/sync-ok", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 200);
        assert_eq!(body_string(&res), "sync ok");

        println!("✅ sync Result Ok works");
        server_handle.abort();
        Ok(())
    }

    /// sync handler 返回非 Result 类型时快路径不受影响
    #[tokio::test]
    async fn test_sync_plain_handler_still_works() -> anyhow::Result<()> {
        let (server_addr, server_handle) = start_server().await?;

        #[potato::http_get("/sync-plain")]
        fn sync_plain_handler() -> HttpResponse {
            HttpResponse::text("sync plain")
        }

        let url = format!("http://{}/sync-plain", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 200);
        assert_eq!(body_string(&res), "sync plain");

        println!("✅ sync plain handler works");
        server_handle.abort();
        Ok(())
    }
}
