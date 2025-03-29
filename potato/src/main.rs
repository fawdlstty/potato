use global_config::ServerConfig;
use potato::*;
//use tikv_jemalloc_ctl::{Access, AsName};

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
    let token = ServerAuth::jwt_issue(payload, std::time::Duration::from_secs(10000000)).await?;
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

//

// fn jemalloc_active_prof(active: bool) -> bool {
//     const PROF_ACTIVE: &'static [u8] = b"prof.active\0";
//     let name = PROF_ACTIVE.name();
//     match name.write(active) {
//         Ok(()) => {
//             println!("[main] active({active}) jemalloc prof success");
//             true
//         }
//         Err(err) => {
//             println!("[main] active({active}) jemalloc prof failed: {err}");
//             false
//         }
//     }
// }

// #[global_allocator]
// static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// #[allow(non_upper_case_globals)]
// #[export_name = "malloc_conf"]
// pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

// #[http_get("/heap.pb.gz")]
// pub async fn heap_pb_gz() -> anyhow::Result<HttpResponse> {
//     let mut prof_ctl = jemalloc_pprof::PROF_CTL.as_ref().unwrap().lock().await;
//     require_profiling_activated(&prof_ctl)?;
//     let data = prof_ctl.dump_pprof()?;
//     Ok(HttpResponse::from_mem_file("heap.bp.gz", data, true))
// }

// #[http_get("/ds.prof")]
// pub async fn ds_prof() -> anyhow::Result<HttpResponse> {
//     jemalloc_active_prof(false);
//     jemalloc_dump_profile("ds.prof");
//     Ok(HttpResponse::html("ds.prof"))
// }

// fn require_profiling_activated(prof_ctl: &jemalloc_pprof::JemallocProfCtl) -> anyhow::Result<()> {
//     if prof_ctl.activated() {
//         Ok(())
//     } else {
//         Err(anyhow::Error::msg("heap profiling not activated"))
//     }
// }

// fn jemalloc_dump_profile(prof_name: &str) -> bool {
//     const PROF_DUMP: &'static [u8] = b"prof.dump\0";
//     let mut prof_name2 = prof_name.to_string();
//     prof_name2.push('\0');
//     let prof_name2 = prof_name2.into_boxed_str();
//     let prof_name_ptr: &'static [u8] = unsafe { std::mem::transmute(prof_name2) };
//     let name = PROF_DUMP.name();
//     match name.write(prof_name_ptr) {
//         Ok(()) => {
//             println!("[main] dump jemalloc prof file[{prof_name}] success");
//             true
//         }
//         Err(err) => {
//             println!("[main] dump jemalloc prof file[{prof_name}] failed: {err}");
//             false
//         }
//     }
// }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    //jemalloc_active_prof(true);
    ServerConfig::set_jwt_secret("AAABBBCCC").await;
    let mut server = HttpServer::new("0.0.0.0:8080");
    server.configure(|ctx| {
        ctx.use_dispatch();
        ctx.use_doc("/doc/");
        //ctx.use_embedded_route("/", embed_dir!("assets/wwwroot"));
        //ctx.use_location_route("/", "/wwwroot");
    });
    println!("visit: http://127.0.0.1:8080/doc/");
    server.serve_http().await
    let res = potato::get(
        "https://www.fawdlstty.com",
        vec![Headers::User_Agent("aaa".into())],
    )
    .await?;
    Ok(())
}

// cargo run -p potato
// cargo publish -p potato-macro --registry crates-io --allow-dirty
// cargo publish -p potato --registry crates-io --allow-dirty
