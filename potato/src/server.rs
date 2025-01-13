use crate::utils::number::HttpCodeExt;
use crate::utils::string::StringUtil;
use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use crate::{RequestHandlerFlag, WebsocketContext};
use lazy_static::lazy_static;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
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

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: u64,
}

lazy_static! {
    static ref JWT_SECRET: RwLock<String> = RwLock::new(StringUtil::rand(32));
}

pub struct JwtAuth;

impl JwtAuth {
    pub async fn set_secret(secret: impl Into<String>) {
        let mut jwt_secret = JWT_SECRET.write().await;
        *jwt_secret = secret.into();
    }

    pub async fn issue(payload: String, expire: Duration) -> anyhow::Result<String> {
        let secret = {
            let jwt_secret = JWT_SECRET.read().await;
            jwt_secret.clone()
        };
        let claims = Claims {
            sub: payload,
            exp: (SystemTime::now() + expire)
                .duration_since(UNIX_EPOCH)?
                .as_micros() as u64,
        };
        Ok(jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )?)
    }

    pub async fn check(token: &str) -> anyhow::Result<String> {
        let secret = {
            let jwt_secret = JWT_SECRET.read().await;
            jwt_secret.clone()
        };
        let decoding_key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
        let validation = jsonwebtoken::Validation::default();
        let claims = jsonwebtoken::decode::<Claims>(token, &decoding_key, &validation)?.claims;
        let expired = SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(claims.exp);
        match SystemTime::now() <= expired {
            true => Ok(claims.sub),
            false => Err(anyhow::Error::msg("token expired")),
        }
    }
}

#[derive(Clone)]
pub enum PipeContextItem {
    Dispatch,
    LocationRoute((String, String)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
}

impl PipeContextItem {
    pub fn allow(&self, req: &HttpRequest) -> bool {
        match self {
            PipeContextItem::Dispatch => match HANDLERS.get(&req.url_path[..]) {
                Some(path_handlers) => match path_handlers.get(&req.method) {
                    Some(_) => true,
                    None => req.method == HttpMethod::HEAD || req.method == HttpMethod::OPTIONS,
                },
                None => false,
            },
            PipeContextItem::LocationRoute((url_path, loc_path)) => {
                if !req.url_path.starts_with(url_path) {
                    return false;
                }
                let mut path = PathBuf::new();
                path.push(loc_path);
                path.push(&req.url_path[url_path.len()..]);
                if let Ok(path) = path.canonicalize() {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if meta.is_file() {
                            return true;
                        } else if meta.is_dir() {
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.htm");
                            if let Ok(tmp) = std::fs::metadata(&tmp_path) {
                                return tmp.is_file();
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.html");
                            if let Ok(tmp) = std::fs::metadata(&tmp_path) {
                                return tmp.is_file();
                            }
                        }
                    }
                }
                return false;
            }
            PipeContextItem::EmbeddedRoute(embedded_items) => {
                embedded_items.contains_key(&req.url_path[..])
            }
            PipeContextItem::FinalRoute(_) => true,
        }
    }

    pub async fn process(
        &self,
        req: HttpRequest,
        client_addr: SocketAddr,
        stream: Box<dyn TcpStreamExt>,
        upgrade_ws: &mut bool,
    ) -> (HttpResponse, Box<dyn TcpStreamExt>) {
        match self {
            PipeContextItem::Dispatch => {
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
                    let res = handler_ref(req, client_addr, &mut wsctx).await;
                    *upgrade_ws = wsctx.is_upgraded_websocket();
                    (res, wsctx.stream)
                } else {
                    if let Some(path_handlers) = HANDLERS.get(&req.url_path[..]) {
                        if req.method == HttpMethod::HEAD {
                            return (HttpResponse::empty(), stream);
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
                            return (res2, stream);
                        }
                    }
                    (HttpResponse::error("internal error: unhandled"), stream)
                }
            }
            PipeContextItem::LocationRoute((url_path, loc_path)) => {
                if !req.url_path.starts_with(url_path) {
                    return (HttpResponse::error("internal error: unhandled"), stream);
                }
                let mut path = PathBuf::new();
                path.push(loc_path);
                path.push(&req.url_path[url_path.len()..]);
                if let Ok(path) = path.canonicalize() {
                    if !path.starts_with(loc_path) {
                        return (HttpResponse::error("url path over directory"), stream);
                    }
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if meta.is_file() {
                            if let Some(path) = path.to_str() {
                                return (HttpResponse::from_file(path), stream);
                            }
                        } else if meta.is_dir() {
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.htm");
                            if let Ok(tmp) = std::fs::metadata(&tmp_path) {
                                if tmp.is_file() {
                                    if let Some(path) = tmp_path.to_str() {
                                        return (HttpResponse::from_file(path), stream);
                                    }
                                }
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.html");
                            if let Some(path) = tmp_path.to_str() {
                                return (HttpResponse::from_file(path), stream);
                            }
                        }
                    }
                }
                (HttpResponse::error("internal error: unhandled"), stream)
            }
            PipeContextItem::EmbeddedRoute(embedded_items) => {
                if let Some(item) = embedded_items.get(&req.url_path[..]) {
                    let ret = HttpResponse::from_mem_file(&req.url_path[..], item.to_vec());
                    return (ret, stream);
                }
                (HttpResponse::error("internal error: unhandled"), stream)
            }
            PipeContextItem::FinalRoute(res) => (res.clone(), stream),
        }
    }
}

#[derive(Clone)]
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

    pub fn use_dispatch(&mut self) {
        self.items.push(PipeContextItem::Dispatch);
    }

    pub fn use_location_route(&mut self, url_path: impl Into<String>, loc_path: impl Into<String>) {
        let (url_path, loc_path) = (url_path.into(), loc_path.into());
        self.items
            .push(PipeContextItem::LocationRoute((url_path, loc_path)));
    }

    pub fn use_embedded_route<T: Embed>(&mut self, url_path: impl Into<String>) {
        let mut ret = HashMap::new();
        let url_path = {
            let mut url_path: String = url_path.into();
            if !url_path.ends_with('/') {
                url_path.push('/');
            }
            url_path
        };
        for name in T::iter().into_iter() {
            if let Some(file) = T::get(&name) {
                if name.ends_with("index.htm") || name.ends_with("index.html") {
                    if let Some(path) = Path::new(&format!("{url_path}{name}")).parent() {
                        if let Some(path) = path.to_str() {
                            ret.insert(path.to_string(), file.data.clone());
                        }
                    }
                }
                ret.insert(format!("{url_path}{name}"), file.data);
            }
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
            let mut paths = std::collections::HashMap::new();
            for flag in inventory::iter::<RequestHandlerFlag> {
                if !flag.doc.show {
                    continue;
                }
                let mut response_http_codes = vec![200, 500];
                let mut root_cur_path = serde_json::json!({
                    "summary": flag.doc.summary,
                    "description": flag.doc.desp,
                });
                let mut arg_pairs = {
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
                    .or_insert_with(std::collections::HashMap::new)
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

        let mut ret = HashMap::new();
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
}

pub struct HttpServer {
    addr: String,
    pipe_ctx: PipeContext,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer {
            addr: addr.into(),
            pipe_ctx: PipeContext::new(),
        }
    }

    pub fn configure(&mut self, callback: impl Fn(&mut PipeContext)) {
        let mut ctx = PipeContext::empty();
        callback(&mut ctx);
        self.pipe_ctx = ctx;
    }

    pub async fn serve_http(&mut self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        loop {
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let mut stream: Box<dyn TcpStreamExt> = Box::new(stream);
            let pipe_ctx = self.pipe_ctx.clone();
            _ = tokio::task::spawn(async move {
                loop {
                    let req = match HttpRequest::from_stream(&mut stream).await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();
                    let mut res = HttpResponse::not_found();
                    let mut upgrade_ws = false;
                    for pipe_item in pipe_ctx.items.iter() {
                        if pipe_item.allow(&req) {
                            (res, stream) = pipe_item
                                .process(req, client_addr, stream, &mut upgrade_ws)
                                .await;
                            break;
                        }
                    }
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

    pub async fn serve_https(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
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
            let pipe_ctx = self.pipe_ctx.clone();
            let acceptor = acceptor.clone();
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let mut stream: Box<dyn TcpStreamExt> = Box::new(stream);
            _ = tokio::task::spawn(async move {
                loop {
                    let req = match HttpRequest::from_stream(&mut stream).await {
                        Ok(req) => req,
                        Err(_) => break,
                    };
                    let cmode = req.get_header_accept_encoding();
                    let mut res = HttpResponse::not_found();
                    let mut upgrade_ws = false;
                    for pipe_item in pipe_ctx.items.iter() {
                        if pipe_item.allow(&req) {
                            (res, stream) = pipe_item
                                .process(req, client_addr, stream, &mut upgrade_ws)
                                .await;
                            break;
                        }
                    }
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
