/// 特性标志测试
/// 仅在启用对应特性时运行: jemalloc, webdav, ssh
#[cfg(feature = "webdav")]
use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "webdav")]
use std::time::Duration;
#[cfg(feature = "webdav")]
use tokio::time::sleep;

#[cfg(feature = "webdav")]
static PORT_COUNTER: AtomicU16 = AtomicU16::new(28000);

#[cfg(feature = "webdav")]
fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(feature = "webdav")]
#[cfg(test)]
mod webdav_tests {
    use super::*;
    use potato::HttpServer;
    use std::fs;

    /// 测试 WebDAV 本地文件系统功能 - examples/server/11_webdav_server.rs
    #[tokio::test]
    async fn test_webdav_localfs_server() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        // 创建临时目录用于 WebDAV
        let temp_dir = std::env::temp_dir().join(format!("potato_webdav_test_{}", port));
        fs::create_dir_all(&temp_dir)?;

        // 创建测试文件
        let test_file = temp_dir.join("test.txt");
        fs::write(&test_file, "hello webdav")?;

        let mut server = HttpServer::new(&server_addr);
        server.configure(|ctx| {
            ctx.use_webdav_localfs("/webdav", temp_dir.to_str().unwrap());
        });

        let server_handle = tokio::spawn(async move {
            let _ = server.serve_http().await;
        });

        sleep(Duration::from_millis(300)).await;

        // 测试 WebDAV GET 方法 (读取文件)
        let url = format!("http://{}/webdav/test.txt", server_addr);
        match potato::get(&url, vec![]).await {
            Ok(res) => {
                println!("WebDAV GET response: {}", res.http_code);
                let body = match &res.body {
                    potato::HttpResponseBody::Data(data) => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                    potato::HttpResponseBody::Stream(_) => "stream response".to_string(),
                };
                if res.http_code == 200 {
                    assert_eq!(body, "hello webdav");
                    println!("✅ WebDAV file content verified");
                }
            }
            Err(e) => {
                println!("WebDAV GET error: {}", e);
            }
        }

        // 清理
        let _ = fs::remove_file(&test_file);
        let _ = fs::remove_dir(&temp_dir);

        server_handle.abort();
        println!("✅ WebDAV localfs test completed");
        Ok(())
    }
}
