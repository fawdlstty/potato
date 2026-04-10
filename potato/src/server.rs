use crate::utils::enums::HttpConnection;
use crate::utils::refstr::HeaderItem;
#[cfg(any(feature = "http2", feature = "http3"))]
use crate::utils::refstr::HeaderOrHipStr;
use crate::utils::tcp_stream::HttpStream;
use crate::CompressMode;
use crate::{
    HttpHandler, HttpMethod, HttpRequest, HttpRequestTargetForm, HttpResponse, PreflightResult,
};
use crate::{RequestHandlerFlag, TransferSession};
#[cfg(feature = "http3")]
use bytes::Buf;
#[cfg(feature = "http2")]
use h2::server as h2_server;
#[cfg(feature = "http3")]
use h3::server as h3_server;
#[cfg(feature = "http3")]
use quinn::crypto::rustls::QuicServerConfig;
#[cfg(feature = "http3")]
use quinn::{self};
use std::any::TypeId;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;
use tokio::net::TcpListener;
use tokio::select;
use tokio::sync::{oneshot, Mutex};
#[cfg(any(feature = "tls", feature = "http3"))]
use tokio_rustls::rustls;
#[cfg(any(feature = "tls", feature = "http3"))]
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
#[cfg(feature = "tls")]
use tokio_rustls::TlsAcceptor;

type AsyncCustomHandler = dyn Fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = Option<HttpResponse>> + Send + '_>>
    + Send
    + Sync;

type SyncCustomHandler = dyn Fn(&mut HttpRequest) -> Option<HttpResponse> + Send + Sync;

#[derive(Clone)]
pub enum CustomHandler {
    Sync(Arc<SyncCustomHandler>),
    Async(Arc<AsyncCustomHandler>),
}

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

static HANDLERS_FLAT: LazyLock<HashMap<(&'static str, HttpMethod), &'static RequestHandlerFlag>> =
    LazyLock::new(|| {
        let mut handlers = HashMap::with_capacity(64);
        for flag in inventory::iter::<RequestHandlerFlag> {
            handlers.insert((flag.path, flag.method), flag);
        }
        handlers
    });

#[derive(Clone)]
pub enum PipeContextItem {
    Handlers(bool),
    LocationRoute((String, String, bool)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
    Custom(CustomHandler),
    ReverseProxy(String, String, bool),
    #[cfg(feature = "jemalloc")]
    Jemalloc(String),
    #[cfg(feature = "webdav")]
    Webdav((String, dav_server::DavHandler)),
}

pub struct PipeContext {
    items: Vec<PipeContextItem>,
}

impl PipeContext {
    fn sanitize_location_route_path(loc_path: &str, request_suffix: &str) -> Option<PathBuf> {
        let mut path = PathBuf::from(loc_path);
        for component in Path::new(request_suffix).components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => path.push(part),
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(path)
    }

    fn path_stays_inside_root(path: &Path, root: &Path) -> bool {
        std::fs::canonicalize(path)
            .map(|resolved| resolved.starts_with(root))
            .unwrap_or(false)
    }

    fn static_file_etag(meta: &Metadata) -> Option<String> {
        if let Ok(modified) = meta.modified() {
            if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                let modified_secs = duration.as_secs();
                let file_size = meta.len();
                return Some(format!("\"{:x}-{:x}\"", modified_secs, file_size));
            }
        }
        None
    }

    fn add_static_validators(res: &mut HttpResponse, meta: &Metadata, etag: Option<&str>) {
        if let Ok(modified) = meta.modified() {
            if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                let modified_time = chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH + duration);
                res.add_header(
                    "Last-Modified".into(),
                    modified_time
                        .format("%a, %d %b %Y %H:%M:%S GMT")
                        .to_string()
                        .into(),
                );
            }
        }
        if let Some(etag) = etag {
            res.add_header("ETag".into(), etag.to_string().into());
        }
    }

    fn add_embedded_validators(res: &mut HttpResponse, meta: Option<&Metadata>, etag: &str) {
        if let Some(meta) = meta {
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_time =
                        chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH + duration);
                    res.add_header(
                        "Last-Modified".into(),
                        modified_time
                            .format("%a, %d %b %Y %H:%M:%S GMT")
                            .to_string()
                            .into(),
                    );
                }
            }
        }
        res.add_header("ETag".into(), etag.to_string().into());
    }

    fn should_apply_range_for_embedded(
        req: &HttpRequest,
        meta: Option<&Metadata>,
        etag: &str,
    ) -> bool {
        if req.get_header_key(HeaderItem::Range).is_none() {
            return false;
        }
        let Some(if_range) = req.get_header_key(HeaderItem::If_Range) else {
            return true;
        };
        let if_range = if_range.trim();
        if if_range.is_empty() {
            return false;
        }
        if if_range.starts_with('"') {
            return etag == if_range;
        }
        let Some(meta) = meta else {
            return false;
        };
        let Some(modified_secs) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
        else {
            return false;
        };
        match crate::parse_http_date(if_range) {
            Ok(since_timestamp) => modified_secs <= since_timestamp,
            Err(_) => false,
        }
    }

    fn should_apply_range(req: &HttpRequest, meta: &Metadata, etag: Option<&str>) -> bool {
        if req.get_header_key(HeaderItem::Range).is_none() {
            return false;
        }
        let Some(if_range) = req.get_header_key(HeaderItem::If_Range) else {
            return true;
        };
        let if_range = if_range.trim();
        if if_range.is_empty() {
            return false;
        }
        if if_range.starts_with('"') {
            return etag.is_some_and(|tag| tag == if_range);
        }
        let Some(modified_secs) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
        else {
            return false;
        };
        match crate::parse_http_date(if_range) {
            Ok(since_timestamp) => modified_secs <= since_timestamp,
            Err(_) => false,
        }
    }

    fn parse_single_byte_range(range_header: &str, file_size: u64) -> Option<Option<(u64, u64)>> {
        let range_header = range_header.trim();
        if file_size == 0 {
            return Some(None);
        }
        let Some(spec) = range_header.strip_prefix("bytes=") else {
            return None;
        };
        if spec.contains(',') {
            return None;
        }
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        if let Some(suffix) = spec.strip_prefix('-') {
            let Ok(suffix_len) = suffix.parse::<u64>() else {
                return None;
            };
            if suffix_len == 0 {
                return Some(None);
            }
            let start = if suffix_len >= file_size {
                0
            } else {
                file_size - suffix_len
            };
            return Some(Some((start, file_size - 1)));
        }

        let Some((start_str, end_str)) = spec.split_once('-') else {
            return None;
        };
        let Ok(start) = start_str.trim().parse::<u64>() else {
            return None;
        };
        if start >= file_size {
            return Some(None);
        }

        if end_str.trim().is_empty() {
            return Some(Some((start, file_size - 1)));
        }

        let Ok(mut end) = end_str.trim().parse::<u64>() else {
            return None;
        };
        if end >= file_size {
            end = file_size - 1;
        }
        if start > end {
            return Some(None);
        }
        Some(Some((start, end)))
    }

    fn read_file_range(path: &str, start: u64, end: u64) -> anyhow::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(start))?;
        let read_len_u64 = end - start + 1;
        let read_len = usize::try_from(read_len_u64)?;
        let mut buffer = vec![0u8; read_len];
        file.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn from_static_file(req: &HttpRequest, path: &str, meta: &Metadata) -> HttpResponse {
        let etag = Self::static_file_etag(meta);
        match req.check_precondition_headers(Some(meta), etag.as_deref()) {
            PreflightResult::NotModified => {
                let mut res = HttpResponse::empty();
                res.http_code = 304;
                Self::add_static_validators(&mut res, meta, etag.as_deref());
                return res;
            }
            PreflightResult::PreconditionFailed => {
                let mut res = HttpResponse::error("Precondition Failed");
                res.http_code = 412;
                Self::add_static_validators(&mut res, meta, etag.as_deref());
                return res;
            }
            PreflightResult::Proceed => {}
        }

        if Self::should_apply_range(req, meta, etag.as_deref()) {
            if let Some(parsed_range) = req
                .get_header_key(HeaderItem::Range)
                .and_then(|range| Self::parse_single_byte_range(range, meta.len()))
            {
                match parsed_range {
                    Some((start, end)) => {
                        let data = match Self::read_file_range(path, start, end) {
                            Ok(data) => data,
                            Err(err) => return HttpResponse::error(format!("{err}")),
                        };
                        let mut res = HttpResponse::from_mem_file(path, data, false, None);
                        res.http_code = 206;
                        res.add_header(
                            "Content-Range".into(),
                            format!("bytes {start}-{end}/{}", meta.len()).into(),
                        );
                        res.add_header("Accept-Ranges".into(), "bytes".into());
                        Self::add_static_validators(&mut res, meta, etag.as_deref());
                        return res;
                    }
                    None => {
                        let mut res = HttpResponse::empty();
                        res.http_code = 416;
                        res.add_header(
                            "Content-Range".into(),
                            format!("bytes */{}", meta.len()).into(),
                        );
                        res.add_header("Accept-Ranges".into(), "bytes".into());
                        Self::add_static_validators(&mut res, meta, etag.as_deref());
                        return res;
                    }
                }
            }
        }

        let mut res = HttpResponse::from_file(path, false, Some(meta.clone()));
        res.add_header("Accept-Ranges".into(), "bytes".into());
        res
    }

    pub fn new() -> Self {
        Self {
            items: vec![PipeContextItem::Handlers(false)],
        }
    }

    pub fn empty() -> Self {
        Self { items: vec![] }
    }

    pub fn clone_items(&self) -> Vec<PipeContextItem> {
        self.items.clone()
    }

    pub fn use_handlers(&mut self, allow_cors: bool) {
        self.items.push(PipeContextItem::Handlers(allow_cors));
    }

    pub fn use_location_route(
        &mut self,
        url_path: impl Into<String>,
        loc_path: impl Into<String>,
        allow_symlink_escape: bool,
    ) {
        let (url_path, loc_path) = (url_path.into(), loc_path.into());
        self.items
            .push(PipeContextItem::LocationRoute((
                url_path,
                loc_path,
                allow_symlink_escape,
            )));
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

    pub fn use_custom<F, Fut>(&mut self, callback: F)
    where
        F: Fn(&mut HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<HttpResponse>> + Send + 'static,
    {
        self.items
            .push(PipeContextItem::Custom(CustomHandler::Async(Arc::new(
                move |req| {
                    let fut = callback(req);
                    Box::pin(async move { fut.await })
                },
            ))));
    }

    pub fn use_custom_sync<F>(&mut self, callback: F)
    where
        F: Fn(&mut HttpRequest) -> Option<HttpResponse> + Send + Sync + 'static,
    {
        self.items
            .push(PipeContextItem::Custom(CustomHandler::Sync(Arc::new(
                callback,
            ))));
    }

    pub fn use_custom_async<F>(&mut self, callback: F)
    where
        F: for<'a> Fn(
                &'a mut HttpRequest,
            )
                -> Pin<Box<dyn Future<Output = Option<HttpResponse>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        self.items
            .push(PipeContextItem::Custom(CustomHandler::Async(Arc::new(
                callback,
            ))));
    }

    pub fn use_reverse_proxy(
        &mut self,
        url_path: impl Into<String>,
        proxy_url: impl Into<String>,
        modify_content: bool,
    ) {
        self.items.push(PipeContextItem::ReverseProxy(
            url_path.into(),
            proxy_url.into(),
            modify_content,
        ));
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
        self2: &PipeContext,
        req: &mut HttpRequest,
        skip: usize,
    ) -> HttpResponse {
        if req.method == HttpMethod::CONNECT {
            let mut res = HttpResponse::text("CONNECT method is not implemented");
            res.http_code = 501;
            return res;
        }

        for (_idx, item) in self2.items.iter().enumerate().skip(skip) {
            match item {
                PipeContextItem::Handlers(allow_cors) => {
                    let handler_ref = HANDLERS_FLAT
                        .get(&(&req.url_path[..], req.method))
                        .map(|p| p.handler);
                    if let Some(handler_ref) = handler_ref {
                        return match handler_ref {
                            HttpHandler::Async(handler) => handler(req).await,
                            HttpHandler::Sync(handler) => handler(req),
                        };
                    } else {
                        if req.method == HttpMethod::HEAD {
                            if let Some(get_handler_ref) = HANDLERS_FLAT
                                .get(&(&req.url_path[..], HttpMethod::GET))
                                .map(|p| p.handler)
                            {
                                req.method = HttpMethod::GET;
                                let mut res = match get_handler_ref {
                                    HttpHandler::Async(handler) => handler(req).await,
                                    HttpHandler::Sync(handler) => handler(req),
                                };
                                req.method = HttpMethod::HEAD;
                                res.body = crate::HttpResponseBody::Data(vec![]);
                                return res;
                            }

                            // If no GET fallback exists, continue the pipeline so
                            // other route handlers (static/custom/proxy) can answer HEAD.
                            continue;
                        } else if req.method == HttpMethod::OPTIONS {
                            let mut res2 = HttpResponse::html("");
                            let methods_str: Cow<'static, str> = {
                                let mut options: HashSet<_> =
                                    [HttpMethod::OPTIONS].into_iter().collect();
                                if req.target_form == HttpRequestTargetForm::Asterisk {
                                    options.extend(HANDLERS_FLAT.keys().map(|(_, method)| *method));
                                    if HANDLERS_FLAT
                                        .keys()
                                        .any(|(_, method)| *method == HttpMethod::GET)
                                    {
                                        options.insert(HttpMethod::HEAD);
                                    }
                                } else if let Some(handlers) = HANDLERS.get(&req.url_path[..]) {
                                    options.extend(handlers.keys().map(|p| *p));
                                    if handlers.contains_key(&HttpMethod::GET) {
                                        options.insert(HttpMethod::HEAD);
                                    }
                                }
                                options
                                    .into_iter()
                                    .map(|m| m.to_string())
                                    .collect::<Vec<_>>()
                                    .join(",")
                                    .into()
                            };

                            res2.add_header("Allow".into(), methods_str.clone());
                            if *allow_cors {
                                res2.add_header("Access-Control-Allow-Origin".into(), "*".into());
                                res2.add_header("Access-Control-Allow-Methods".into(), methods_str);
                                res2.add_header("Access-Control-Allow-Headers".into(), "*".into());
                            }
                            return res2;
                        } else {
                            continue;
                        }
                    }
                }
                PipeContextItem::LocationRoute((url_path, loc_path, allow_symlink_escape)) => {
                    if !req.url_path.starts_with(url_path) {
                        continue;
                    }
                    let canonical_root = if *allow_symlink_escape {
                        None
                    } else {
                        std::fs::canonicalize(loc_path).ok()
                    };
                    let req_suffix = req.url_path[url_path.len()..].trim_start_matches('/');
                    let path = match Self::sanitize_location_route_path(loc_path, req_suffix) {
                        Some(path) => path,
                        None => return HttpResponse::error("url path over directory"),
                    };
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if meta.is_file() {
                            if let Some(root) = canonical_root.as_ref() {
                                if !Self::path_stays_inside_root(&path, root) {
                                    return HttpResponse::error("url path over directory");
                                }
                            }
                            if let Some(path) = path.to_str() {
                                return Self::from_static_file(req, path, &meta);
                            }
                        } else if meta.is_dir() {
                            if let Some(root) = canonical_root.as_ref() {
                                if !Self::path_stays_inside_root(&path, root) {
                                    return HttpResponse::error("url path over directory");
                                }
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.htm");
                            if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                if tmp_meta.is_file() {
                                    if let Some(root) = canonical_root.as_ref() {
                                        if !Self::path_stays_inside_root(&tmp_path, root) {
                                            return HttpResponse::error("url path over directory");
                                        }
                                    }
                                    if let Some(path) = tmp_path.to_str() {
                                        return Self::from_static_file(req, path, &tmp_meta);
                                    }
                                }
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.html");
                            if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                if tmp_meta.is_file() {
                                    if let Some(root) = canonical_root.as_ref() {
                                        if !Self::path_stays_inside_root(&tmp_path, root) {
                                            return HttpResponse::error("url path over directory");
                                        }
                                    }
                                    if let Some(path) = tmp_path.to_str() {
                                        return Self::from_static_file(req, path, &tmp_meta);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                PipeContextItem::EmbeddedRoute(embedded_items) => {
                    if let Some(item) = embedded_items.get(&req.url_path[..]) {
                        let meta = std::env::current_exe()
                            .ok()
                            .map(|p| std::fs::metadata(&p).ok())
                            .flatten();

                        // Generate ETag (based on content hash and file size)
                        let etag = {
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut hasher = DefaultHasher::new();
                            item.hash(&mut hasher);
                            let content_hash = hasher.finish();
                            format!("\"{:x}-{:x}\"", content_hash, item.len())
                        };

                        // Execute preflight check
                        match req.check_precondition_headers(meta.as_ref(), Some(etag.as_str())) {
                            PreflightResult::NotModified => {
                                let mut res = HttpResponse::empty();
                                res.http_code = 304;
                                Self::add_embedded_validators(
                                    &mut res,
                                    meta.as_ref(),
                                    etag.as_str(),
                                );
                                return res;
                            }
                            PreflightResult::PreconditionFailed => {
                                let mut res = HttpResponse::error("Precondition Failed");
                                res.http_code = 412;
                                Self::add_embedded_validators(
                                    &mut res,
                                    meta.as_ref(),
                                    etag.as_str(),
                                );
                                return res;
                            }
                            PreflightResult::Proceed => {
                                // Continue processing
                            }
                        }

                        if Self::should_apply_range_for_embedded(req, meta.as_ref(), etag.as_str())
                        {
                            if let Some(parsed_range) =
                                req.get_header_key(HeaderItem::Range).and_then(|range| {
                                    Self::parse_single_byte_range(range, item.len() as u64)
                                })
                            {
                                match parsed_range {
                                    Some((start, end)) => {
                                        let data = item[start as usize..=end as usize].to_vec();
                                        let mut res = HttpResponse::from_mem_file(
                                            &req.url_path,
                                            data,
                                            false,
                                            None,
                                        );
                                        res.http_code = 206;
                                        res.add_header(
                                            "Content-Range".into(),
                                            format!("bytes {start}-{end}/{}", item.len()).into(),
                                        );
                                        res.add_header("Accept-Ranges".into(), "bytes".into());
                                        Self::add_embedded_validators(
                                            &mut res,
                                            meta.as_ref(),
                                            etag.as_str(),
                                        );
                                        return res;
                                    }
                                    None => {
                                        let mut res = HttpResponse::empty();
                                        res.http_code = 416;
                                        res.add_header(
                                            "Content-Range".into(),
                                            format!("bytes */{}", item.len()).into(),
                                        );
                                        res.add_header("Accept-Ranges".into(), "bytes".into());
                                        Self::add_embedded_validators(
                                            &mut res,
                                            meta.as_ref(),
                                            etag.as_str(),
                                        );
                                        return res;
                                    }
                                }
                            }
                        }

                        let mut ret =
                            HttpResponse::from_mem_file(&req.url_path, item.to_vec(), false, None);
                        ret.add_header("Accept-Ranges".into(), "bytes".into());
                        Self::add_embedded_validators(&mut ret, meta.as_ref(), etag.as_str());
                        return ret;
                    }
                    continue;
                }
                PipeContextItem::FinalRoute(res) => return res.clone(),
                PipeContextItem::Custom(handler) => match handler {
                    CustomHandler::Sync(handler) => match handler.as_ref()(req) {
                        Some(res) => return res,
                        None => continue,
                    },
                    CustomHandler::Async(handler) => match handler.as_ref()(req).await {
                        Some(res) => return res,
                        None => continue,
                    },
                },
                PipeContextItem::ReverseProxy(path, proxy_url, modify_content) => {
                    if !req.url_path.starts_with(path) {
                        continue;
                    }

                    let mut transfer_session =
                        TransferSession::from_reverse_proxy(path.clone(), proxy_url.clone());

                    match transfer_session.transfer(req, *modify_content).await {
                        Ok(response) => return response,
                        Err(err) => return HttpResponse::error(format!("{}", err)),
                    }
                }

                #[cfg(feature = "jemalloc")]
                PipeContextItem::Jemalloc(path) => {
                    if path == &req.url_path[..] {
                        return match crate::dump_jemalloc_profile().await {
                            Ok(data) => {
                                // Generate ETag (based on content hash and file size)
                                let etag = {
                                    use std::collections::hash_map::DefaultHasher;
                                    use std::hash::{Hash, Hasher};
                                    let mut hasher = DefaultHasher::new();
                                    data.hash(&mut hasher);
                                    let content_hash = hasher.finish();
                                    Some(format!("\"{:x}-{:x}\"", content_hash, data.len()))
                                };

                                // Execute preflight check
                                match req.check_precondition_headers(None, etag.as_deref()) {
                                    PreflightResult::NotModified => {
                                        let mut res = HttpResponse::empty();
                                        res.http_code = 304;
                                        res
                                    }
                                    PreflightResult::PreconditionFailed => {
                                        let mut res = HttpResponse::error("Precondition Failed");
                                        res.http_code = 412;
                                        res
                                    }
                                    PreflightResult::Proceed => HttpResponse::from_mem_file(
                                        "profile.pdf",
                                        data,
                                        false,
                                        None,
                                    ),
                                }
                            }
                            Err(err) => HttpResponse::error(format!("{err}")),
                        };
                    }
                }
                #[cfg(feature = "webdav")]
                PipeContextItem::Webdav((path, dav_server)) => {
                    use crate::utils::string::StringExt;
                    use futures_util::StreamExt;
                    if !req.url_path.starts_with(path) {
                        continue;
                    }
                    let new_req = {
                        let mut new_req = http::Request::new(match req.body.len() {
                            0 => dav_server::body::Body::empty(),
                            _ => {
                                let bytes = bytes::Bytes::copy_from_slice(&req.body[..]);
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
                        // Modify URI to remove the specified path prefix and preserve original scheme and authority
                        use std::str::FromStr;
                        let adjusted_path = if req.url_path.starts_with(path) {
                            // Remove the path prefix from the original path
                            &req.url_path[path.len()..]
                        } else {
                            &req.url_path[..]
                        };

                        // Ensure the path starts with / for valid URI
                        let final_path = if adjusted_path.is_empty() {
                            "/"
                        } else {
                            adjusted_path
                        };

                        // Try to get original URI and preserve scheme/authority if available
                        match req.get_uri(false) {
                            Ok(original_uri) => {
                                *new_req.uri_mut() = http::uri::Builder::new()
                                    .scheme(
                                        original_uri.scheme().map(|s| s.as_str()).unwrap_or("http"),
                                    )
                                    .authority(
                                        original_uri
                                            .authority()
                                            .map(|a| a.as_str())
                                            .unwrap_or("127.0.0.1"),
                                    )
                                    .path_and_query(final_path)
                                    .build()
                                    .unwrap_or_else(|_| http::Uri::from_str(final_path).unwrap());
                            }
                            Err(_) => {
                                // If original URI is not available, construct with path only
                                *new_req.uri_mut() = http::uri::Builder::new()
                                    .path_and_query(final_path)
                                    .build()
                                    .unwrap_or_else(|_| http::Uri::from_str(final_path).unwrap());
                            }
                        };
                        for (k, v) in req.headers.iter() {
                            if let Ok(v) = http::HeaderValue::from_str(&v[..]) {
                                let k: &'static str = unsafe { std::mem::transmute(k.to_str()) };
                                new_req.headers_mut().append(k, v);
                            }
                        }
                        new_req
                    };
                    let res = {
                        let mut new_res = dav_server.handle(new_req).await;
                        let mut res = HttpResponse::empty();
                        let headers: Vec<(String, String)> = new_res
                            .headers()
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.as_str().http_std_case(),
                                    v.to_str().unwrap_or("").to_string(),
                                )
                            })
                            .collect();
                        for (k, v) in headers {
                            res.add_header(k.into(), v.into());
                        }
                        res.http_code = new_res.status().as_u16();
                        res.version = format!("{:?}", new_res.version());
                        let body = new_res.body_mut();
                        let mut body_data = Vec::new();
                        while let Some(Ok(part)) = body.next().await {
                            body_data.extend(part.iter());
                        }
                        res.body = crate::HttpResponseBody::Data(body_data);
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

    #[cfg(feature = "tls")]
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

    #[cfg(feature = "http2")]
    pub async fn serve_http2(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = self.serve_http2_impl(cert_file, key_file) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => self.serve_http2_impl(cert_file, key_file).await,
        }
    }

    #[cfg(feature = "http3")]
    pub async fn serve_http3(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = self.serve_http3_impl(cert_file, key_file) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => self.serve_http3_impl(cert_file, key_file).await,
        }
    }

    fn spawn_http1_connection(
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
        stream: HttpStream,
    ) {
        let mut stream = Arc::new(Mutex::new(stream));
        _ = tokio::task::spawn(async move {
            let mut buf: Vec<u8> = Vec::with_capacity(4096);
            loop {
                let (mut req, n) = {
                    match HttpRequest::from_stream(&mut buf, Arc::clone(&stream)).await {
                        Ok((req, n)) => (req, n),
                        Err(err) => {
                            if let Some(mut res) = HttpRequest::parse_error_response(&err) {
                                let mut stream_guard = stream.lock().await;
                                let _ = res
                                    .write_to_stream(&mut stream_guard, CompressMode::None, None)
                                    .await;
                            }
                            break;
                        }
                    }
                };
                req.client_addr = Some(client_addr);
                req.add_ext(Arc::clone(&stream));
                let cmode = req.get_header_accept_encoding();
                let conn = req.get_header_connection();
                let mut res = PipeContext::handle_request(pipe_ctx.as_ref(), &mut req, 0).await;
                if conn != HttpConnection::KeepAlive {
                    res.add_header("Connection".into(), "close".into());
                }
                let stream_for_write = req.exts.remove(&TypeId::of::<Mutex<HttpStream>>());
                match stream_for_write {
                    Some(stream_in_req) => {
                        drop(stream_in_req);
                        let write_res = if let Some(stream_mutex) = Arc::get_mut(&mut stream) {
                            res.write_to_stream(stream_mutex.get_mut(), cmode, Some(req.method))
                                .await
                        } else {
                            let mut stream = stream.lock().await;
                            res.write_to_stream(&mut stream, cmode, Some(req.method))
                                .await
                        };
                        match write_res {
                            Ok(()) => {
                                if n > 0 {
                                    let remain = buf.len().saturating_sub(n);
                                    if remain > 0 {
                                        buf.copy_within(n.., 0);
                                    }
                                    buf.truncate(remain);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    None => break,
                }
                if conn != HttpConnection::KeepAlive {
                    break;
                }
            }
        });
    }

    #[cfg(feature = "tls")]
    fn tls_acceptor_with_alpn(
        cert_file: &str,
        key_file: &str,
        alpn: Option<Vec<Vec<u8>>>,
    ) -> anyhow::Result<TlsAcceptor> {
        let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_file)?;
        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        if let Some(alpn) = alpn {
            config.alpn_protocols = alpn;
        }
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    #[cfg(feature = "http3")]
    fn quinn_server_config(cert_file: &str, key_file: &str) -> anyhow::Result<quinn::ServerConfig> {
        let certs = CertificateDer::pem_file_iter(cert_file)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_file)?;

        let mut tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        tls_config.max_early_data_size = u32::MAX;
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        Ok(quinn::ServerConfig::with_crypto(Arc::new(
            QuicServerConfig::try_from(tls_config)?,
        )))
    }

    #[cfg(any(feature = "http2", feature = "http3"))]
    fn h2_method_to_http_method(method: &http::Method) -> anyhow::Result<HttpMethod> {
        Ok(match method.as_str() {
            "GET" => HttpMethod::GET,
            "PUT" => HttpMethod::PUT,
            "COPY" => HttpMethod::COPY,
            "HEAD" => HttpMethod::HEAD,
            "LOCK" => HttpMethod::LOCK,
            "MOVE" => HttpMethod::MOVE,
            "POST" => HttpMethod::POST,
            "MKCOL" => HttpMethod::MKCOL,
            "PATCH" => HttpMethod::PATCH,
            "TRACE" => HttpMethod::TRACE,
            "DELETE" => HttpMethod::DELETE,
            "UNLOCK" => HttpMethod::UNLOCK,
            "CONNECT" => HttpMethod::CONNECT,
            "OPTIONS" => HttpMethod::OPTIONS,
            "PROPFIND" => HttpMethod::PROPFIND,
            "PROPPATCH" => HttpMethod::PROPPATCH,
            _ => anyhow::bail!("unsupported method: {method}"),
        })
    }

    #[cfg(any(feature = "http2", feature = "http3"))]
    fn is_h2_h3_forbidden_response_header(name: &str) -> bool {
        name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("te")
            || name.eq_ignore_ascii_case("trailer")
    }

    #[cfg(any(feature = "http2", feature = "http3"))]
    fn is_forbidden_trailer_for_h2_h3(name: &str) -> bool {
        name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("trailer")
            || name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("te")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || name.eq_ignore_ascii_case("proxy-authorization")
    }

    #[cfg(any(feature = "http2", feature = "http3"))]
    fn should_suppress_response_body(status: u16, request_method: HttpMethod) -> bool {
        (100..200).contains(&status)
            || status == 204
            || status == 304
            || request_method == HttpMethod::HEAD
    }

    #[cfg(feature = "http2")]
    async fn handle_h2_request(
        mut req_head: http::Request<h2::RecvStream>,
        mut respond: h2_server::SendResponse<bytes::Bytes>,
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
    ) -> anyhow::Result<()> {
        let mut req = HttpRequest::new();
        req.method = Self::h2_method_to_http_method(req_head.method())?;
        req.target_form = HttpRequestTargetForm::Origin;
        req.version = 20;
        req.client_addr = Some(client_addr);

        let path_and_query = req_head
            .uri()
            .path_and_query()
            .map(|v| v.as_str())
            .unwrap_or("/");
        match path_and_query.split_once('?') {
            Some((path, query)) => {
                req.url_path = path.into();
                req.url_query = query
                    .split('&')
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| (a.into(), b.into()))
                    .collect();
            }
            None => {
                req.url_path = path_and_query.into();
            }
        }

        let authority = req_head.uri().authority().map(|v| v.as_str().to_string());
        for (key, value) in req_head.headers().iter() {
            if let Ok(value) = value.to_str() {
                req.headers
                    .insert(HeaderOrHipStr::from_str(key.as_str()), value.into());
            }
        }
        if let Some(authority) = authority {
            if let Some(host) = req.get_header("Host") {
                if !host.eq_ignore_ascii_case(&authority) {
                    let head = http::Response::builder().status(400).body(())?;
                    let _ = respond.send_response(head, true)?;
                    return Ok(());
                }
            }
            req.headers
                .insert(HeaderOrHipStr::from_str("Host"), authority.into());
        }

        let body_stream = req_head.body_mut();
        let mut request_body = Vec::new();
        while let Some(next) = body_stream.data().await {
            let chunk = next?;
            request_body.extend_from_slice(&chunk);
        }
        req.body = request_body.into();

        let res = PipeContext::handle_request(pipe_ctx.as_ref(), &mut req, 0).await;

        let suppress_body = Self::should_suppress_response_body(res.http_code, req.method);
        let mut response_builder = http::Response::builder().status(res.http_code);
        for (key, value) in res.headers.iter() {
            if Self::is_h2_h3_forbidden_response_header(key) {
                continue;
            }
            response_builder = response_builder.header(key.as_ref(), value.as_ref());
        }

        let mut trailers = http::HeaderMap::new();
        for (key, value) in res.trailers.iter() {
            if Self::is_forbidden_trailer_for_h2_h3(key) {
                continue;
            }
            if let Ok(name) = http::header::HeaderName::from_bytes(key.as_bytes()) {
                if let Ok(value) = http::HeaderValue::from_str(value) {
                    trailers.insert(name, value);
                }
            }
        }
        let has_trailers = !trailers.is_empty();

        if suppress_body {
            let head = response_builder.body(())?;
            let _ = respond.send_response(head, true)?;
            return Ok(());
        }

        match res.body {
            crate::HttpResponseBody::Data(data) => {
                let head = response_builder.body(())?;
                let mut send = respond.send_response(head, data.is_empty() && !has_trailers)?;
                if !data.is_empty() {
                    send.send_data(data.into(), !has_trailers)?;
                }
                if has_trailers {
                    send.send_trailers(trailers)?;
                }
            }
            crate::HttpResponseBody::Stream(mut rx) => {
                let head = response_builder.body(())?;
                let mut send = respond.send_response(head, false)?;
                while let Some(chunk) = rx.recv().await {
                    if chunk.is_empty() {
                        continue;
                    }
                    send.send_data(chunk.into(), false)?;
                }
                if has_trailers {
                    send.send_trailers(trailers)?;
                } else {
                    send.send_data(Vec::<u8>::new().into(), true)?;
                }
            }
        }
        Ok(())
    }

    async fn serve_http_impl(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            _ = stream.set_nodelay(true);
            Self::spawn_http1_connection(
                Arc::clone(&pipe_ctx),
                client_addr,
                HttpStream::from_tcp(stream),
            );
        }
    }

    #[cfg(feature = "tls")]
    async fn serve_https_impl(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        let acceptor = Self::tls_acceptor_with_alpn(cert_file, key_file, None)?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            _ = stream.set_nodelay(true);
            let acceptor = acceptor.clone();
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            _ = tokio::task::spawn(async move {
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(_) => return,
                };
                Self::spawn_http1_connection(
                    pipe_ctx2,
                    client_addr,
                    HttpStream::from_server_tls(stream),
                );
            });
        }
    }

    #[cfg(feature = "http2")]
    async fn serve_http2_impl(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        let acceptor = Self::tls_acceptor_with_alpn(
            cert_file,
            key_file,
            Some(vec![b"h2".to_vec(), b"http/1.1".to_vec()]),
        )?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            _ = stream.set_nodelay(true);
            let acceptor = acceptor.clone();
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            _ = tokio::task::spawn(async move {
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(_) => return,
                };

                let negotiated_h2 = stream
                    .get_ref()
                    .1
                    .alpn_protocol()
                    .map(|p| p == b"h2")
                    .unwrap_or(false);

                if !negotiated_h2 {
                    Self::spawn_http1_connection(
                        pipe_ctx2,
                        client_addr,
                        HttpStream::from_server_tls(stream),
                    );
                    return;
                }

                let mut h2_conn = match h2_server::handshake(stream).await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };

                while let Some(next) = h2_conn.accept().await {
                    let (req_head, respond) = match next {
                        Ok(parts) => parts,
                        Err(_) => break,
                    };
                    let pipe_ctx3 = Arc::clone(&pipe_ctx2);
                    _ = tokio::task::spawn(async move {
                        let _ = Self::handle_h2_request(req_head, respond, pipe_ctx3, client_addr)
                            .await;
                    });
                }
            });
        }
    }

    #[cfg(feature = "http3")]
    async fn serve_http3_impl(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let server_config = Self::quinn_server_config(cert_file, key_file)?;
        let endpoint = quinn::Endpoint::server(server_config, addr)?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);

        while let Some(new_conn) = endpoint.accept().await {
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            _ = tokio::task::spawn(async move {
                let conn = match new_conn.await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };
                let client_addr = conn.remote_address();
                let mut h3_conn: h3_server::Connection<_, bytes::Bytes> =
                    match h3_server::Connection::new(h3_quinn::Connection::new(conn)).await {
                        Ok(conn) => conn,
                        Err(_) => return,
                    };

                loop {
                    let resolver = match h3_conn.accept().await {
                        Ok(Some(resolver)) => resolver,
                        Ok(None) => break,
                        Err(_) => break,
                    };
                    let pipe_ctx3 = Arc::clone(&pipe_ctx2);
                    _ = tokio::task::spawn(async move {
                        let (req_head, mut stream) = match resolver.resolve_request().await {
                            Ok(req_stream) => req_stream,
                            Err(_) => return,
                        };

                        let mut req = HttpRequest::new();
                        req.method = match Self::h2_method_to_http_method(req_head.method()) {
                            Ok(method) => method,
                            Err(_) => return,
                        };
                        req.target_form = HttpRequestTargetForm::Origin;
                        req.version = 30;
                        req.client_addr = Some(client_addr);

                        let path_and_query = req_head
                            .uri()
                            .path_and_query()
                            .map(|v| v.as_str())
                            .unwrap_or("/");
                        match path_and_query.split_once('?') {
                            Some((path, query)) => {
                                req.url_path = path.into();
                                req.url_query = query
                                    .split('&')
                                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                                    .map(|(a, b)| (a.into(), b.into()))
                                    .collect();
                            }
                            None => {
                                req.url_path = path_and_query.into();
                            }
                        }

                        let authority = req_head.uri().authority().map(|v| v.as_str().to_string());
                        for (key, value) in req_head.headers().iter() {
                            if let Ok(value) = value.to_str() {
                                req.headers
                                    .insert(HeaderOrHipStr::from_str(key.as_str()), value.into());
                            }
                        }
                        if let Some(authority) = authority {
                            if let Some(host) = req.get_header("Host") {
                                if !host.eq_ignore_ascii_case(&authority) {
                                    let response =
                                        match http::Response::builder().status(400).body(()) {
                                            Ok(resp) => resp,
                                            Err(_) => return,
                                        };
                                    let _ = stream.send_response(response).await;
                                    let _ = stream.finish().await;
                                    return;
                                }
                            }
                            req.headers
                                .insert(HeaderOrHipStr::from_str("Host"), authority.into());
                        }

                        let mut request_body = Vec::new();
                        loop {
                            match stream.recv_data().await {
                                Ok(Some(mut chunk)) => {
                                    request_body
                                        .extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()));
                                }
                                Ok(None) => break,
                                Err(_) => return,
                            }
                        }
                        req.body = request_body.into();

                        let res =
                            PipeContext::handle_request(pipe_ctx3.as_ref(), &mut req, 0).await;

                        let mut response_builder = http::Response::builder().status(res.http_code);
                        for (key, value) in res.headers.iter() {
                            if Self::is_h2_h3_forbidden_response_header(key) {
                                continue;
                            }
                            response_builder =
                                response_builder.header(key.as_ref(), value.as_ref());
                        }
                        let response = match response_builder.body(()) {
                            Ok(resp) => resp,
                            Err(_) => return,
                        };
                        if stream.send_response(response).await.is_err() {
                            return;
                        }

                        let suppress_body =
                            Self::should_suppress_response_body(res.http_code, req.method);
                        if !suppress_body {
                            match res.body {
                                crate::HttpResponseBody::Data(data) => {
                                    if !data.is_empty()
                                        && stream.send_data(bytes::Bytes::from(data)).await.is_err()
                                    {
                                        return;
                                    }
                                }
                                crate::HttpResponseBody::Stream(mut rx) => {
                                    while let Some(chunk) = rx.recv().await {
                                        if stream
                                            .send_data(bytes::Bytes::from(chunk))
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                }
                            }
                        }

                        if !res.trailers.is_empty() {
                            let mut trailers = http::HeaderMap::new();
                            for (key, value) in res.trailers.iter() {
                                if Self::is_forbidden_trailer_for_h2_h3(key) {
                                    continue;
                                }
                                if let Ok(name) =
                                    http::header::HeaderName::from_bytes(key.as_bytes())
                                {
                                    if let Ok(value) = http::HeaderValue::from_str(value) {
                                        trailers.insert(name, value);
                                    }
                                }
                            }
                            let _ = stream.send_trailers(trailers).await;
                        }

                        let _ = stream.finish().await;
                    });
                }
            });
        }

        endpoint.wait_idle().await;
        Ok(())
    }
}
