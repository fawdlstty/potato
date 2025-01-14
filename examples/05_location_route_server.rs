use potato::*;

#[tokio::main]
async fn main() {
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_location_route("/", "/wwwroot");
    });
    println!("visit: https://127.0.0.1:8080/");
    _ = server.serve_http().await;
}
