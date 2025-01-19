use potato::*;

#[http_get("/hello")]
async fn hello() -> HttpResponse {
    HttpResponse::html("hello world")
}

#[http_get("/hello_name")]
async fn hello_name(name: String) -> HttpResponse {
    HttpResponse::html(format!("hello world {name}"))
}

#[http_post("/upload")]
async fn upload(file1: PostFile) -> HttpResponse {
    HttpResponse::html(format!(
        "file[{}] len: {}",
        file1.filename,
        file1.data.to_buf().len()
    ))
}

#[http_get("/issue")]
async fn issue(payload: String) -> anyhow::Result<HttpResponse> {
    let token = server::JwtAuth::issue(payload, std::time::Duration::from_secs(10000000)).await?;
    Ok(HttpResponse::html(token))
}

#[http_get(path="/check", auth_arg=payload)]
async fn check(payload: String) -> HttpResponse {
    HttpResponse::html(format!("payload: [{payload}]"))
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::JwtAuth::set_secret("AAAAAAAAAAAAAAABBBCCC").await;
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_dispatch();
        ctx.use_doc("/doc/");
        //ctx.use_embedded_route("/", embed_dir!("assets/wwwroot"));
        //ctx.use_location_route("/", "/wwwroot");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}

// cargo run -p potato
// cargo publish -p potato-macro --registry crates-io --allow-dirty
// cargo publish -p potato --registry crates-io --allow-dirty
