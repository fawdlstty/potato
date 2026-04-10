#[potato::preprocess]
async fn auth_guard(req: &mut potato::HttpRequest) -> anyhow::Result<Option<potato::HttpResponse>> {
    let token = req
        .url_query
        .get(&potato::hipstr::LocalHipStr::from("token"))
        .map(|v| v.as_str())
        .unwrap_or_default();
    if token != "ok" {
        let mut res = potato::HttpResponse::text("unauthorized");
        res.http_code = 401;
        return Ok(Some(res));
    }
    Ok(None)
}

#[potato::postprocess]
fn add_server_mark(_req: &mut potato::HttpRequest, res: &mut potato::HttpResponse) {
    res.add_header("X-Server-Mark".into(), "potato-hooks".into());
}

#[potato::postprocess]
async fn append_signature(
    _req: &mut potato::HttpRequest,
    res: &mut potato::HttpResponse,
) -> anyhow::Result<()> {
    if let potato::HttpResponseBody::Data(body) = &mut res.body {
        body.extend_from_slice(b"\n-- postprocess --");
    }
    Ok(())
}

#[potato::http_get("/hello")]
#[potato::preprocess(auth_guard)]
#[potato::postprocess(add_server_mark)]
#[potato::postprocess(append_signature)]
async fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html("hello world")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    println!("visit: http://127.0.0.1:8080/hello?token=ok");
    server.serve_http().await
}
