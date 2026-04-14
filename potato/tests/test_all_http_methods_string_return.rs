/// 测试所有 HTTP 方法宏都支持 String 返回类型

#[test]
fn test_http_post_string_return() {
    #[potato::http_post("/test-post")]
    async fn handler_post() -> String {
        "POST response".to_string()
    }
    assert!(true);
}

#[test]
fn test_http_put_string_return() {
    #[potato::http_put("/test-put")]
    async fn handler_put() -> String {
        "PUT response".to_string()
    }
    assert!(true);
}

#[test]
fn test_http_delete_string_return() {
    #[potato::http_delete("/test-delete")]
    async fn handler_delete() -> String {
        "DELETE response".to_string()
    }
    assert!(true);
}

#[test]
fn test_http_options_string_return() {
    #[potato::http_options("/test-options")]
    async fn handler_options() -> String {
        "OPTIONS response".to_string()
    }
    assert!(true);
}

#[test]
fn test_http_head_string_return() {
    #[potato::http_head("/test-head")]
    async fn handler_head() -> String {
        "HEAD response".to_string()
    }
    assert!(true);
}

#[test]
fn test_http_post_static_str_return() {
    #[potato::http_post("/test-post-static")]
    async fn handler_post_static() -> &'static str {
        "POST static str response"
    }
    assert!(true);
}

#[test]
fn test_http_put_result_string_return() {
    #[potato::http_put("/test-put-result")]
    async fn handler_put_result() -> anyhow::Result<String> {
        Ok("PUT result response".to_string())
    }
    assert!(true);
}
