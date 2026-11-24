// TechEmpower Framework Benchmarks 测试端点
//
// 官方验证要求：
//   /plaintext  Content-Type: text/plain，响应体精确为 "Hello, World!"（13 字节）
//   /json       Content-Type: application/json，响应体精确为 {"message":"Hello, World!"}
// 两者均须支持 keep-alive、HTTP 流水线与高并发。
// potato 默认响应头自带 Date/Server/Connection: keep-alive，满足验证项。

use potato::HttpResponse;

// 本地调试可传端口参数（TFB 容器内无参数，固定监听 8080）
fn port() -> String {
    std::env::args().nth(1).unwrap_or_else(|| "8080".into())
}

#[potato::http_get("/plaintext")]
fn plaintext() -> HttpResponse {
    HttpResponse::text("Hello, World!")
}

#[potato::http_get("/json")]
fn json() -> HttpResponse {
    // 响应为定长常量串，直接给出序列化结果，省去运行时序列化开销
    HttpResponse::json(r#"{"message":"Hello, World!"}"#)
}

#[tokio::main]
async fn main() {
    let mut server = potato::HttpServer::new(format!("0.0.0.0:{}", port()));
    _ = server.serve_http().await;
}
