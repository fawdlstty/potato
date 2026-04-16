/// 示例：演示如何使用 #[header(...)] 标注为HTTP handler添加响应头
use potato::{HttpServer, HttpRequest, HttpResponse};

// 示例1：添加单个header
#[potato::http_get("/single")]
#[header(Cache_Control = "no-store, no-cache, max-age=0")]
async fn single_header() -> HttpResponse {
    HttpResponse::text("Single header example")
}

// 示例2：添加多个header
#[potato::http_get("/multiple")]
#[header(Cache_Control = "no-cache")]
#[header(X_Custom_Header = "custom-value")]
#[header(X_Another_Header = "another-value")]
async fn multiple_headers() -> HttpResponse {
    HttpResponse::text("Multiple headers example")
}

// 示例3：header与其他返回类型一起使用 - String
#[potato::http_get("/string-return")]
#[header(X_Response_Type = "string")]
async fn string_return() -> String {
    "String return with header".to_string()
}

// 示例4：header与HttpRequest参数一起使用
#[potato::http_get("/with-request")]
#[header(X_Processed = "true")]
async fn with_request(req: &mut HttpRequest) -> HttpResponse {
    let addr = req.get_client_addr().await.map(|a| a.to_string()).unwrap_or("unknown".to_string());
    HttpResponse::text(format!("Request from: {addr}"))
}

// 示例5：header与Result返回类型一起使用
#[potato::http_get("/result-return")]
#[header(X_Has_Error_Handling = "true")]
async fn result_return(success: bool) -> anyhow::Result<HttpResponse> {
    if success {
        Ok(HttpResponse::text("Success"))
    } else {
        anyhow::bail!("Something went wrong")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("HTTP Header Annotation Examples");
    println!("================================");
    println!();
    println!("Available endpoints:");
    println!("  GET /single          - Single header example");
    println!("  GET /multiple        - Multiple headers example");
    println!("  GET /string-return   - String return with header");
    println!("  GET /with-request    - Header with HttpRequest parameter");
    println!("  GET /result-return   - Header with Result return type");
    println!();
    
    let mut server = HttpServer::new("127.0.0.1:8080");
    println!("Server starting on http://127.0.0.1:8080");
    println!();
    
    server.serve_http().await?;
    Ok(())
}
