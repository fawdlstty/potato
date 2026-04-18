/// Controller 功能测试示例

#[potato::preprocess]
fn my_preprocess(_req: &mut potato::HttpRequest) -> anyhow::Result<()> {
    println!("Preprocess called");
    Ok(())
}

#[potato::controller("/api/users")]
pub struct UsersController<'a> {
    pub once_cache: &'a potato::OnceCache,
    pub sess_cache: &'a potato::SessionCache,
}

#[potato::preprocess(my_preprocess)]
impl<'a> UsersController<'a> {
    #[potato::http_get("/")]  // 地址为 "/api/users/"
    pub async fn get(&self) -> anyhow::Result<&'static str> {
        Ok("get users data")
    }

    #[potato::http_post("/")] // 地址为 "/api/users/"
    pub async fn post(&mut self) -> anyhow::Result<&'static str> {
        Ok("post users data")
    }

    #[potato::http_get("/any")] // 地址为 "/api/users/any"
    pub async fn get_any(&self) -> anyhow::Result<&'static str> {
        Ok("get users any data")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_handlers(true);
        ctx.use_openapi("/doc/");
    });
    println!("visit: http://127.0.0.1:8080/api/users/");
    println!("visit: http://127.0.0.1:8080/api/users/any");
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
}
