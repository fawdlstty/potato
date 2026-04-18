//! 测试编译时检查：preprocess 使用 SessionCache 但 handler 没有声明
//! 这个文件应该编译失败

use potato::{http_get, preprocess};

#[preprocess]
async fn my_preprocess_with_session(
    _req: &mut potato::HttpRequest,
    _session_cache: &mut potato::SessionCache,
) {
}

// 这个 handler 没有声明 SessionCache 参数，但 preprocess 使用了
// 应该导致编译错误
#[http_get("/")]
#[preprocess(my_preprocess_with_session)]
async fn handler_without_session(_req: &mut potato::HttpRequest) -> potato::HttpResponse {
    potato::HttpResponse::text("OK")
}

fn main() {}
