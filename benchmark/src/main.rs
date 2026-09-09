use std::{net::SocketAddr, sync::atomic::Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Barrier;
use tokio::time::Instant;

const HELLO: &str = "hello world";

#[potato::http_get("/hello")]
fn hello() -> potato::HttpResponse {
    potato::HttpResponse::html(HELLO)
}

async fn serve_potato(addr: SocketAddr) -> anyhow::Result<()> {
    let mut server = potato::HttpServer::new(addr.to_string());
    server.serve_http().await
}

async fn serve_ntex(addr: SocketAddr) -> std::io::Result<()> {
    ntex::web::HttpServer::new(move || {
        ntex::web::App::new().service(ntex::web::resource("/hello").route(
            ntex::web::get().to(|| async {
                Ok::<_, ntex::web::Error>(ntex::web::HttpResponse::Ok().body(HELLO))
            }),
        ))
    })
    .bind(addr)?
    .run()
    .await
}

fn response_is_hello(buf: &[u8]) -> bool {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let body_len = buf.len() - end - 4;
    body_len == HELLO.len() && buf[buf.len() - HELLO.len()..] == *HELLO.as_bytes()
}

async fn bench_connection(
    addr: SocketAddr,
    requests: u64,
    barrier: std::sync::Arc<Barrier>,
    counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> anyhow::Result<()> {
    let mut stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    let request = format!("GET /hello HTTP/1.1\r\nHost: {addr}\r\nConnection: keep-alive\r\n\r\n");
    let mut buf = Vec::with_capacity(16 * 1024);
    barrier.wait().await;

    for _ in 0..requests {
        stream.write_all(request.as_bytes()).await?;
        loop {
            let mut chunk = [0_u8; 16 * 1024];
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                anyhow::bail!("connection closed");
            }
            buf.extend_from_slice(&chunk[..n]);
            if response_is_hello(&buf) {
                counter.fetch_add(1, Ordering::Relaxed);
                buf.clear();
                break;
            }
        }
    }
    Ok(())
}

async fn run_bench(addr: SocketAddr, connections: usize, requests: u64) -> anyhow::Result<()> {
    let total_requests = connections as u64 * requests;
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let barrier = std::sync::Arc::new(Barrier::new(connections + 1));
    let mut tasks = tokio::task::JoinSet::new();

    for _ in 0..connections {
        let (addr, requests, barrier, counter) = (addr, requests, barrier.clone(), counter.clone());
        tasks.spawn(bench_connection(addr, requests, barrier, counter));
    }

    let started = Instant::now();
    barrier.wait().await;
    let mut errors = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Err(err) = joined? {
            errors.push(err.to_string());
        }
    }
    let elapsed = started.elapsed();
    if !errors.is_empty() {
        anyhow::bail!(
            "{} bench connections failed; first error: {}",
            errors.len(),
            errors[0]
        );
    }
    println!(
        "qps={} completed={total_requests} elapsed={elapsed:.3?} connections={connections}",
        total_requests as f64 / elapsed.as_secs_f64()
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());

    let (addr, framework, connections, requests) = match command.as_str() {
        "serve" => {
            let framework = args.next().unwrap_or_default();
            let port = args
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(18080_u16);
            (SocketAddr::from(([127, 0, 0, 1], port)), framework, 0, 0)
        }
        "bench" => {
            let port = args
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(18080_u16);
            let connections = args
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(128_usize);
            let requests = args.next().and_then(|v| v.parse().ok()).unwrap_or(5000);
            (
                SocketAddr::from(([127, 0, 0, 1], port)),
                "ignored".to_string(),
                connections,
                requests,
            )
        }
        _ => (
            SocketAddr::from(([127, 0, 0, 1], 18080)),
            String::new(),
            0,
            0,
        ),
    };

    if command == "serve" && framework == "ntex" {
        return ntex_main(addr);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()?;
    match command.as_str() {
        "serve" => runtime.block_on(serve_potato(addr)),
        "bench" => runtime.block_on(run_bench(addr, connections, requests)),
        _ => {
            eprintln!("Usage:");
            eprintln!("  worker serve <potato|ntex> [port]");
            eprintln!("  worker bench ignored [connections] [requests-per-connection]");
            Ok(())
        }
    }
}

#[ntex::main]
async fn ntex_main(addr: SocketAddr) -> anyhow::Result<()> {
    Ok(serve_ntex(addr).await?)
}
