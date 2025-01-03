use potato::*;

// async fn common_handler(req: HttpRequest) -> Option<HttpResponse> {
//     match req.uri.query().unwrap_or("").len() > 3 {
//         true => Some(HttpResponse::html("hello middleware")),
//         false => None,
//     }
// }

// AAAAAAAAAAAAAAAA
// BBBBBBBBBBBBBBBB
#[http_get("/hello")]
async fn hello(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello world, {name}!"))
}

#[http_post("/test")]
async fn test(name: String) -> HttpResponse {
    HttpResponse::html("hello")
}

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
                };
            </script>
        </body>
    </html>"#,
    )
}

#[http_get("/ws")]
async fn websocket(req: HttpRequest, wsctx: &mut WebsocketContext) -> anyhow::Result<()> {
    let mut ws = wsctx.upgrade_websocket(&req).await?;
    ws.send_ping().await?;
    loop {
        match ws.recv_frame().await? {
            WsFrame::Text(text) => ws.send_text(&text).await?,
            WsFrame::Binary(bin) => ws.send_binary(bin).await?,
        }
    }
}

declare_doc_path!("/doc/");

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    //server.set_static_path("E:\\", "/");
    _ = server.serve_http().await;
    // _ = server.serve_https("cert.pem", "key.pem").await;
}

// cargo run -p potato
// cargo publish -p potato-macro --registry crates-io --allow-dirty
// cargo publish -p potato --registry crates-io --allow-dirty
