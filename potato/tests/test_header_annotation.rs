/// 集成测试：验证 #[header(...)] 标注功能
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(30000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::{HttpRequest, HttpResponse, HttpServer};

    /// 测试单个 header 标注
    #[tokio::test]
    async fn test_single_header_annotation() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/single-header")]
        #[header(Cache_Control = "no-store, no-cache, max-age=0")]
        async fn single_header_handler() -> HttpResponse {
            HttpResponse::text("single header test")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/single-header", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);

        // 调试输出：查看所有headers
        println!("Response headers: {:?}", res.headers);

        // 验证 header 被正确添加
        let cache_control = res.headers.get("Cache-Control");
        assert!(
            cache_control.is_some(),
            "Cache-Control header should be present"
        );
        assert_eq!(cache_control.unwrap(), "no-store, no-cache, max-age=0");

        println!("✅ Single header annotation works");

        server_handle.abort();
        Ok(())
    }

    /// 测试多个 header 标注
    #[tokio::test]
    async fn test_multiple_header_annotations() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/multi-headers")]
        #[header(Cache_Control = "no-cache")]
        #[header(X_Custom_Header = "custom-value")]
        async fn multi_headers_handler() -> HttpResponse {
            HttpResponse::text("multiple headers test")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/multi-headers", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);

        // 验证多个 headers 被正确添加
        let cache_control = res.headers.get("Cache-Control");
        assert!(
            cache_control.is_some(),
            "Cache-Control header should be present"
        );
        assert_eq!(cache_control.unwrap(), "no-cache");

        let custom_header = res.headers.get("X-Custom-Header");
        assert!(custom_header.is_some(), "X-Custom-Header should be present");
        assert_eq!(custom_header.unwrap(), "custom-value");

        println!("✅ Multiple header annotations work");

        server_handle.abort();
        Ok(())
    }

    /// 测试 header 标注与 HttpRequest 参数一起使用
    #[tokio::test]
    async fn test_header_with_http_request() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/req-header")]
        #[header(X_Request_Processed = "true")]
        async fn header_with_request_handler(req: &mut HttpRequest) -> HttpResponse {
            let _ = req.get_client_addr().await;
            HttpResponse::text("request with header")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/req-header", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);

        println!("Request header response headers: {:?}", res.headers);

        let req_header = res.headers.get("X-Request-Processed");
        assert!(
            req_header.is_some(),
            "X-Request-Processed header should be present"
        );
        assert_eq!(req_header.unwrap(), "true");

        println!("✅ Header annotation with HttpRequest works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 header 标注与不同返回类型一起使用
    #[tokio::test]
    async fn test_header_with_different_return_types() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        // 测试 String 返回类型
        #[potato::http_get("/string-return")]
        #[header(X_Return_Type = "string")]
        async fn string_return_handler() -> String {
            "string return".to_string()
        }

        // 测试 Result<HttpResponse> 返回类型
        #[potato::http_get("/result-return")]
        #[header(X_Return_Type = "result")]
        async fn result_return_handler() -> anyhow::Result<HttpResponse> {
            Ok(HttpResponse::text("result return"))
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 测试 String 返回
        let url = format!("http://{}/string-return", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 200);
        println!("String return response headers: {:?}", res.headers);
        let header = res.headers.get("X-Return-Type");
        assert!(
            header.is_some(),
            "X-Return-Type header should be present for String return"
        );
        assert_eq!(header.unwrap(), "string");

        // 测试 Result 返回
        let url = format!("http://{}/result-return", server_addr);
        let res = potato::get(&url, vec![]).await?;
        assert_eq!(res.http_code, 200);
        let header = res.headers.get("X-Return-Type");
        assert!(
            header.is_some(),
            "X-Return-Type header should be present for Result return"
        );
        assert_eq!(header.unwrap(), "result");

        println!("✅ Header annotation with different return types works");

        server_handle.abort();
        Ok(())
    }

    /// 测试 potato::header 标注和 Custom 语法
    #[tokio::test]
    async fn test_potato_header_with_custom_syntax() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        #[potato::http_get("/potato-custom-header")]
        #[potato::header(Authorization = "Bearer TEST_TOKEN")]
        #[potato::header(Custom("X-Custom-Header") = "hello")]
        async fn potato_custom_header_handler() -> HttpResponse {
            HttpResponse::text("potato custom header test")
        }

        let mut server = HttpServer::new(&server_addr);
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        let url = format!("http://{}/potato-custom-header", server_addr);
        let res = potato::get(&url, vec![]).await?;

        assert_eq!(res.http_code, 200);

        // 调试输出：查看所有headers
        println!("Response headers: {:?}", res.headers);

        // 验证 Authorization header 被正确添加
        let auth_header = res.headers.get("Authorization");
        assert!(
            auth_header.is_some(),
            "Authorization header should be present"
        );
        assert_eq!(auth_header.unwrap(), "Bearer TEST_TOKEN");

        // 验证 Custom header 被正确添加
        let custom_header = res.headers.get("X-Custom-Header");
        assert!(custom_header.is_some(), "X-Custom-Header should be present");
        assert_eq!(custom_header.unwrap(), "hello");

        println!("✅ potato::header with Custom syntax works");

        server_handle.abort();
        Ok(())
    }
}
