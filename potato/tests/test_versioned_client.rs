//! 测试 HTTP 协议版本选择功能

#[cfg(test)]
mod versioned_client_tests {
    #[test]
    fn test_http_version_enum_exists() {
        // 验证 HttpVersion 枚举存在
        let _v11 = potato::client::HttpVersion::Http11;

        #[cfg(feature = "http2")]
        let _v2 = potato::client::HttpVersion::Http2;

        #[cfg(feature = "http3")]
        let _v3 = potato::client::HttpVersion::Http3;
    }

    #[test]
    fn test_versioned_url_functions() {
        // 测试 URL 包装器函数
        let url1 = potato::client::http11("https://example.com");
        assert_eq!(url1.url, "https://example.com");

        #[cfg(feature = "http2")]
        {
            let url2 = potato::client::http2("https://example.com");
            assert_eq!(url2.url, "https://example.com");
        }

        #[cfg(feature = "http3")]
        {
            let url3 = potato::client::http3("https://example.com");
            assert_eq!(url3.url, "https://example.com");
        }
    }

    #[test]
    fn test_macro_detect_url_version() {
        // 这个测试验证宏能够正确展开
        // 实际的网络请求测试需要在有网络的环境下进行
        use potato::__potato_detect_url_version;

        // 测试默认情况（HTTP/1.1）
        let _url1 = __potato_detect_url_version!("https://example.com");

        // 测试 http11() 包装器
        let _url2 = __potato_detect_url_version!(http11("https://example.com"));

        #[cfg(feature = "http2")]
        {
            // 测试 http2() 包装器
            let _url3 = __potato_detect_url_version!(http2("https://example.com"));
        }

        #[cfg(feature = "http3")]
        {
            // 测试 http3() 包装器
            let _url4 = __potato_detect_url_version!(http3("https://example.com"));
        }
    }
}
