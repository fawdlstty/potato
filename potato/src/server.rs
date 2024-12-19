use crate::{utils::tcp_stream::TcpStreamExt, RequestHandlerFlag};
use crate::{HttpMethod, HttpResponse, RequestContext};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use tokio::{io::AsyncWriteExt, net::TcpListener};

pub struct HttpServer {
    pub addr: String,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer { addr: addr.into() }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let handlers = {
            let mut handlers = HashMap::new();
            for flag in inventory::iter::<RequestHandlerFlag> {
                handlers
                    .entry(flag.path)
                    .or_insert_with(HashMap::new)
                    .insert(flag.method, flag.handler);
            }
            handlers
        };

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (mut socket, addr) = listener.accept().await?;

            let handlers2 = handlers.clone();
            _ = tokio::task::spawn(async move {
                loop {
                    let req = match socket.read_request().await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();
                    let ctx = RequestContext { addr, req };

                    // call process pipes
                    let res = if let Some(path_handlers) = handlers2.get(ctx.req.uri.path()) {
                        if let Some(handler) = path_handlers.get(&ctx.req.method) {
                            handler(ctx).await
                        } else if ctx.req.method == HttpMethod::HEAD {
                            HttpResponse::html("")
                        } else if ctx.req.method == HttpMethod::OPTIONS {
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
                    if let Err(_) = socket.write_all(&res.as_bytes(cmode)).await {
                        break;
                    }
                }
            });
        }
    }
}
