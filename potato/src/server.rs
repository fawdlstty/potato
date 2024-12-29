use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpResponse};
use crate::{RequestHandlerFlag, WebsocketContext};
use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

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
    pub static_paths: Vec<(String, String)>,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer {
            addr: addr.into(),
            static_paths: vec![],
        }
    }

    pub fn set_static_path(&mut self, loc_path: impl Into<String>, url_path: impl Into<String>) {
        self.static_paths.push((loc_path.into(), url_path.into()));
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (mut stream, client_addr) = listener.accept().await?;
            let static_paths = self.static_paths.clone();
            _ = tokio::task::spawn(async move {
                loop {
                    let req = match stream.read_request().await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();

                    // call process pipes
                    let mut upgrade_ws = false;
                    let mut res = None;
                    let handler_ref = match HANDLERS.get(&req.url_path[..]) {
                        Some(path_handlers) => match path_handlers.get(&req.method) {
                            Some(handler) => Some(handler.handler),
                            None => None,
                        },
                        None => None,
                    };
                    if let Some(handler_ref) = handler_ref {
                        let mut wsctx = WebsocketContext {
                            stream,
                            upgrade_ws: false,
                        };
                        res = Some(handler_ref(req, client_addr, &mut wsctx).await);
                        (stream, upgrade_ws) = (wsctx.stream, wsctx.upgrade_ws);
                    } else {
                        if let Some(path_handlers) = HANDLERS.get(&req.url_path[..]) {
                            if req.method == HttpMethod::HEAD {
                                res = Some(HttpResponse::html(""));
                            } else if req.method == HttpMethod::OPTIONS {
                                let mut res2 = HttpResponse::html("");
                                res2.add_header("Allow", {
                                    let mut options: HashSet<_> =
                                        path_handlers.keys().map(|p| *p).collect();
                                    options.extend([HttpMethod::HEAD, HttpMethod::OPTIONS]);
                                    options
                                        .into_iter()
                                        .map(|m| m.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                });
                                res = Some(res2);
                            }
                        }
                        //
                        if res.is_none() {
                            let mut static_path = None;
                            for (loca_path, url_path) in &static_paths {
                                if req.url_path.starts_with(&url_path[..]) {
                                    let mut path = PathBuf::new();
                                    path.push(&loca_path);
                                    path.push(&req.url_path);
                                    macro_rules! assign_static_path {
                                        () => {
                                            if path.exists() {
                                                static_path = Some(path);
                                                break;
                                            }
                                        };
                                    }
                                    if req.url_path.ends_with('/') {
                                        path.push("index.htm");
                                        assign_static_path!();
                                        path.pop();
                                        path.push("index.html");
                                        assign_static_path!();
                                    } else {
                                        assign_static_path!();
                                    }
                                }
                            }
                            if let Some(static_path) = static_path {
                                res = Some(HttpResponse::from_file(
                                    static_path.to_str().unwrap_or(""),
                                ));
                            }
                        }
                    }
                    let res = res.unwrap_or(HttpResponse::not_found());
                    //
                    if upgrade_ws {
                        break;
                    }
                    if let Err(_) = stream.write_all(&res.as_bytes(cmode)).await {
                        break;
                    }
                }
            });
        }
    }
}
