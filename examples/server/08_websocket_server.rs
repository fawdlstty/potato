use potato::*;

#[http_get("/")]
async fn index() -> HttpResponse {
    HttpResponse::html(
        r#"<!DOCTYPE html><html>
        <head><title>Websocket Test</title></head>
        <body>
            <h1>Websocket Test</h1>
            <div id="status"><p><em>Connecting...</em></p></div>
            <script>
                const status = document.getElementById('status');
                const ws = new WebSocket(`ws://${location.host}/ws`);
                ws.onopen = function() {
                    status.innerHTML = '<p><em>Connected!</em></p>';
                    ws.send('hello world1');
                    ws.send('hello world2');
                    ws.send('hello world3');
                };
            </script>
        </body>
    </html>"#,
    )
}

#[http_get("/ws")]
async fn websocket(req: &mut HttpRequest) -> anyhow::Result<()> {
    let mut ws = req.upgrade_websocket().await?;
    ws.send_ping().await?;
    loop {
        match ws.recv_frame().await? {
            WsFrame::Text(text) => ws.send_text(&text).await?,
            WsFrame::Binary(bin) => ws.send_binary(bin).await?,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/");
    server.serve_http().await
}
