#[cfg(feature = "http2")]
mod http2;
#[cfg(feature = "http3")]
mod http3;

use crate::utils::enums::HttpConnection;
use crate::utils::refstr::HeaderItem;
use crate::utils::tcp_stream::HttpStream;
use crate::CompressMode;
use crate::{
    HttpHandler, HttpMethod, HttpRequest, HttpRequestTargetForm, HttpResponse, PreflightResult,
};
use crate::{RequestHandlerFlag, TransferSession};
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
use tokio::time::{interval, Duration};
#[cfg(any(feature = "tls", feature = "http3"))]
use tokio_rustls::rustls;
#[cfg(any(feature = "tls", feature = "http3"))]
use tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
#[cfg(feature = "tls")]
use tokio_rustls::TlsAcceptor;

/// CORS配置
#[derive(Debug, Clone)]
pub struct CorsConfig {
    pub origin: Option<String>,         // Access-Control-Allow-Origin
    pub methods: Option<String>,        // Access-Control-Allow-Methods
    pub headers: Option<String>,        // Access-Control-Allow-Headers
    pub max_age: Option<String>,        // Access-Control-Max-Age
    pub credentials: bool,              // Access-Control-Allow-Credentials
    pub expose_headers: Option<String>, // Access-Control-Expose-Headers
}

impl CorsConfig {
    /// 创建最小限制默认配置
    pub fn default_minimal() -> Self {
        Self {
            origin: Some("*".to_string()),
            methods: None, // 自动计算
            headers: Some("*".to_string()),
            max_age: Some("86400".to_string()),
            credentials: false,
            expose_headers: None,
        }
    }
}

type AsyncCustomHandler = dyn Fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = Option<HttpResponse>> + Send + '_>>
    + Send
    + Sync;

type SyncCustomHandler = dyn Fn(&mut HttpRequest) -> Option<HttpResponse> + Send + Sync;

type GlobalPreprocessHandler = for<'a> fn(
    &'a mut HttpRequest,
) -> Pin<
    Box<dyn Future<Output = anyhow::Result<Option<HttpResponse>>> + Send + 'a>,
>;

type GlobalPostprocessHandler =
    for<'a> fn(
        &'a mut HttpRequest,
        &'a mut HttpResponse,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

// Re-export WebTransport types from http3 module
#[cfg(feature = "http3")]
pub use http3::{WebTransportConfig, WebTransportHandler, WebTransportSession, WebTransportStream};

#[derive(Clone)]
pub enum CustomHandler {
    Sync(Arc<SyncCustomHandler>),
    Async(Arc<AsyncCustomHandler>),
}

#[derive(Clone)]
pub enum PreprocessHandler {
    Fn(GlobalPreprocessHandler),
}

#[derive(Clone)]
pub enum PostprocessHandler {
    Fn(GlobalPostprocessHandler),
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

pub enum PipeContextItem {
    Handlers,
    LocationRoute((String, String, bool)),
    EmbeddedRoute(HashMap<String, Cow<'static, [u8]>>),
    FinalRoute(HttpResponse),
    Custom(CustomHandler),
    Preprocess(PreprocessHandler),
    Postprocess(PostprocessHandler),
    LimitSize(usize, usize), // (max_header_bytes, max_body_bytes)
    TransferRate(u64, u64),  // (入站速率限制 bits/sec, 出站速率限制 bits/sec)
    ReverseProxy(String, String, bool),
    #[cfg(feature = "jemalloc")]
    Jemalloc(String),
    #[cfg(feature = "webdav")]
    Webdav((String, dav_server::DavHandler)),
    #[cfg(feature = "http3")]
    WebTransport((String, WebTransportConfig, WebTransportHandler)),
    #[cfg(feature = "webrtc")]
    WebRTC((crate::webrtc::WebRTCConfig, crate::webrtc::WebRTCEvents)),
}

// 手动实现 Clone，因为 WebTransportHandler 不能 Clone
impl Clone for PipeContextItem {
    fn clone(&self) -> Self {
        match self {
            PipeContextItem::Handlers => PipeContextItem::Handlers,
            PipeContextItem::LocationRoute(v) => PipeContextItem::LocationRoute(v.clone()),
            PipeContextItem::EmbeddedRoute(v) => PipeContextItem::EmbeddedRoute(v.clone()),
            PipeContextItem::FinalRoute(v) => PipeContextItem::FinalRoute(v.clone()),
            PipeContextItem::Custom(v) => PipeContextItem::Custom(v.clone()),
            PipeContextItem::Preprocess(v) => PipeContextItem::Preprocess(v.clone()),
            PipeContextItem::Postprocess(v) => PipeContextItem::Postprocess(v.clone()),
            PipeContextItem::LimitSize(h, b) => PipeContextItem::LimitSize(*h, *b),
            PipeContextItem::TransferRate(r1, r2) => PipeContextItem::TransferRate(*r1, *r2),
            PipeContextItem::ReverseProxy(v1, v2, v3) => {
                PipeContextItem::ReverseProxy(v1.clone(), v2.clone(), *v3)
            }
            #[cfg(feature = "jemalloc")]
            PipeContextItem::Jemalloc(v) => PipeContextItem::Jemalloc(v.clone()),
            #[cfg(feature = "webdav")]
            PipeContextItem::Webdav(v) => PipeContextItem::Webdav(v.clone()),
            #[cfg(feature = "http3")]
            PipeContextItem::WebTransport(_) => panic!("WebTransport handler cannot be cloned"),
            #[cfg(feature = "webrtc")]
            PipeContextItem::WebRTC(v) => PipeContextItem::WebRTC(v.clone()),
        }
    }
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

    pub fn use_location_route(
        &mut self,
        url_path: impl Into<String>,
        loc_path: impl Into<String>,
        allow_symlink_escape: bool,
    ) {
        let (url_path, loc_path) = (url_path.into(), loc_path.into());
        self.items.push(PipeContextItem::LocationRoute((
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

    /// 添加全局预处理函数
    ///
    /// 预处理函数在所有路由处理之前执行，可以用于认证检查、日志记录等。
    /// 如果返回 `Some(response)`，则直接返回该响应，跳过后续所有处理。
    ///
    /// # 参数
    /// * `handler` - 通过 `#[potato::preprocess]` 宏标注的预处理函数
    ///
    /// # 示例
    /// ```rust,ignore
    /// #[potato::preprocess]
    /// async fn my_preprocess(req: &mut HttpRequest) -> Option<HttpResponse> {
    ///     // 预处理逻辑
    ///     None
    /// }
    ///
    /// server.configure(|ctx| {
    ///     ctx.use_preprocess(my_preprocess);
    ///     ctx.use_handlers();
    /// });
    /// ```
    pub fn use_preprocess(&mut self, handler: GlobalPreprocessHandler) {
        self.items
            .push(PipeContextItem::Preprocess(PreprocessHandler::Fn(handler)));
    }

    /// 添加全局后处理函数
    ///
    /// 后处理函数在 handler 生成响应后执行，可以修改响应内容（如添加响应头）。
    ///
    /// # 参数
    /// * `handler` - 通过 `#[potato::postprocess]` 宏标注的后处理函数
    ///
    /// # 示例
    /// ```rust,ignore
    /// #[potato::postprocess]
    /// async fn my_postprocess(req: &mut HttpRequest, res: &mut HttpResponse) {
    ///     res.add_header("X-Custom".into(), "value".into());
    /// }
    ///
    /// server.configure(|ctx| {
    ///     ctx.use_postprocess(my_postprocess);
    ///     ctx.use_handlers();
    /// });
    /// ```
    pub fn use_postprocess(&mut self, handler: GlobalPostprocessHandler) {
        self.items
            .push(PipeContextItem::Postprocess(PostprocessHandler::Fn(
                handler,
            )));
    }

    /// 添加请求体大小限制中间件
    ///
    /// # 参数
    /// * `max_header_bytes` - Header 总大小限制 (字节)
    /// * `max_body_bytes` - Body 总大小限制 (字节)
    ///
    /// # 示例
    /// ```rust
    /// let mut server = potato::HttpServer::new("127.0.0.1:8080");
    /// server.configure(|ctx| {
    ///     ctx.use_limit_size(1024 * 1024, 50 * 1024 * 1024); // 1MB header, 50MB body
    ///     ctx.use_handlers();
    /// });
    /// ```
    pub fn use_limit_size(&mut self, max_header_bytes: usize, max_body_bytes: usize) {
        self.items.push(PipeContextItem::LimitSize(
            max_header_bytes.max(1),
            max_body_bytes.max(1),
        ));
    }

    /// 添加传输速率限制中间件
    ///
    /// # 参数
    /// * `inbound_rate_bits_per_sec` - 入站最大传输速率（bits/sec），接收请求数据的速率限制
    /// * `outbound_rate_bits_per_sec` - 出站最大传输速率（bits/sec），发送响应数据的速率限制
    ///
    /// # 示例
    /// ```rust
    /// let mut server = potato::HttpServer::new("127.0.0.1:8080");
    /// server.configure(|ctx| {
    ///     ctx.use_transfer_limit(10_000_000, 20_000_000); // 入站 10 Mbps，出站 20 Mbps
    ///     ctx.use_handlers();
    /// });
    /// ```
    pub fn use_transfer_limit(
        &mut self,
        inbound_rate_bits_per_sec: u64,
        outbound_rate_bits_per_sec: u64,
    ) {
        if inbound_rate_bits_per_sec == 0 {
            panic!("Inbound transfer rate limit must be greater than 0");
        }
        if outbound_rate_bits_per_sec == 0 {
            panic!("Outbound transfer rate limit must be greater than 0");
        }
        self.items.push(PipeContextItem::TransferRate(
            inbound_rate_bits_per_sec,
            outbound_rate_bits_per_sec,
        ));
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
        static AUTHOR_REGEX: std::sync::LazyLock<Result<regex::Regex, regex::Error>> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"([[:word:]]+)\s*<([^>]+)>"));
        let contact = {
            match AUTHOR_REGEX
                .as_ref()
                .ok()
                .and_then(|re| re.captures(env!("CARGO_PKG_AUTHORS")))
            {
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

    #[cfg(feature = "http3")]
    pub fn use_webtransport<F, Fut>(&mut self, url_path: impl Into<String>, handler: F)
    where
        F: Fn(WebTransportSession) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.items.push(PipeContextItem::WebTransport((
            url_path.into(),
            WebTransportConfig::default(),
            Box::new(move |session| Box::pin(handler(session))),
        )));
    }

    #[cfg(feature = "webrtc")]
    pub fn use_webrtc(&mut self) -> crate::webrtc::WebRTCBuilder<'_> {
        crate::webrtc::WebRTCBuilder::new(self)
    }

    #[cfg(feature = "webrtc")]
    pub(crate) fn add_webrtc(
        &mut self,
        config: crate::webrtc::WebRTCConfig,
        events: crate::webrtc::WebRTCEvents,
    ) {
        self.items.push(PipeContextItem::WebRTC((config, events)));
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

        // 收集所有 Postprocess handlers
        let postprocess_handlers: Vec<&PostprocessHandler> = self2
            .items
            .iter()
            .filter_map(|item| {
                if let PipeContextItem::Postprocess(handler) = item {
                    Some(handler)
                } else {
                    None
                }
            })
            .collect();

        // 执行 Postprocess 的辅助函数
        async fn execute_postprocess(
            handlers: &[&PostprocessHandler],
            req: &mut HttpRequest,
            res: &mut HttpResponse,
        ) {
            for handler in handlers {
                match handler {
                    PostprocessHandler::Fn(fn_handler) => {
                        if let Err(e) = fn_handler(req, res).await {
                            eprintln!("[Postprocess] Error: {}", e);
                        }
                    }
                }
            }
        }

        for (_idx, item) in self2.items.iter().enumerate().skip(skip) {
            match item {
                PipeContextItem::Postprocess(_) => {
                    // Postprocess 已在函数开始时收集,在此跳过
                    continue;
                }
                PipeContextItem::Handlers => {
                    let handler_ref = HANDLERS_FLAT
                        .get(&(&req.url_path[..], req.method))
                        .map(|p| p.handler);
                    if let Some(handler_ref) = handler_ref {
                        let mut res = match handler_ref {
                            HttpHandler::Async(handler) => handler(req).await,
                            HttpHandler::Sync(handler) => handler(req),
                        };
                        execute_postprocess(&postprocess_handlers, req, &mut res).await;
                        return res;
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
                                execute_postprocess(&postprocess_handlers, req, &mut res).await;
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

                            res2.add_header("Allow".into(), methods_str);
                            execute_postprocess(&postprocess_handlers, req, &mut res2).await;
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
                        None => {
                            let mut res = HttpResponse::error("url path over directory");
                            execute_postprocess(&postprocess_handlers, req, &mut res).await;
                            return res;
                        }
                    };
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if meta.is_file() {
                            if let Some(root) = canonical_root.as_ref() {
                                if !Self::path_stays_inside_root(&path, root) {
                                    let mut res = HttpResponse::error("url path over directory");
                                    execute_postprocess(&postprocess_handlers, req, &mut res).await;
                                    return res;
                                }
                            }
                            if let Some(path) = path.to_str() {
                                let mut res = Self::from_static_file(req, path, &meta);
                                execute_postprocess(&postprocess_handlers, req, &mut res).await;
                                return res;
                            }
                        } else if meta.is_dir() {
                            if let Some(root) = canonical_root.as_ref() {
                                if !Self::path_stays_inside_root(&path, root) {
                                    let mut res = HttpResponse::error("url path over directory");
                                    execute_postprocess(&postprocess_handlers, req, &mut res).await;
                                    return res;
                                }
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.htm");
                            if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                if tmp_meta.is_file() {
                                    if let Some(root) = canonical_root.as_ref() {
                                        if !Self::path_stays_inside_root(&tmp_path, root) {
                                            let mut res =
                                                HttpResponse::error("url path over directory");
                                            execute_postprocess(
                                                &postprocess_handlers,
                                                req,
                                                &mut res,
                                            )
                                            .await;
                                            return res;
                                        }
                                    }
                                    if let Some(path) = tmp_path.to_str() {
                                        let mut res = Self::from_static_file(req, path, &tmp_meta);
                                        execute_postprocess(&postprocess_handlers, req, &mut res)
                                            .await;
                                        return res;
                                    }
                                }
                            }
                            let mut tmp_path = path.clone();
                            tmp_path.push("index.html");
                            if let Ok(tmp_meta) = std::fs::metadata(&tmp_path) {
                                if tmp_meta.is_file() {
                                    if let Some(root) = canonical_root.as_ref() {
                                        if !Self::path_stays_inside_root(&tmp_path, root) {
                                            let mut res =
                                                HttpResponse::error("url path over directory");
                                            execute_postprocess(
                                                &postprocess_handlers,
                                                req,
                                                &mut res,
                                            )
                                            .await;
                                            return res;
                                        }
                                    }
                                    if let Some(path) = tmp_path.to_str() {
                                        let mut res = Self::from_static_file(req, path, &tmp_meta);
                                        execute_postprocess(&postprocess_handlers, req, &mut res)
                                            .await;
                                        return res;
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
                                execute_postprocess(&postprocess_handlers, req, &mut res).await;
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
                                execute_postprocess(&postprocess_handlers, req, &mut res).await;
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
                                        execute_postprocess(&postprocess_handlers, req, &mut res)
                                            .await;
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
                                        execute_postprocess(&postprocess_handlers, req, &mut res)
                                            .await;
                                        return res;
                                    }
                                }
                            }
                        }

                        let mut ret =
                            HttpResponse::from_mem_file(&req.url_path, item.to_vec(), false, None);
                        ret.add_header("Accept-Ranges".into(), "bytes".into());
                        Self::add_embedded_validators(&mut ret, meta.as_ref(), etag.as_str());
                        execute_postprocess(&postprocess_handlers, req, &mut ret).await;
                        return ret;
                    }
                    continue;
                }
                PipeContextItem::FinalRoute(res) => {
                    let mut res = res.clone();
                    execute_postprocess(&postprocess_handlers, req, &mut res).await;
                    return res;
                }
                PipeContextItem::LimitSize(_max_header, max_body) => {
                    // 检查 body 大小
                    let body_len = req.body.len();
                    if body_len > *max_body {
                        let mut res = HttpResponse::text(format!(
                            "Payload Too Large: body size {} bytes exceeds limit {} bytes",
                            body_len, max_body
                        ));
                        res.http_code = 413;
                        execute_postprocess(&postprocess_handlers, req, &mut res).await;
                        return res;
                    }
                    // Header 大小已在解析阶段检查，此处为双重保险
                    continue;
                }
                PipeContextItem::TransferRate(_inbound_rate, _outbound_rate) => {
                    // 速率限制在连接层处理，此处不需要额外处理
                    continue;
                }
                PipeContextItem::Custom(handler) => match handler {
                    CustomHandler::Sync(handler) => match handler.as_ref()(req) {
                        Some(mut res) => {
                            execute_postprocess(&postprocess_handlers, req, &mut res).await;
                            return res;
                        }
                        None => continue,
                    },
                    CustomHandler::Async(handler) => match handler.as_ref()(req).await {
                        Some(mut res) => {
                            execute_postprocess(&postprocess_handlers, req, &mut res).await;
                            return res;
                        }
                        None => continue,
                    },
                },
                PipeContextItem::Preprocess(handler) => {
                    match handler {
                        PreprocessHandler::Fn(fn_handler) => {
                            match fn_handler(req).await {
                                Ok(Some(mut response)) => {
                                    execute_postprocess(&postprocess_handlers, req, &mut response)
                                        .await;
                                    return response;
                                }
                                Ok(None) => {} // 继续处理
                                Err(e) => {
                                    let mut res =
                                        HttpResponse::error(format!("Preprocess error: {e}"));
                                    execute_postprocess(&postprocess_handlers, req, &mut res).await;
                                    return res;
                                }
                            }
                        }
                    }
                }
                PipeContextItem::ReverseProxy(path, proxy_url, modify_content) => {
                    if !req.url_path.starts_with(path) {
                        continue;
                    }

                    let mut transfer_session =
                        TransferSession::from_reverse_proxy(path.clone(), proxy_url.clone());

                    match transfer_session.transfer(req, *modify_content).await {
                        Ok(mut response) => {
                            execute_postprocess(&postprocess_handlers, req, &mut response).await;
                            return response;
                        }
                        Err(err) => {
                            let mut res = HttpResponse::error(format!("{err}"));
                            execute_postprocess(&postprocess_handlers, req, &mut res).await;
                            return res;
                        }
                    }
                }

                #[cfg(feature = "jemalloc")]
                PipeContextItem::Jemalloc(path) => {
                    if path == &req.url_path[..] {
                        let mut res = match crate::dump_jemalloc_profile().await {
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
                        execute_postprocess(&postprocess_handlers, req, &mut res).await;
                        return res;
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
                                *new_req.uri_mut() = match http::uri::Builder::new()
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
                                {
                                    Ok(uri) => uri,
                                    Err(e) => {
                                        return HttpResponse::error(format!(
                                            "Failed to build URI: {e}"
                                        ));
                                    }
                                };
                            }
                            Err(_) => {
                                // If original URI is not available, construct with path only
                                *new_req.uri_mut() = match http::uri::Builder::new()
                                    .path_and_query(final_path)
                                    .build()
                                {
                                    Ok(uri) => uri,
                                    Err(e) => {
                                        return HttpResponse::error(format!(
                                            "Failed to build URI: {e}"
                                        ));
                                    }
                                };
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
                    let mut res = res;
                    execute_postprocess(&postprocess_handlers, req, &mut res).await;
                    return res;
                }
                #[cfg(feature = "webrtc")]
                PipeContextItem::WebRTC((config, _events)) => {
                    // WebRTC信令处理
                    // WebSocket信令在WebSocket upgrade时处理
                    // REST信令在这里处理
                    if req.url_path.starts_with(&config.rest_prefix) {
                        // 处理REST信令请求
                        // TODO: 实现完整的REST信令处理逻辑
                        // 目前返回提示信息
                        let host = req.get_header("Host").unwrap_or("127.0.0.1:8080");
                        let json_response = serde_json::json!({
                            "status": "WebRTC REST signaling endpoint",
                            "ws_url": format!("ws://{host}{}", config.ws_path),
                            "rest_prefix": config.rest_prefix,
                        });
                        let mut res = HttpResponse::json(json_response.to_string());
                        res.add_header("Content-Type".into(), "application/json".into());
                        execute_postprocess(&postprocess_handlers, req, &mut res).await;
                        return res;
                    }
                }
                #[cfg(feature = "http3")]
                PipeContextItem::WebTransport(_) => {
                    // WebTransport 在 HTTP/3 层处理，这里不会到达
                    // CONNECT 请求已经在 serve_http3_impl 中处理
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
    #[cfg(feature = "acme")]
    acme_manager: Option<crate::acme::AcmeManager>,
    #[cfg(feature = "acme")]
    acme_acceptor: Option<crate::acme::DynamicTlsAcceptor>,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer {
            addr: addr.into(),
            pipe_ctx: Arc::new(PipeContext::new()),
            shutdown_signal: None,
            #[cfg(feature = "acme")]
            acme_manager: None,
            #[cfg(feature = "acme")]
            acme_acceptor: None,
        }
    }

    /// 启动后台SessionCache清理任务
    /// 该任务会定期清理过期的session缓存
    fn start_session_cache_cleanup() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);

        // 确保只启动一次
        if CLEANUP_STARTED.swap(true, Ordering::Relaxed) {
            return;
        }

        tokio::spawn(async {
            let mut interval = interval(Duration::from_secs(60)); // 每60秒清理一次
            loop {
                interval.tick().await;
                // 调用SessionCache的清理方法
                crate::SessionCache::cleanup_expired_sessions();
            }
        });
    }

    pub fn configure(&mut self, callback: impl Fn(&mut PipeContext)) {
        let mut ctx = PipeContext::empty();
        callback(&mut ctx);
        self.pipe_ctx = Arc::new(ctx);
    }

    pub fn shutdown_signal(&mut self) -> Option<oneshot::Sender<()>> {
        if self.shutdown_signal.is_some() {
            return None; // Signal already set
        }
        let (tx, rx) = oneshot::channel();
        self.shutdown_signal = Some(rx);
        Some(tx)
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
        let pipe_ctx = Arc::clone(&self.pipe_ctx);
        let addr = self.addr.clone();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = http2::serve_http2_impl(&addr, cert_file, key_file, pipe_ctx) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => {
                http2::serve_http2_impl(&self.addr, cert_file, key_file, Arc::clone(&self.pipe_ctx))
                    .await
            }
        }
    }

    #[cfg(feature = "http3")]
    pub async fn serve_http3(&mut self, cert_file: &str, key_file: &str) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        let pipe_ctx = Arc::clone(&self.pipe_ctx);
        let addr = self.addr.clone();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = http3::serve_http3_impl(&addr, cert_file, key_file, pipe_ctx) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => {
                http3::serve_http3_impl(&self.addr, cert_file, key_file, Arc::clone(&self.pipe_ctx))
                    .await
            }
        }
    }

    /// 启动 HTTP/3 服务器（无加密模式，使用 http:// 协议）
    #[cfg(feature = "http3")]
    pub async fn serve_http3_without_encrypt(&mut self) -> anyhow::Result<()> {
        let shutdown_signal = self.shutdown_signal.take();
        let pipe_ctx = Arc::clone(&self.pipe_ctx);
        let addr = self.addr.clone();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = http3::serve_http3_without_encrypt_impl(&addr, pipe_ctx) => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => {
                http3::serve_http3_without_encrypt_impl(&self.addr, Arc::clone(&self.pipe_ctx))
                    .await
            }
        }
    }

    #[cfg(feature = "acme")]
    pub async fn serve_acme(
        &mut self,
        domain: impl Into<String>,
        email: impl Into<String>,
    ) -> anyhow::Result<()> {
        let opts = crate::acme::AcmeOptions::new(domain, email);
        self.serve_acme_with_opts(opts).await
    }

    #[cfg(feature = "acme")]
    pub async fn serve_acme_with_opts(
        &mut self,
        opts: crate::acme::AcmeOptions,
    ) -> anyhow::Result<()> {
        let (acme_manager, acme_acceptor) = crate::acme::AcmeManager::new(opts).await?;

        // 启动后台续期循环
        let manager_clone = acme_manager.clone();
        let acceptor_clone = acme_acceptor.clone();
        tokio::spawn(async move {
            if let Err(e) = manager_clone.start_renewal_loop(acceptor_clone).await {
                eprintln!("[ACME] Renewal loop error: {e}");
            }
        });

        self.acme_manager = Some(acme_manager);
        self.acme_acceptor = Some(acme_acceptor);

        let shutdown_signal = self.shutdown_signal.take();
        match shutdown_signal {
            Some(shutdown_signal) => {
                select! {
                    result = self.serve_acme_impl() => result,
                    _ = shutdown_signal => Ok(()),
                }
            }
            None => self.serve_acme_impl().await,
        }
    }

    pub(crate) fn spawn_http1_connection(
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
        stream: HttpStream,
    ) {
        // 检查是否有速率限制配置
        let rate_limit = pipe_ctx.items.iter().find_map(|item| {
            if let PipeContextItem::TransferRate(inbound, outbound) = item {
                Some((*inbound, *outbound))
            } else {
                None
            }
        });

        // 如果有速率限制，使用 RateLimitedStream 包装
        let stream: HttpStream = if let Some((inbound_rate, outbound_rate)) = rate_limit {
            HttpStream::RateLimited(crate::utils::tcp_stream::RateLimitedStream::new(
                stream,
                inbound_rate,
                outbound_rate,
            ))
        } else {
            stream
        };

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
        // 初始化 rustls CryptoProvider（如果尚未初始化）
        {
            use rustls::crypto::ring::default_provider;
            use rustls::crypto::CryptoProvider;
            let _ = CryptoProvider::install_default(default_provider());
        }

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

    async fn serve_http_impl(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        // 启动后台SessionCache清理任务
        Self::start_session_cache_cleanup();

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

    #[cfg(feature = "acme")]
    async fn serve_acme_impl(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "jemalloc")]
        crate::init_jemalloc()?;

        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;
        let acme_acceptor = self
            .acme_acceptor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ACME acceptor not initialized"))?;
        let pipe_ctx = Arc::clone(&self.pipe_ctx);
        let acme_manager = self
            .acme_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ACME manager not initialized"))?;
        let acme_manager_clone = acme_manager.clone();

        loop {
            let (stream, client_addr) = listener.accept().await?;
            _ = stream.set_nodelay(true);
            let acceptor = acme_acceptor.get_acceptor().await;
            let pipe_ctx2 = Arc::clone(&pipe_ctx);
            let acme_manager2 = acme_manager_clone.clone();
            let acceptor_clone = acceptor.clone();

            _ = tokio::task::spawn(async move {
                let stream = match acceptor_clone.accept(stream).await {
                    Ok(stream) => stream,
                    Err(_) => return,
                };

                // 直接处理ACME挑战请求
                Self::handle_acme_or_normal(
                    pipe_ctx2,
                    client_addr,
                    HttpStream::from_server_tls(stream),
                    &acme_manager2,
                )
                .await;
            });
        }
    }

    #[cfg(feature = "acme")]
    async fn handle_acme_or_normal(
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
        mut stream: HttpStream,
        acme_manager: &crate::acme::AcmeManager,
    ) {
        // 先读取部分数据检查是否是ACME挑战
        let mut buf = vec![0u8; 4096];
        let n = match stream.read(&mut buf).await {
            Ok(n) => n,
            Err(_) => return,
        };

        if n == 0 {
            return;
        }

        // 检查是否是ACME挑战请求
        let initial_data = String::from_utf8_lossy(&buf[..n]);
        if initial_data.contains("/.well-known/acme-challenge/") {
            // 解析请求路径
            if let Some(path_start) = initial_data.find("/.well-known/acme-challenge/") {
                let path_end = initial_data[path_start..]
                    .find(|c: char| c.is_whitespace() || c == ' ')
                    .map(|e| path_start + e)
                    .unwrap_or(initial_data.len());
                let full_path = &initial_data[path_start..path_end];
                let token = &full_path["/.well-known/acme-challenge/".len()..];

                let challenges = acme_manager.get_challenges().await;
                for challenge in challenges {
                    if challenge.token == token {
                        // 返回ACME挑战响应
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            challenge.key_authorization.len(),
                            challenge.key_authorization
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            }
        }

        // 正常HTTP请求处理 - 需要重新实现完整请求处理
        // 这里简化处理，实际应该将initial_data和后续数据一起处理
        // 由于复杂度较高，暂时只支持已缓存证书的常规请求
        Self::spawn_http1_connection_with_initial(pipe_ctx, client_addr, stream, &buf[..n]);
    }

    #[cfg(feature = "acme")]
    fn spawn_http1_connection_with_initial(
        pipe_ctx: Arc<PipeContext>,
        client_addr: SocketAddr,
        stream: HttpStream,
        initial_data: &[u8],
    ) {
        // 使用WithPreRead包装流，将initial_data作为预读取数据
        let stream_with_pre_read = HttpStream::with_pre_read(stream, initial_data.to_vec());
        Self::spawn_http1_connection(pipe_ctx, client_addr, stream_with_pre_read);
    }
}
