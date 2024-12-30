use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use crate::{RequestHandlerFlag, WebsocketContext};
use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::server::TlsStream;
use tokio_rustls::{rustls, TlsAcceptor};

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

    pub async fn run_http1(&mut self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (mut stream, client_addr) = listener.accept().await?;
            let static_paths = self.static_paths.clone();
            _ = tokio::task::spawn(async move {
                loop {
                    let req = match HttpRequest::from_stream::<TcpStream>(&mut stream).await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();
                    let (res, upgrade_ws);
                    (res, upgrade_ws, stream) =
                        Self::process_request(req, client_addr, &static_paths, stream).await;
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

    pub async fn run_https1(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        //let cert_file = &mut BufReader::new(File::open("cert.pem")?);
        //let key_file = &mut BufReader::new(File::open("key.pem")?);
        let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_file)?;
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));

        loop {
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let static_paths = self.static_paths.clone();
            let acceptor = acceptor.clone();
            let mut stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            _ = tokio::task::spawn(async move {
                loop {
                    let req =
                        match HttpRequest::from_stream::<TlsStream<TcpStream>>(&mut stream).await {
                            Ok(req) => req,
                            Err(_) => break,
                        };
                    let cmode = req.get_header_accept_encoding();
                    let (res, upgrade_ws);
                    (res, upgrade_ws, stream) =
                        Self::process_request(req, client_addr, &static_paths, stream).await;
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

    async fn process_request<T: TcpStreamExt>(
        req: HttpRequest,
        client_addr: SocketAddr,
        static_paths: &Vec<(String, String)>,
        mut stream: T,
    ) -> (HttpResponse, bool, T) {
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
                stream: Box::new(stream),
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
                        let mut options: HashSet<_> = path_handlers.keys().map(|p| *p).collect();
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
                for (loca_path, url_path) in static_paths.iter() {
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
                    res = Some(HttpResponse::from_file(static_path.to_str().unwrap_or("")));
                }
            }
        }
        (res.unwrap_or(HttpResponse::not_found()), upgrade_ws, stream)
    }
}
