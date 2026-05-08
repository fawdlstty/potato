//! 测试HTTP handler宏中Send标志的一致性
//!
//! 测试覆盖以下场景：
//! 1. 直接使用http_get等宏，带Send标志
//! 2. 直接使用http_get等宏，不带Send标志（默认为Send）

use potato::{HttpResponse, HttpServer, OnceCache, SessionCache};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(18500);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：直接使用http_get，带Send标志（显式）
    #[tokio::test]
    async fn test_direct_send_explicit() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::http_get("/test_send_explicit", Send)]
        async fn handler() -> HttpResponse {
            HttpResponse::text("direct_send_explicit")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/test_send_explicit");
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "direct_send_explicit");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：直接使用http_get，不带Send标志（默认为Send）
    #[tokio::test]
    async fn test_direct_send_default() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::http_get("/test_send_default")]
        async fn handler() -> HttpResponse {
            HttpResponse::text("direct_send_default")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/test_send_default");
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "direct_send_default");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：直接使用http_post，带Send标志
    #[tokio::test]
    async fn test_direct_post_send() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::http_post("/test_post_send", Send)]
        async fn handler() -> HttpResponse {
            HttpResponse::text("direct_post_send")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/test_post_send");
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "direct_post_send");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：直接使用http_post，不带Send标志
    #[tokio::test]
    async fn test_direct_post_no_send_flag() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::http_post("/test_post_no_send_flag")]
        async fn handler() -> HttpResponse {
            HttpResponse::text("direct_post_no_send_flag")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/test_post_no_send_flag");
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "direct_post_no_send_flag");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：controller中使用http_get，带Send标志
    #[tokio::test]
    async fn test_controller_send_explicit() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::controller]
        struct TestController<'a> {
            pub once_cache: &'a OnceCache,
        }

        #[potato::controller("/api")]
        impl<'a> TestController<'a> {
            #[potato::http_get("/test_ctrl_send_explicit", Send)]
            async fn handler(&self) -> HttpResponse {
                HttpResponse::text("controller_send_explicit")
            }
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/api/test_ctrl_send_explicit");
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "controller_send_explicit");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：controller中使用http_get，不带Send标志（默认为Send）
    #[tokio::test]
    async fn test_controller_send_default() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::controller]
        struct TestController2<'a> {
            pub once_cache: &'a OnceCache,
        }

        #[potato::controller("/api")]
        impl<'a> TestController2<'a> {
            #[potato::http_get("/test_ctrl_send_default")]
            async fn handler(&self) -> HttpResponse {
                HttpResponse::text("controller_send_default")
            }
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/api/test_ctrl_send_default");
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "controller_send_default");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：controller中使用http_post，带Send标志
    #[tokio::test]
    async fn test_controller_post_send() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::controller]
        struct TestController3<'a> {
            pub once_cache: &'a OnceCache,
        }

        #[potato::controller("/api")]
        impl<'a> TestController3<'a> {
            #[potato::http_post("/test_ctrl_post_send", Send)]
            async fn handler(&self) -> HttpResponse {
                HttpResponse::text("controller_post_send")
            }
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/api/test_ctrl_post_send");
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "controller_post_send");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }

    /// 测试：controller中使用http_post，不带Send标志
    #[tokio::test]
    async fn test_controller_post_no_send_flag() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{port}");

        #[potato::controller]
        struct TestController4<'a> {
            pub once_cache: &'a OnceCache,
        }

        #[potato::controller("/api")]
        impl<'a> TestController4<'a> {
            #[potato::http_post("/test_ctrl_post_no_send_flag")]
            async fn handler(&self) -> HttpResponse {
                HttpResponse::text("controller_post_no_send_flag")
            }
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{server_addr}/api/test_ctrl_post_no_send_flag");
        match potato::post(&url, vec![], vec![]).await {
            Ok(res) => {
                assert_eq!(res.http_code, 200);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream".to_string(),
                };
                assert_eq!(body, "controller_post_no_send_flag");
            }
            Err(e) => panic!("Request failed: {e}"),
        }

        server_handle.abort();
        Ok(())
    }
}
