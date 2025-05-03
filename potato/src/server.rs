use crate::utils::enums::HttpConnection;
use crate::utils::tcp_stream::HttpStream;
use crate::{HttpHandler, RequestHandlerFlag};
use crate::{HttpMethod, HttpRequest, HttpResponse};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use tokio::net::TcpListener;
use tokio::select;
use tokio::sync::{oneshot, Mutex};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::{rustls, TlsAcceptor};

type CustomNextHandler =
    Box<dyn Fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>>;

type CustomHandler = dyn Fn(
        &mut HttpRequest,
        CustomNextHandler,
    ) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + Sync + '_>>
    + Send
    + Sync;

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
    Handlers,
    LocationRoute((String, String)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
    Custom(Arc<CustomHandler>),
    #[cfg(feature = "jemalloc")]
    Jemalloc(String),
    #[cfg(feature = "webdav")]
    Webdav((String, dav_server::DavHandler)),
}

pub struct PipeContext {
    items: Vec<PipeContextItem>,
}

impl PipeContext {
    pub fn new() -> Self {
        Self {
            items: vec![PipeContextItem::Handlers],
        }
    }

    pub fn empty() -> Self {
        Self { items: vec![] }
    }

    pub fn clone_items(&self) -> Vec<PipeContextItem> {
        self.items.clone()
    }

    pub fn use_handlers(&mut self) {
        self.items.push(PipeContextItem::Handlers);
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

    pub fn use_custom<F>(&mut self, callback: F)
    where
        F: Fn(
                &mut HttpRequest,
                CustomNextHandler,
            ) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + Sync + '_>>
            + Send
            + Sync
            + 'static,
    {
        self.items.push(PipeContextItem::Custom(Arc::new(callback)));
    }

    #[cfg(feature = "jemalloc")]
    pub fn use_jemalloc(&mut self, url_path: impl Into<String>) {
        self.items.push(PipeContextItem::Jemalloc(url_path.into()));
    }

    #[cfg(feature = "openapi")]
    fn openapi_index_json() -> String {
        use crate::utils::number::HttpCodeExt;
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
        let (tags, paths) = {
            let mut tags = HashMap::with_capacity(16);
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
                let otag = {
                    let mut otag = None;
                    if let Some(idx) = flag.path.rfind('/') {
                        if idx > 0 {
                            otag = Some(flag.path[1..idx].replace('/', "_"));
                        }
                    }
                    otag
                };
                if let Some(tag) = otag {
                    tags.insert(tag.clone(), "");
                    root_cur_path["tags"] = serde_json::json!([tag]);
                };
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
            let mut tags: Vec<_> = tags.into_iter().collect::<Vec<_>>();
            tags.sort_by(|a, b| a.0.cmp(&b.0));
            let tags: Vec<_> = tags
                .into_iter()
                .map(|(k, v)| serde_json::json!({"name": k, "description": v}))
                .collect();
            (tags, paths)
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
            "tags": tags,
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

    #[cfg(feature = "openapi")]
    pub fn use_openapi(&mut self, url_path: impl Into<String>) {
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
            let bytes = Self::openapi_index_json().into_bytes();
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
                    if let Some(path) = std::path::Path::new(&format!("{url_path}{name}")).parent()
                    {
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

    #[cfg(feature = "webdav")]
    pub fn use_webdav_localfs(
        &mut self,
        url_path: impl Into<String>,
        local_path: impl Into<String>,
    ) {
        let dav_server = dav_server::DavHandler::builder()
            .filesystem(dav_server::localfs::LocalFs::new(
                local_path.into(),
                true,
                false,
                false,
            ))
            .locksystem(dav_server::fakels::FakeLs::new())
            .build_handler();
        self.items
            .push(PipeContextItem::Webdav((url_path.into(), dav_server)));
    }

    #[cfg(feature = "webdav")]
    pub fn use_webdav_memfs(&mut self, url_path: impl Into<String>) {
        let dav_server = dav_server::DavHandler::builder()
            .filesystem(dav_server::memfs::MemFs::new())
            .locksystem(dav_server::fakels::FakeLs::new())
            .build_handler();
        self.items
            .push(PipeContextItem::Webdav((url_path.into(), dav_server)));
    }

    pub async fn handle_request(
        self2: Arc<PipeContext>,
        req: &mut HttpRequest,
        skip: usize,
    ) -> HttpResponse {
        for (idx, item) in self2.items.iter().enumerate().skip(skip) {
            match item {
                PipeContextItem::Handlers => {
                    let handler_ref = match HANDLERS.get(req.url_path.to_str()) {
                        Some(handlers) => handlers.get(&req.method).map(|p| p.handler),
                        None => None,
                    };
                    if let Some(handler_ref) = handler_ref {
                        return handler_ref(req).await;
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
                PipeContextItem::Custom(handler) => {
                    let next = Box::new(|r| {
                        Box::pin(async move { Self::handle_request(self2, r, idx + 1).await })
                    }) as CustomNextHandler;
                    return handler.as_ref()(req, next).await;
                }
                #[cfg(feature = "jemalloc")]
                PipeContextItem::Jemalloc(path) => {
                    if path == req.url_path.to_str() {
                        return match crate::dump_jemalloc_profile().await {
                            Ok(data) => HttpResponse::from_mem_file("profile.pdf", data, false),
                            Err(err) => HttpResponse::error(format!("{err}")),
                        };
                    }
                }
                #[cfg(feature = "webdav")]
                PipeContextItem::Webdav((path, dav_server)) => {
                    use crate::utils::string::StringExt;
                    use futures_util::StreamExt;
                    if !req.url_path.to_str().starts_with(path) {
                        continue;
                    }
                    let new_req = {
                        let mut new_req = http::Request::new(match req.body.to_buf().len() {
                            0 => dav_server::body::Body::empty(),
                            _ => {
                                let bytes = bytes::Bytes::copy_from_slice(req.body.to_buf());
                                dav_server::body::Body::from(bytes)
                            }
                        });

                        if let Ok(method) =
                            http::Method::from_bytes(req.method.to_string().as_bytes())
                        {
                            *new_req.method_mut() = method;
                        }
                        *new_req.version_mut() = match req.version {
                            9 => http::Version::HTTP_09,
                            10 => http::Version::HTTP_10,
                            11 => http::Version::HTTP_11,
                            20 => http::Version::HTTP_2,
                            30 => http::Version::HTTP_3,
                            _ => http::Version::HTTP_11,
                        };
                        if let Ok(uri) = req.get_uri(false) {
                            *new_req.uri_mut() = uri;
                        }
                        for (k, v) in req.headers.iter() {
                            if let Ok(v) = http::HeaderValue::from_str(v.to_str()) {
                                let k: &'static str = unsafe { std::mem::transmute(k.to_str()) };
                                new_req.headers_mut().append(k, v);
                            }
                        }
                        new_req
                    };
                    let res = {
                        let mut new_res = dav_server.handle(new_req).await;
                        let mut res = HttpResponse::empty();
                        for (k, v) in new_res.headers().iter() {
                            res.add_header(k.as_str().http_std_case(), v.to_str().unwrap_or(""));
                        }
                        res.http_code = new_res.status().as_u16();
                        res.version = format!("{:?}", new_res.version());
                        let body = new_res.body_mut();
                        while let Some(Ok(part)) = body.next().await {
                            res.body.extend(part.iter());
                        }
                        res
                    };
                    return res;
                }
            }
        }

        HttpResponse::not_found()
    }
}

pub struct HttpServer {
    addr: String,
    pipe_ctx: Arc<PipeContext>,
    shutdown_signal: Option<oneshot::Receiver<()>>,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer {
            addr: addr.into(),
            pipe_ctx: Arc::new(PipeContext::new()),
            shutdown_signal: None,
        }
    }

    pub fn configure(&mut self, callback: impl Fn(&mut PipeContext)) {
        let mut ctx = PipeContext::empty();
        callback(&mut ctx);
        self.pipe_ctx = Arc::new(ctx);
    }

    pub fn shutdown_signal(&mut self) -> oneshot::Sender<()> {
        let (tx, rx) = oneshot::channel();
        if self.shutdown_signal.is_some() {
            panic!("shutdown signal already set");
        }
        self.shutdown_signal = Some(rx);
        tx
    }

    pub async fn serve_http(&mut self) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = self.serve_http_impl() => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => self.serve_http_impl().await,
        }
    }

    pub async fn serve_https(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = self.serve_https_impl(cert_file, key_file) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => self.serve_https_impl(cert_file, key_file).await,
        }
    }

    async fn serve_http_impl(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        loop {
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let stream = Arc::new(Mutex::new(HttpStream::from_tcp(stream)));
            _ = tokio::task::spawn(async move {
                let client_addr = Arc::new(client_addr);
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (mut req, n) = {
                        match HttpRequest::from_stream(&mut buf, Arc::clone(&stream)).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    req.add_ext(Arc::clone(&client_addr));
                    req.add_ext(Arc::clone(&stream));
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let res =
                        PipeContext::handle_request(Arc::clone(&pipe_ctx2), &mut req, 0).await;
                    {
                        let mut stream = stream.lock().await;
                        match stream.write_all(&res.as_bytes(cmode)).await {
                            Ok(()) => _ = buf.drain(..n),
                            Err(_) => break,
                        }
                    }
                    if req.get_ext::<Mutex<HttpStream>>().is_none() {
                        break;
                    }
                    if conn != HttpConnection::KeepAlive {
                        break;
                    }
                }
            });
        }
    }

    async fn serve_https_impl(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_file)?;
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        loop {
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            // accept connection
            let (stream, client_addr) = listener.accept().await?;
            let acceptor = acceptor.clone();
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let stream = Arc::new(Mutex::new(HttpStream::from_tls(stream)));
            _ = tokio::task::spawn(async move {
                let client_addr = Arc::new(client_addr);
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (mut req, n) = {
                        match HttpRequest::from_stream(&mut buf, Arc::clone(&stream)).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    req.add_ext(Arc::clone(&client_addr));
                    req.add_ext(Arc::clone(&stream));
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let res =
                        PipeContext::handle_request(Arc::clone(&pipe_ctx2), &mut req, 0).await;
                    {
                        let mut stream = stream.lock().await;
                        match stream.write_all(&res.as_bytes(cmode)).await {
                            Ok(()) => _ = buf.drain(..n),
                            Err(_) => break,
                        }
                    }
                    if req.get_ext::<Mutex<HttpStream>>().is_none() {
                        break;
                    }
                    if conn != HttpConnection::KeepAlive {
                        break;
                    }
                }
            });
        }
    }
}
