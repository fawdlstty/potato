#[tokio::main]
async fn main() {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_custom_async({
            |req| {
                Box::pin(async move {
                    let mut sess =
                        potato::TransferSession::from_reverse_proxy("/", "http://127.0.0.1:8080");
                    if let Err(err) = sess.with_ssh_jumpbox(&potato::SshJumpboxInfo {
                        host: "192.168.0.100".to_string(),
                        port: 22,
                        username: "root".to_string(),
                        password: "root".to_string(),
                    })
                    .await
                    {
                        return Some(potato::HttpResponse::error(format!("{err}")));
                    }

                    match sess.transfer(req, true).await {
                        Ok(res) => Some(res),
                        Err(err) => Some(potato::HttpResponse::error(format!("{err}"))),
                    }
                })
            }
        });
    });
    _ = server.serve_http().await;
}
