#[cfg(test)]
mod tests {
    use potato::{HttpRequest, HttpResponse, HttpServer, SessionCache};

    #[potato::preprocess]
    fn my_preprocess(_req: &mut HttpRequest) -> anyhow::Result<()> {
        Ok(())
    }

    #[potato::controller]
    pub struct UsersController<'a> {
        pub once_cache: &'a potato::OnceCache,
        pub sess_cache: &'a SessionCache,
    }

    #[potato::controller("/api/users")]
    #[potato::preprocess(my_preprocess)]
    impl<'a> UsersController<'a> {
        #[potato::http_get]
        pub async fn get(&self) -> anyhow::Result<&'static str> {
            Ok("get users data")
        }
    }

    #[cfg(feature = "openapi")]
    #[tokio::test]
    async fn test_controller_tag_without_lifetime() {
        use std::time::Duration;

        let port = 18081;
        let server_addr = format!("127.0.0.1:{}", port);

        // 启动服务器
        let mut server = HttpServer::new(&server_addr);
        server.configure(|ctx| {
            ctx.use_handlers();
            ctx.use_openapi("/doc");
        });

        // 在后台启动服务器
        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        // 等待服务器启动
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 获取 OpenAPI JSON
        let url = format!("http://{}/doc/index.json", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                let swagger_json = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8_lossy(data).to_string()
                    }
                    _ => panic!("Unexpected response body type"),
                };

                let json: serde_json::Value =
                    serde_json::from_str(&swagger_json).expect("Failed to parse JSON");

                // 验证 tags 字段
                let tags = json.get("tags").expect("No tags in OpenAPI JSON");
                let tag_array = tags.as_array().expect("Tags is not an array");

                // 应该只有一个 tag
                assert_eq!(tag_array.len(), 1, "Expected exactly 1 tag");

                // 验证 tag 名称不包含生命周期参数
                let tag_name = tag_array[0].get("name").expect("Tag has no name");
                let tag_name_str = tag_name.as_str().expect("Tag name is not a string");

                // 关键验证：tag 名称应该是 "UsersController" 而不是 "UsersController < 'a >"
                assert_eq!(
                    tag_name_str, "UsersController",
                    "Tag name should be 'UsersController' without lifetime parameters, but got: {}",
                    tag_name_str
                );

                // 验证 paths 中的 tags 也不包含生命周期参数
                let paths = json.get("paths").expect("No paths in OpenAPI JSON");
                let api_users = paths.get("/api/users").expect("No /api/users path");
                let get_method = api_users.get("get").expect("No GET method");
                let method_tags = get_method.get("tags").expect("No tags in GET method");
                let method_tag_array = method_tags.as_array().expect("Method tags is not an array");

                assert_eq!(
                    method_tag_array.len(),
                    1,
                    "Expected exactly 1 tag in method"
                );
                let method_tag = method_tag_array[0]
                    .as_str()
                    .expect("Method tag is not a string");

                assert_eq!(
                    method_tag, "UsersController",
                    "Method tag should be 'UsersController' without lifetime parameters, but got: {}",
                    method_tag
                );
            }
            Err(e) => panic!("Failed to get OpenAPI JSON: {}", e),
        }

        // 清理：取消服务器任务
        server_handle.abort();
    }
}
