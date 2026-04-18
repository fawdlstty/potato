//! 测试 preprocess/postprocess 使用 SessionCache 时的编译时检查

// 测试1: preprocess 使用 SessionCache，handler 也使用 - 应该编译成功
mod test_valid_session_cache_in_preprocess {
    use potato::{http_get, preprocess};

    #[preprocess]
    async fn my_preprocess_with_session(
        _req: &mut potato::HttpRequest,
        _session_cache: &mut potato::SessionCache,
    ) {
    }

    #[http_get("/")]
    #[preprocess(my_preprocess_with_session)]
    async fn handler_with_session(
        _req: &mut potato::HttpRequest,
        _session_cache: &mut potato::SessionCache,
    ) -> potato::HttpResponse {
        potato::HttpResponse::text("OK")
    }
}

// 测试2: postprocess 使用 SessionCache，handler 也使用 - 应该编译成功
mod test_valid_session_cache_in_postprocess {
    use potato::{http_get, postprocess};

    #[postprocess]
    async fn my_postprocess_with_session(
        _req: &mut potato::HttpRequest,
        _res: &mut potato::HttpResponse,
        _session_cache: &mut potato::SessionCache,
    ) {
    }

    #[http_get("/")]
    #[postprocess(my_postprocess_with_session)]
    async fn handler_with_session(
        _req: &mut potato::HttpRequest,
        _session_cache: &mut potato::SessionCache,
    ) -> potato::HttpResponse {
        potato::HttpResponse::text("OK")
    }
}

// 测试3: preprocess 不使用 SessionCache，handler 也不使用 - 应该编译成功
mod test_no_session_cache {
    use potato::{http_get, preprocess};

    #[preprocess]
    async fn my_preprocess_no_session(_req: &mut potato::HttpRequest) {}

    #[http_get("/")]
    #[preprocess(my_preprocess_no_session)]
    async fn handler_no_session(_req: &mut potato::HttpRequest) -> potato::HttpResponse {
        potato::HttpResponse::text("OK")
    }
}

// 测试4: 编译失败测试 - preprocess 使用 SessionCache 但 handler 没有
// 这个测试应该在编译时失败
// 取消注释下面的代码应该会导致编译错误：
//
// mod test_invalid_preprocess_without_handler_session {
//     use potato::{http_get, preprocess};
//
//     #[preprocess]
//     async fn my_preprocess_with_session(
//         _req: &mut potato::HttpRequest,
//         _session_cache: &mut potato::SessionCache,
//     ) {
//     }
//
//     #[http_get("/", preprocess = "my_preprocess_with_session")]
//     async fn handler_without_session(_req: &mut potato::HttpRequest) -> potato::HttpResponse {
//         potato::HttpResponse::text("OK")
//     }
// }

// 测试5: 编译失败测试 - postprocess 使用 SessionCache 但 handler 没有
// 这个测试应该在编译时失败
// 取消注释下面的代码应该会导致编译错误：
//
// mod test_invalid_postprocess_without_handler_session {
//     use potato::{http_get, postprocess};
//
//     #[postprocess]
//     async fn my_postprocess_with_session(
//         _req: &mut potato::HttpRequest,
//         _res: &mut potato::HttpResponse,
//         _session_cache: &potato::SessionCache,
//     ) {
//     }
//
//     #[http_get("/", postprocess = "my_postprocess_with_session")]
//     async fn handler_without_session(_req: &mut potato::HttpRequest) -> potato::HttpResponse {
//         potato::HttpResponse::text("OK")
//     }
// }

#[tokio::test]
async fn test_session_cache_compile_check() {
    // 如果这个文件编译成功，说明测试通过
    println!("✅ SessionCache compile check test passed");
}
