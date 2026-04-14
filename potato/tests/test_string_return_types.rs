/// 测试 String 和 &'static str 返回类型支持

#[test]
fn test_string_return_type() {
    // 测试异步函数返回 String
    #[potato::http_get("/test-string")]
    async fn handler_string() -> String {
        "<html><body><h1>Hello</h1></body></html>".to_string()
    }

    // 这个测试主要验证宏能正确编译
    assert!(true);
}

#[test]
fn test_static_str_return_type() {
    // 测试异步函数返回 &'static str
    #[potato::http_get("/test-static-str")]
    async fn handler_static_str() -> &'static str {
        "<html><body><h1>Hello</h1></body></html>"
    }

    assert!(true);
}

#[test]
fn test_result_string_return_type() {
    // 测试异步函数返回 anyhow::Result<String>
    #[potato::http_get("/test-result-string")]
    async fn handler_result_string() -> anyhow::Result<String> {
        Ok("<html><body><h1>Hello</h1></body></html>".to_string())
    }

    assert!(true);
}

#[test]
fn test_result_static_str_return_type() {
    // 测试异步函数返回 anyhow::Result<&'static str>
    #[potato::http_get("/test-result-static-str")]
    async fn handler_result_static_str() -> anyhow::Result<&'static str> {
        Ok("<html><body><h1>Hello</h1></body></html>")
    }

    assert!(true);
}

#[test]
fn test_sync_string_return_type() {
    // 测试同步函数返回 String
    #[potato::http_get("/test-string-sync")]
    fn handler_string_sync() -> String {
        "<html><body><h1>Hello</h1></body></html>".to_string()
    }

    assert!(true);
}

#[test]
fn test_sync_static_str_return_type() {
    // 测试同步函数返回 &'static str
    #[potato::http_get("/test-static-str-sync")]
    fn handler_static_str_sync() -> &'static str {
        "<html><body><h1>Hello</h1></body></html>"
    }

    assert!(true);
}

#[test]
fn test_sync_result_string_return_type() {
    // 测试同步函数返回 anyhow::Result<String>
    #[potato::http_get("/test-result-string-sync")]
    fn handler_result_string_sync() -> anyhow::Result<String> {
        Ok("<html><body><h1>Hello</h1></body></html>".to_string())
    }

    assert!(true);
}

#[test]
fn test_sync_result_static_str_return_type() {
    // 测试同步函数返回 anyhow::Result<&'static str>
    #[potato::http_get("/test-result-static-str-sync")]
    fn handler_result_static_str_sync() -> anyhow::Result<&'static str> {
        Ok("<html><body><h1>Hello</h1></body></html>")
    }

    assert!(true);
}
