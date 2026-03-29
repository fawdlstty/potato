use crate::utils::enums::HttpConnection;
use crate::utils::refstr::HeaderItem;
use crate::utils::tcp_stream::HttpStream;
use crate::{HttpHandler, HttpMethod, HttpRequest, HttpResponse, PreflightResult};
use crate::{RequestHandlerFlag, TransferSession};
use std::any::TypeId;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;
use tokio::net::TcpListener;
use tokio::select;
use tokio::sync::{oneshot, Mutex};
#[cfg(feature = "tls")]
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
#[cfg(feature = "tls")]
use tokio_rustls::{rustls, TlsAcceptor};

type CustomHandler = dyn Fn(
        &mut HttpRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<HttpResponse>>> + Send + '_>>
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
    LocationRoute((String, String)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
    Custom(Arc<CustomHandler>),
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
                return res;
            }
            PreflightResult::PreconditionFailed => {
                let mut res = HttpResponse::error("Precondition Failed");
                res.http_code = 412;
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

        let mut res = HttpResponse::from_file(path, false, std::fs::metadata(path).ok());
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
        F: for<'a> Fn(
                &'a mut HttpRequest,
            ) -> Pin<
                Box<dyn Future<Output = anyhow::Result<Option<HttpResponse>>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    {
        self.items.push(PipeContextItem::Custom(Arc::new(callback)));
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

                            let mut res = HttpResponse::not_found();
                            res.body = crate::HttpResponseBody::Data(vec![]);
                            return res;
                        } else if req.method == HttpMethod::OPTIONS {
                            let mut res2 = HttpResponse::html("");
                            let methods_str: Cow<'static, str> = {
                                let mut options: HashSet<_> =
                                    [HttpMethod::HEAD, HttpMethod::OPTIONS]
                                        .into_iter()
                                        .collect();
                                if let Some(handlers) = HANDLERS.get(&req.url_path[..]) {
                                    options.extend(handlers.keys().map(|p| *p));
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
                PipeContextItem::LocationRoute((url_path, loc_path)) => {
                    if !req.url_path.starts_with(url_path) {
                        continue;
                    }
                    let static_root = {
                        let mut root = match PathBuf::from(loc_path).canonicalize() {
                            Ok(root) => root.to_string_lossy().to_string(),
                            Err(_) => continue,
                        };
                        if root.starts_with("\\\\?\\") {
                            root.drain(..4);
                        }
                        #[cfg(target_os = "windows")]
                        {
                            root = root.to_ascii_lowercase();
                        }
                        root
                    };
                    let mut path = PathBuf::new();
                    path.push(loc_path);
                    path.push(&req.url_path[url_path.len()..]);
                    if let Ok(path) = path.canonicalize() {
                        let mut temp_path = path.to_string_lossy().to_string();
                        if temp_path.starts_with("\\\\?\\") {
                            temp_path.drain(..4);
                        }
                        #[cfg(target_os = "windows")]
                        {
                            temp_path = temp_path.to_ascii_lowercase();
                        }
                        if !temp_path.starts_with(&static_root) {
                            return HttpResponse::error("url path over directory");
                        }
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if meta.is_file() {
                                if let Some(path) = path.to_str() {
                                    return Self::from_static_file(req, path, &meta);
                                }
                            } else if meta.is_dir() {
                                let mut tmp_path = path.clone();
                                tmp_path.push("index.htm");
                                if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                    if tmp_meta.is_file() {
                                        if let Some(path) = tmp_path.to_str() {
                                            return Self::from_static_file(req, path, &tmp_meta);
                                        }
                                    }
                                }
                                let mut tmp_path = path.clone();
                                tmp_path.push("index.html");
                                if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                    if tmp_meta.is_file() {
                                        if let Some(path) = tmp_path.to_str() {
                                            return Self::from_static_file(req, path, &tmp_meta);
                                        }
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
                            Some(format!("\"{:x}-{:x}\"", content_hash, item.len()))
                        };

                        // Execute preflight check
                        match req.check_precondition_headers(meta.as_ref(), etag.as_deref()) {
                            PreflightResult::NotModified => {
                                let mut res = HttpResponse::empty();
                                res.http_code = 304;
                                return res;
                            }
                            PreflightResult::PreconditionFailed => {
                                let mut res = HttpResponse::error("Precondition Failed");
                                res.http_code = 412;
                                return res;
                            }
                            PreflightResult::Proceed => {
                                // Continue processing
                            }
                        }

                        let ret =
                            HttpResponse::from_mem_file(&req.url_path, item.to_vec(), false, meta);
                        return ret;
                    }
                    continue;
                }
                PipeContextItem::FinalRoute(res) => return res.clone(),
                PipeContextItem::Custom(handler) => match handler.as_ref()(req).await {
                    Ok(Some(res)) => return res,
                    Ok(None) => continue,
                    Err(err) => return HttpResponse::error(format!("{err}")),
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
            _ = stream.set_nodelay(true);
            let mut stream = Arc::new(Mutex::new(HttpStream::from_tcp(stream)));
            _ = tokio::task::spawn(async move {
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (mut req, n) = {
                        match HttpRequest::from_stream(&mut buf, Arc::clone(&stream)).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    req.client_addr = Some(client_addr);
                    req.add_ext(Arc::clone(&stream));
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let mut res =
                        PipeContext::handle_request(pipe_ctx2.as_ref(), &mut req, 0).await;
                    let stream_for_write = req.exts.remove(&TypeId::of::<Mutex<HttpStream>>());
                    match stream_for_write {
                        Some(stream_in_req) => {
                            drop(stream_in_req);
                            let write_res = if let Some(stream_mutex) = Arc::get_mut(&mut stream) {
                                res.write_to_stream(stream_mutex.get_mut(), cmode).await
                            } else {
                                let mut stream = stream.lock().await;
                                res.write_to_stream(&mut stream, cmode).await
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
    }

    #[cfg(feature = "tls")]
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
            _ = stream.set_nodelay(true);
            let acceptor = acceptor.clone();
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let mut stream = Arc::new(Mutex::new(HttpStream::from_server_tls(stream)));
            _ = tokio::task::spawn(async move {
                let mut buf: Vec<u8> = Vec::with_capacity(4096);
                loop {
                    let (mut req, n) = {
                        match HttpRequest::from_stream(&mut buf, Arc::clone(&stream)).await {
                            Ok((req, n)) => (req, n),
                            Err(_) => break,
                        }
                    };
                    req.client_addr = Some(client_addr);
                    req.add_ext(Arc::clone(&stream));
                    let cmode = req.get_header_accept_encoding();
                    let conn = req.get_header_connection();
                    let mut res =
                        PipeContext::handle_request(pipe_ctx2.as_ref(), &mut req, 0).await;
                    let stream_for_write = req.exts.remove(&TypeId::of::<Mutex<HttpStream>>());
                    match stream_for_write {
                        Some(stream_in_req) => {
                            drop(stream_in_req);
                            let write_res = if let Some(stream_mutex) = Arc::get_mut(&mut stream) {
                                res.write_to_stream(stream_mutex.get_mut(), cmode).await
                            } else {
                                let mut stream = stream.lock().await;
                                res.write_to_stream(&mut stream, cmode).await
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
    }
}
