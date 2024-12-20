use crate::{HttpConnection, RequestHandlerFlag};
use crate::{HttpMethod, HttpResponse};
use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

lazy_static! {
    pub static ref HANDLERS: HashMap<&'static str, HashMap<HttpMethod, &'static RequestHandlerFlag>> = {
        let mut handlers = HashMap::new();
        for flag in inventory::iter::<RequestHandlerFlag> {
            handlers
                .entry(flag.path)
                .or_insert_with(HashMap::new)
                .insert(flag.method, flag);
        }
        handlers
    };
}

pub struct HttpServer {
    pub addr: String,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer { addr: addr.into() }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            _ = tokio::task::spawn(async move {
                let mut conn = HttpConnection {
                    stream,
                    upgrade_ws: false,
                };
                loop {
                    let req = match conn.read_request(client_addr).await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();

                    // call process pipes
                    let res = if let Some(path_handlers) = HANDLERS.get(req.uri.path()) {
                        if let Some(&handler) = path_handlers.get(&req.method) {
                            (handler.handler)(req, &mut conn).await
                        } else if req.method == HttpMethod::HEAD {
                            HttpResponse::html("")
                        } else if req.method == HttpMethod::OPTIONS {
                            let mut res = HttpResponse::html("");
                            res.add_header("Allow", {
                                let mut options: HashSet<_> =
                                    path_handlers.keys().map(|p| *p).collect();
                                options.extend([HttpMethod::HEAD, HttpMethod::OPTIONS]);
                                options
                                    .into_iter()
                                    .map(|m| m.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            });
                            res
                        } else {
                            HttpResponse::not_found()
                        }
                    } else {
                        HttpResponse::not_found()
                    };
                    if conn.upgrade_ws {
                        break;
                    }
                    if let Err(_) = conn.write_binary(&res.as_bytes(cmode)).await {
                        break;
                    }
                }
            });
        }
    }
}
