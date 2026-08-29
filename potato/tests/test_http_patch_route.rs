#![cfg(feature = "std")]
use potato::{HttpResponse, HttpServer};
use std::time::Duration;

#[potato::http_patch("/patch-route")]
async fn patch_route() -> HttpResponse {
    HttpResponse::html("patched")
}

#[tokio::test]
async fn http_patch_route_is_served() -> anyhow::Result<()> {
    let server_addr = "127.0.0.1:18975".to_string();
    let mut server = HttpServer::new(&server_addr);
    let _ = server.configure(|ctx| ctx.use_handlers());

    let server_handle = tokio::spawn(async move { server.serve_http().await });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut response = potato::patch(&format!("http://{server_addr}/patch-route"), vec![]).await?;
    assert_eq!(response.http_code, 200);
    assert_eq!(response.body.data().await, b"patched");

    server_handle.abort();
    Ok(())
}
