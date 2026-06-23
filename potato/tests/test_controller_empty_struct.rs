use potato::HttpServer;

#[potato::controller]
struct EmptyController {}

#[potato::controller("/api/empty")]
impl EmptyController {
    #[potato::http_get("/ping")]
    pub async fn ping(&self) -> anyhow::Result<&'static str> {
        Ok("pong")
    }
}

#[tokio::test]
async fn test_controller_empty_struct() {
    let port = 18892;
    let server_addr = format!("127.0.0.1:{port}");

    let mut server = HttpServer::new(&server_addr);
    server.configure(|ctx| {
        ctx.use_handlers();
    });

    let server_handle = tokio::spawn(async move {
        let _ = server.serve_http().await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url = format!("http://{server_addr}/api/empty/ping");
    let res = potato::get(&url, vec![]).await.expect("request failed");
    assert_eq!(res.http_code, 200);

    server_handle.abort();
}
