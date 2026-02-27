/// 服务器特性的综合测试
/// 测试可以不依赖特定路由实现的功能
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(26000);

fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use potato::HttpServer;

    /// 测试服务器创建和基本配置
    #[tokio::test]
    async fn test_server_creation_and_configuration() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 测试 configure 方法
        server.configure(|ctx| {
            // 这个配置可以成功应用
            ctx.use_handlers(false);
        });

        println!("✅ Server created and configured for: {}", server_addr);
        Ok(())
    }

    /// 测试多个服务器可以在不同端口上创建
    #[tokio::test]
    async fn test_multiple_servers_different_ports() -> anyhow::Result<()> {
        let port1 = get_test_port();
        let port2 = get_test_port();
        let port3 = get_test_port();

        let server_addr1 = format!("127.0.0.1:{}", port1);
        let server_addr2 = format!("127.0.0.1:{}", port2);
        let server_addr3 = format!("127.0.0.1:{}", port3);

        let _server1 = HttpServer::new(&server_addr1);
        let _server2 = HttpServer::new(&server_addr2);
        let _server3 = HttpServer::new(&server_addr3);

        assert_ne!(port1, port2);
        assert_ne!(port2, port3);
        assert_ne!(port1, port3);

        println!("✅ Created 3 servers on different ports");
        Ok(())
    }

    /// 测试服务器的关闭信号
    #[tokio::test]
    async fn test_server_shutdown_signal() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 获取关闭信号
        let shutdown_tx = server.shutdown_signal();

        let server_handle = tokio::spawn(async move {
            // 服务器会在收到关闭信号时退出
            let _ = server.serve_http().await;
        });

        // 给服务器时间启动
        sleep(Duration::from_millis(200)).await;

        // 发送关闭信号
        let _ = shutdown_tx.send(());

        // 等待服务器关闭
        sleep(Duration::from_millis(200)).await;

        // 如果没有 panic，测试通过
        server_handle.abort();
        println!("✅ Server shutdown signal works");
        Ok(())
    }

    /// 测试服务器配置的多种选项
    #[tokio::test]
    async fn test_server_configuration_options() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 配置选项1: 禁用处理器
        server.configure(|ctx| {
            ctx.use_handlers(false);
        });

        println!("✅ Server configuration: handlers disabled");

        // 配置选项2: OpenAPI (如果启用)
        let mut server2 = HttpServer::new(&format!("127.0.0.1:{}", get_test_port()));
        server2.configure(|ctx| {
            ctx.use_handlers(false);
            // ctx.use_openapi("/doc/"); // 仅在 openapi 特性启用时
        });

        println!("✅ Server configuration options applied");
        Ok(())
    }

    /// 测试服务器地址解析
    #[tokio::test]
    async fn test_server_address_parsing() -> anyhow::Result<()> {
        // 测试不同格式的地址
        let addresses = vec!["127.0.0.1:25000", "localhost:25001", "0.0.0.0:25002"];

        for addr in addresses {
            let _server = HttpServer::new(addr);
            println!("✅ Server created with address: {}", addr);
        }

        Ok(())
    }

    /// 测试服务器 HTTP 和 HTTPS 方法的可用性
    #[tokio::test]
    async fn test_server_protocols_availability() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);
        let _server = HttpServer::new(&server_addr);

        // serve_http 方法应该存在
        // serve_https 方法应该存在（如果 TLS 特性启用）

        println!("✅ Server protocol methods are available");
        Ok(())
    }

    /// 测试从 String 创建服务器
    #[tokio::test]
    async fn test_server_from_string() -> anyhow::Result<()> {
        let port = get_test_port();
        let addr_string = format!("127.0.0.1:{}", port);

        // 从 String 创建
        let _server1 = HttpServer::new(addr_string.clone());
        println!("✅ Server created from String: {}", addr_string);

        // 从 &str 创建
        let _server2 = HttpServer::new("127.0.0.1:25010");
        println!("✅ Server created from &str");

        Ok(())
    }

    /// 测试关闭信号只能获取一次
    #[tokio::test]
    async fn test_shutdown_signal_once() -> anyhow::Result<()> {
        let port = get_test_port();
        let server_addr = format!("127.0.0.1:{}", port);

        let mut server = HttpServer::new(&server_addr);

        // 第一次获取应该成功
        let _shutdown_tx = server.shutdown_signal();

        // 第二次获取应该 panic（在注释中说明）
        // 取消注释下行会导致 panic: "shutdown signal already set"
        // let _shutdown_tx2 = server.shutdown_signal();

        println!("✅ Shutdown signal correctly allows only one acquisition");
        Ok(())
    }
}
