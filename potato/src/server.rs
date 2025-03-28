use crate::utils::enums::HttpConnection;
use crate::utils::number::HttpCodeExt;
use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use crate::{RequestHandlerFlag, WebsocketContext};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::{rustls, TlsAcceptor};

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
const PROF_ACTIVE: &'static [u8] = b"prof.active\0";

static HANDLERS: LazyLock<HashMap<&'static str, HashMap<HttpMethod, &'static RequestHandlerFlag>>> =
    LazyLock::new(|| {
        let mut handlers = HashMap::with_capacity(16);
        for flag in inventory::iter::<RequestHandlerFlag> {
            handlers
                .entry(flag.path)
                .or_insert_with(|| HashMap::with_capacity(16))
                .insert(flag.method, flag);
        }
        handlers
    });

#[derive(Clone)]
pub enum PipeContextItem {
    Dispatch,
    LocationRoute((String, String)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
    #[cfg(feature = "jemalloc")]
    Jemalloc(String),
}

pub struct PipeContext {
    items: Vec<PipeContextItem>,
}

impl PipeContext {
    pub fn new() -> Self {
        Self {
            items: vec![PipeContextItem::Dispatch],
        }
    }

    pub fn empty() -> Self {
        Self { items: vec![] }
    }

    pub fn clone_items(&self) -> Vec<PipeContextItem> {
        self.items.clone()
    }

    pub fn use_dispatch(&mut self) {
        self.items.push(PipeContextItem::Dispatch);
    }

    pub fn use_location_route(&mut self, url_path: impl Into<String>, loc_path: impl Into<String>) {
        let (url_path, loc_path) = (url_path.into(), loc_path.into());
        self.items
            .push(PipeContextItem::LocationRoute((url_path, loc_path)));
    }

    pub fn use_embedded_route(
        &mut self,
        url_path: impl Into<String>,
        assets: HashMap<String, Cow<'static, [u8]>>,
    ) {
        let mut ret = HashMap::with_capacity(16);
        let url_path = {
            let mut url_path: String = url_path.into();
            if url_path.ends_with('/') {
                url_path.pop();
            }
            url_path
        };
        for (key, value) in assets.into_iter() {
            ret.insert(format!("{url_path}/{key}"), value);
        }
        self.items.push(PipeContextItem::EmbeddedRoute(ret));
    }

    fn doc_index_json() -> String {
        let mut any_use_auth = false;
        let contact = {
            let re = regex::Regex::new(r"([[:word:]]+)\s*<([^>]+)>").unwrap();
            match re.captures(env!("CARGO_PKG_AUTHORS")) {
                Some(caps) => {
                    let name = caps.get(1).map_or("", |m| m.as_str());
                    let email = caps.get(2).map_or("", |m| m.as_str());
                    serde_json::json!({ "name": name, "email": email })
                }
                None => serde_json::json!({}),
            }
        };
        let paths = {
            let mut paths = std::collections::HashMap::with_capacity(16);
            for flag in inventory::iter::<RequestHandlerFlag> {
                if !flag.doc.show {
                    continue;
                }
                let mut response_http_codes = vec![200, 500];
                let mut root_cur_path = serde_json::json!({
                    "summary": flag.doc.summary,
                    "description": flag.doc.desp,
                });
                let arg_pairs = {
                    let mut arg_pairs = vec![];
                    if let Ok(args) = serde_json::from_str::<serde_json::Value>(flag.doc.args) {
                        if let Some(args) = args.as_array() {
                            for arg in args.iter() {
                                let arg_name = arg["name"].as_str().unwrap_or("");
                                let arg_type = {
                                    let arg_type = arg["type"].as_str().unwrap_or("");
                                    match arg_type.starts_with('i') || arg_type.starts_with('u') {
                                        true => "number",
                                        false if arg_type == "PostFile" => "file",
                                        false => "string",
                                    }
                                };
                                arg_pairs.push((arg_name.to_string(), arg_type.to_string()));
                            }
                        }
                    }
                    arg_pairs
                };
                if !arg_pairs.is_empty() {
                    if flag.method == HttpMethod::GET {
                        let mut parameters = vec![];
                        for (arg_name, arg_type) in arg_pairs.iter() {
                            parameters.push(serde_json::json!({
                                "name": arg_name,
                                "in": "query",
                                "description": "",
                                "required": true,
                                "schema": { "type": arg_type },
                            }));
                        }
                        root_cur_path["parameters"] = serde_json::Value::Array(parameters);
                    } else {
                        let mut properties = serde_json::json!({});
                        let mut required = vec![];
                        for (arg_name, arg_type) in arg_pairs.iter() {
                            properties[arg_name] = match arg_type == "file" {
                                true => {
                                    serde_json::json!({ "type": "string", "format": "binary" })
                                }
                                false => serde_json::json!({ "type": arg_type }),
                            };
                            required.push(arg_name);
                        }
                        // TODO add file
                        root_cur_path["requestBody"]["content"] = serde_json::json!({
                            "multipart/form-data": {
                                "schema": {
                                    "type": "object",
                                    "properties": properties,
                                    "required": required
                                }
                            }
                        });
                    }
                }
                if flag.doc.auth {
                    root_cur_path["security"] = serde_json::json!([{ "bearerAuth": [] }]);
                    response_http_codes = vec![200u16, 401, 500];
                    any_use_auth = true;
                }
                for http_code in response_http_codes.into_iter() {
                    let http_code_str = http_code.to_string();
                    root_cur_path["responses"][http_code_str]["description"] =
                        http_code.http_code_to_desp().into();
                }
                paths
                    .entry(flag.path)
                    .or_insert_with(|| HashMap::with_capacity(16))
                    .insert(flag.method.to_string().to_lowercase(), root_cur_path);
            }
            paths
        };
        let mut root = serde_json::json!({
            "openapi": "3.1.0",
            "info": {
                "title": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
                "description": env!("CARGO_PKG_DESCRIPTION"),
                "contact": contact,
            },
            "paths": paths,
        });
        if any_use_auth {
            root["components"]["securitySchemes"]["bearerAuth"] = serde_json::json!({
                "description": "Bearer token using a JWT",
                "type": "http",
                "scheme": "Bearer",
                "bearerFormat": "JWT",
            });
        }
        serde_json::to_string(&root).unwrap_or("{}".to_string())
    }

    pub fn use_doc(&mut self, url_path: impl Into<String>) {
        #[derive(rust_embed::Embed)]
        #[folder = "swagger_res"]
        struct DocAsset;

        let mut ret = HashMap::with_capacity(16);
        let url_path = {
            let mut url_path: String = url_path.into();
            if !url_path.ends_with('/') {
                url_path.push('/');
            }
            url_path
        };
        //
        ret.insert(format!("{url_path}index.json"), {
            let bytes = Self::doc_index_json().into_bytes();
            let static_bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            Cow::Borrowed(static_bytes)
        });
        //
        for name in DocAsset::iter().into_iter() {
            if name == "swagger-initializer.js" {
                ret.insert(
                    format!("{url_path}{name}"),
                    Cow::Borrowed(
                        r#"window.onload = function() {
  window.ui = SwaggerUIBundle({
    url: "index.json",
    dom_id: '#swagger-ui',
    deepLinking: true,
    presets: [ SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset ],
    plugins: [ SwaggerUIBundle.plugins.DownloadUrl ],
    layout: "StandaloneLayout"
  });
};"#
                        .as_bytes(),
                    ),
                );
            } else if let Some(file) = DocAsset::get(&name) {
                if name.ends_with("index.htm") || name.ends_with("index.html") {
                    if let Some(path) = Path::new(&format!("{url_path}{name}")).parent() {
                        if let Some(path) = path.to_str() {
                            let mut path = path.to_string();
                            if !path.ends_with('/') {
                                path.push('/');
                            }
                            ret.insert(path, file.data.clone());
                        }
                    }
                }
                ret.insert(format!("{url_path}{name}"), file.data);
            }
        }
        self.items.push(PipeContextItem::EmbeddedRoute(ret));
    }

    #[cfg(feature = "jemalloc")]
    pub fn use_jemalloc(&mut self, url_path: impl Into<String>) {
        self.items.push(PipeContextItem::Jemalloc(url_path.into()));
    }
}

pub struct PipeHandlerContext {
    pipe_ctx: Arc<PipeContext>,
    client_addr: SocketAddr,
    pub stream: Option<Box<dyn TcpStreamExt>>,
}

impl PipeHandlerContext {
    pub fn new(
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
        stream: Box<dyn TcpStreamExt>,
    ) -> Self {
        Self {
            pipe_ctx,
            client_addr,
            stream: Some(stream),
        }
    }

    pub async fn handle_request(&mut self, req: HttpRequest) -> HttpResponse {
        for item in self.pipe_ctx.items.iter() {
            match item {
                PipeContextItem::Dispatch => {
                    let handler_ref = match HANDLERS.get(req.url_path.to_str()) {
                        Some(handlers) => handlers.get(&req.method).map(|p| p.handler),
                        None => None,
                    };
                    if let Some(handler_ref) = handler_ref {
                        let mut wsctx = WebsocketContext {
                            stream: self.stream.take().unwrap(),
                            upgrade_ws: false,
                        };
                        let res = handler_ref(req, self.client_addr, &mut wsctx).await;
                        if !wsctx.is_upgraded_websocket() {
                            self.stream = Some(wsctx.stream);
                        }
                        return res;
                    } else {
                        if req.method == HttpMethod::HEAD {
                            return HttpResponse::empty();
                        } else if req.method == HttpMethod::OPTIONS {
                            let mut res2 = HttpResponse::html("");
                            let mut options: HashSet<_> = [HttpMethod::HEAD, HttpMethod::OPTIONS]
                                .into_iter()
                                .collect();
                            if let Some(handlers) = HANDLERS.get(req.url_path.to_str()) {
                                options.extend(handlers.keys().map(|p| *p));
                            }
                            res2.add_header("Allow", {
                                options
                                    .into_iter()
                                    .map(|m| m.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            });
                            return res2;
                        } else {
                            continue;
                        }
                    }
                }
                PipeContextItem::LocationRoute((url_path, loc_path)) => {
                    if !req.url_path.to_str().starts_with(url_path) {
                        continue;
                    }
                    let mut path = PathBuf::new();
                    path.push(loc_path);
                    path.push(&req.url_path.to_str()[url_path.len()..]);
                    if let Ok(path) = path.canonicalize() {
                        if !path.starts_with(loc_path) {
                            return HttpResponse::error("url path over directory");
                        }
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if meta.is_file() {
                                if let Some(path) = path.to_str() {
                                    return HttpResponse::from_file(path, false);
                                }
                            } else if meta.is_dir() {
                                let mut tmp_path = path.clone();
                                tmp_path.push("index.htm");
                                if let Ok(tmp) = std::fs::metadata(&tmp_path) {
                                    if tmp.is_file() {
                                        if let Some(path) = tmp_path.to_str() {
                                            return HttpResponse::from_file(path, false);
                                        }
                                    }
                                }
                                let mut tmp_path = path.clone();
                                tmp_path.push("index.html");
                                if let Some(path) = tmp_path.to_str() {
                                    return HttpResponse::from_file(path, false);
                                }
                            }
                        }
                    }
                    continue;
                }
                PipeContextItem::EmbeddedRoute(embedded_items) => {
                    if let Some(item) = embedded_items.get(req.url_path.to_str()) {
                        let ret = HttpResponse::from_mem_file(
                            req.url_path.to_str(),
                            item.to_vec(),
                            false,
                        );
                        return ret;
                    }
                    continue;
                }
                PipeContextItem::FinalRoute(res) => return res.clone(),
                #[cfg(feature = "jemalloc")]
                PipeContextItem::Jemalloc(path) => {
                    if path == req.url_path.to_str() {
                        let err_msg = match jemalloc_pprof::PROF_CTL.as_ref() {
                            Some(prof_ctl) => {
                                let mut prof_ctl = prof_ctl.lock().await;
                                match prof_ctl.activated() {
                                    true => match prof_ctl.dump_pprof() {
                                        Ok(data) => {
                                            return HttpResponse::from_mem_file(
                                                "heap.pb.gz",
                                                data,
                                                true,
                                            )
                                        }
                                        Err(err) => format!("{err}"),
                                    },
                                    false => format!("PROF_CTL is not activated"),
                                }
                            }
                            None => format!("PROF_CTL is empty"),
                        };
                        return HttpResponse::error(err_msg);
                    }
                }
            }
        }

        HttpResponse::not_found()
    }
}

pub struct HttpServer {
    addr: String,
    pipe_ctx: Arc<PipeContext>,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer {
            addr: addr.into(),
            pipe_ctx: Arc::new(PipeContext::new()),
        }
    }

    pub fn configure(&mut self, callback: impl Fn(&mut PipeContext)) {
        let mut ctx = PipeContext::empty();
        callback(&mut ctx);
        self.pipe_ctx = Arc::new(ctx);
    }

    #[cfg(feature = "jemalloc")]
    fn init_jemalloc(&self) -> anyhow::Result<()> {
        if let Ok(conf) = std::env::var("MALLOC_CONF") {
            if &conf == "prof:true" {
                use tikv_jemalloc_ctl::*;
                let name = PROF_ACTIVE.name();
                return Ok(name.write(true)?);
            }
        }
        Err(anyhow::anyhow!(
            "run `MALLOC_CONF=prof:true {}` for enable jemalloc",
            env!("CARGO_PKG_NAME")
        ))
    }

    pub async fn serve_http(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        self.init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let mut pipe_ctx =
                PipeHandlerContext::new(Arc::clone(&self.pipe_ctx), client_addr, Box::new(stream));
            _ = tokio::task::spawn(async move {
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (req, n) = {
                        let stream = pipe_ctx.stream.as_mut().unwrap();
                        match HttpRequest::from_stream(&mut buf, stream).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let res = pipe_ctx.handle_request(req).await;
                    if let Some(stream) = pipe_ctx.stream.as_mut() {
                        if stream.write_all(&res.as_bytes(cmode)).await.is_ok() {
                            buf.drain(..n);
                            continue;
                        }
                    }
                    if conn != HttpConnection::KeepAlive {
                        break;
                    }
                }
            });
        }
    }

    pub async fn serve_https(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        self.init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_file)?;
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));

        loop {
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let acceptor = acceptor.clone();
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let stream: Box<dyn TcpStreamExt> = Box::new(stream);
            let mut pipe_ctx =
                PipeHandlerContext::new(Arc::clone(&self.pipe_ctx), client_addr, stream);
            _ = tokio::task::spawn(async move {
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (req, n) = {
                        let stream = pipe_ctx.stream.as_mut().unwrap();
                        match HttpRequest::from_stream(&mut buf, stream).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let res = pipe_ctx.handle_request(req).await;
                    if let Some(stream) = pipe_ctx.stream.as_mut() {
                        if stream.write_all(&res.as_bytes(cmode)).await.is_ok() {
                            buf.drain(..n);
                            continue;
                        }
                    }
                    if conn != HttpConnection::KeepAlive {
                        break;
                    }
                }
            });
        }
    }
}
