#[cfg(feature = "acme")]
#[path = "acme/mod.rs"]
pub mod acme;
#[path = "client/mod.rs"]
pub mod client;
#[path = "global_config.rs"]
pub mod global_config;
#[path = "server/mod.rs"]
pub mod server;
#[path = "utils/mod.rs"]
pub mod utils;
#[cfg(feature = "webrtc")]
#[path = "webrtc/mod.rs"]
pub mod webrtc;

pub use client::*;
pub use global_config::*;
pub use hipstr;
pub use inventory;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
pub use serde_json;
pub use server::CorsConfig;
pub use server::*;
pub use utils::refstr::Headers;

#[cfg(all(feature = "jemalloc", not(target_os = "windows")))]
pub use utils::jemalloc_helper::*;

#[cfg(feature = "webrtc")]
pub use webrtc::*;

use crate::ssformat;
use anyhow::anyhow;
use base64::Engine;
use chrono::Utc;
use core::str;
use hipstr::{LocalHipByt, LocalHipStr};
use http::uri::Scheme;
use http::Uri;
use rust_embed::Embed;
use rustc_hash::FxHashMap;
use sha1::{Digest, Sha1};
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::fs::{File, Metadata};
use std::io::{ErrorKind, Read};
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, collections::HashSet, future::Future, pin::Pin};
use strum::Display;
use thread_local::ThreadLocal;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use utils::bytes::CompressExt;
use utils::enums::{HttpConnection, HttpContentType};
use utils::number::HttpCodeExt;
use utils::refstr::{HeaderItem, HeaderOrHipStr};
use utils::string::StringExt;
use utils::tcp_stream::{HttpStream, VecU8Ext};

/// 一次性缓存,用于单次请求的前处理、后处理及handler方法间传递参数
#[derive(Debug)]
pub struct OnceCache {
    data: HashMap<(String, TypeId), Box<dyn Any + Send + Sync>>,
}

impl Default for OnceCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OnceCache {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    /// 获取不可变引用
    pub fn get<T: Any + Send + Sync + 'static>(&self, name: &str) -> Option<&T> {
        let key = (name.to_string(), TypeId::of::<T>());
        self.data
            .get(&key)
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// 获取值或返回默认值（需要类型实现Clone）
    pub fn get_or_default<T: Any + Send + Sync + Clone + 'static>(
        &self,
        name: &str,
        default: T,
    ) -> T {
        self.get::<T>(name).cloned().unwrap_or(default)
    }

    /// 获取可变引用
    pub fn get_mut<T: Any + Send + Sync + 'static>(&mut self, name: &str) -> Option<&mut T> {
        let key = (name.to_string(), TypeId::of::<T>());
        self.data
            .get_mut(&key)
            .and_then(|boxed| boxed.downcast_mut::<T>())
    }

    /// 插入或更新值
    pub fn set<T: Any + Send + Sync + 'static>(&mut self, name: &str, value: T) {
        let key = (name.to_string(), TypeId::of::<T>());
        self.data.insert(key, Box::new(value));
    }

    /// 移除并返回值
    pub fn remove<T: Any + Send + Sync + 'static>(&mut self, name: &str) -> Option<T> {
        let key = (name.to_string(), TypeId::of::<T>());
        self.data
            .remove(&key)
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    /// 检查是否包含指定的键
    pub fn contains_key<T: Any + Send + Sync + 'static>(&self, name: &str) -> bool {
        let key = (name.to_string(), TypeId::of::<T>());
        self.data.contains_key(&key)
    }

    /// 清空所有缓存
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// 获取缓存项数量
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 判断缓存是否为空
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

use dashmap::DashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// SessionCache 错误类型
#[derive(Debug)]
pub enum SessionCacheError {
    /// Token 解析失败（格式错误、签名验证失败等）
    InvalidToken(String),
    /// Token 已过期
    TokenExpired,
    /// Session 已过期
    SessionExpired,
    /// 缺少 Authorization header
    MissingAuthHeader,
    /// 内部错误
    InternalError(String),
}

impl std::fmt::Display for SessionCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionCacheError::InvalidToken(msg) => write!(f, "Invalid token: {msg}"),
            SessionCacheError::TokenExpired => write!(f, "Token has expired"),
            SessionCacheError::SessionExpired => write!(f, "Session has expired"),
            SessionCacheError::MissingAuthHeader => write!(f, "Missing Authorization header"),
            SessionCacheError::InternalError(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl std::error::Error for SessionCacheError {}

// SessionCache 的内部管理器(私有)
// 注意：JWT 密钥现在统一使用 global_config 中的 SERVER_JWT_SECRET
static SESSION_CACHE_MANAGER: std::sync::LazyLock<DashMap<[u8; 32], (SessionCache, Instant)>> =
    std::sync::LazyLock::new(DashMap::new);

/// Cookie属性配置
#[derive(Debug, Clone)]
pub struct CookieBuilder {
    /// Cookie名称
    name: String,
    /// Cookie值
    value: String,
    /// 路径（默认"/"）
    path: Option<String>,
    /// 域名
    domain: Option<String>,
    /// 过期时间（UTC时间戳，秒）
    expires: Option<i64>,
    /// 最大存活时间（秒）
    max_age: Option<i64>,
    /// 是否仅HTTPS传输
    secure: bool,
    /// 是否禁止JavaScript访问
    http_only: bool,
    /// SameSite策略: Strict, Lax, None
    same_site: Option<String>,
}

impl CookieBuilder {
    /// 创建新的Cookie
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_string(),
            value: value.to_string(),
            path: Some("/".to_string()),
            domain: None,
            expires: None,
            max_age: None,
            secure: false,
            http_only: false,
            same_site: None,
        }
    }

    /// 设置路径
    pub fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }

    /// 设置域名
    pub fn domain(mut self, domain: &str) -> Self {
        self.domain = Some(domain.to_string());
        self
    }

    /// 设置过期时间（Unix时间戳，秒）
    pub fn expires(mut self, timestamp: i64) -> Self {
        self.expires = Some(timestamp);
        self
    }

    /// 设置最大存活时间（秒）
    pub fn max_age(mut self, seconds: i64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// 设置Secure标志（仅HTTPS传输）
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// 设置HttpOnly标志（禁止JavaScript访问）
    pub fn http_only(mut self, http_only: bool) -> Self {
        self.http_only = http_only;
        self
    }

    /// 设置SameSite策略 ("Strict", "Lax", "None")
    pub fn same_site(mut self, policy: &str) -> Self {
        self.same_site = Some(policy.to_string());
        self
    }

    /// 生成Set-Cookie header值
    pub fn to_set_cookie_string(&self) -> String {
        let mut parts = vec![format!("{}={}", self.name, self.value)];

        if let Some(ref path) = self.path {
            parts.push(format!("Path={}", path));
        }

        if let Some(ref domain) = self.domain {
            parts.push(format!("Domain={}", domain));
        }

        if let Some(expires) = self.expires {
            // 将Unix时间戳转换为HTTP日期格式
            let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(expires, 0);
            if let Some(dt) = datetime {
                parts.push(format!(
                    "Expires={}",
                    dt.format("%a, %d %b %Y %H:%M:%S GMT")
                ));
            }
        }

        if let Some(max_age) = self.max_age {
            parts.push(format!("Max-Age={}", max_age));
        }

        if self.secure {
            parts.push("Secure".to_string());
        }

        if self.http_only {
            parts.push("HttpOnly".to_string());
        }

        if let Some(ref same_site) = self.same_site {
            parts.push(format!("SameSite={}", same_site));
        }

        parts.join("; ")
    }

    /// 生成删除cookie的Set-Cookie header值
    pub fn to_delete_cookie_string(&self) -> String {
        let mut parts = vec![
            format!("{}=", self.name),
            format!("Path={}", self.path.as_deref().unwrap_or("/")),
            "Expires=Thu, 01 Jan 1970 00:00:00 GMT".to_string(),
        ];

        if let Some(ref domain) = self.domain {
            parts.push(format!("Domain={}", domain));
        }

        parts.join("; ")
    }

    fn is_delete_cookie(&self) -> bool {
        self.value.is_empty() && self.expires == Some(0)
    }
}

/// SessionCache数据类型别名
type SessionCacheData = Arc<RwLock<HashMap<(String, TypeId), Box<dyn Any + Send + Sync>>>>;

/// 会话级缓存,用于同一用户的不同请求间传递参数
/// 基于Bearer token中的id区分不同Session
#[derive(Debug, Clone)]
pub struct SessionCache {
    data: SessionCacheData,
    /// 存储从请求中读取的cookies
    request_cookies: Arc<RwLock<HashMap<String, String>>>,
    /// 存储需要设置到响应的cookies（包含完整属性）
    response_cookies: Arc<RwLock<Vec<CookieBuilder>>>,
}

impl Default for SessionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionCache {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            request_cookies: Arc::new(RwLock::new(HashMap::new())),
            response_cookies: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 从请求的Cookie header中解析cookies
    pub fn parse_request_cookies(&mut self, cookie_header: &str) {
        if let Ok(mut cookies) = self.request_cookies.write() {
            for pair in cookie_header.split(';') {
                let pair = pair.trim();
                if let Some((key, value)) = pair.split_once('=') {
                    cookies.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
    }

    /// 获取请求中的cookie值
    pub fn get_cookie(&self, name: &str) -> Option<String> {
        let cookies = self.request_cookies.read().ok()?;
        cookies.get(name).cloned()
    }

    /// 设置响应cookie（简单版本，仅设置名称和值）
    pub fn set_cookie(&self, name: &str, value: &str) {
        if let Ok(mut cookies) = self.response_cookies.write() {
            cookies.push(CookieBuilder::new(name, value));
        }
    }

    /// 设置响应cookie（完整配置版本）
    pub fn set_cookie_with_builder(&self, cookie: CookieBuilder) {
        if let Ok(mut cookies) = self.response_cookies.write() {
            cookies.push(cookie);
        }
    }

    /// 移除响应cookie（设置过期时间为过去）
    pub fn remove_cookie(&self, name: &str) {
        if let Ok(mut cookies) = self.response_cookies.write() {
            cookies.push(CookieBuilder::new(name, "").expires(0));
        }
    }

    /// 移除响应cookie（带域名配置）
    pub fn remove_cookie_with_domain(&self, name: &str, domain: &str) {
        if let Ok(mut cookies) = self.response_cookies.write() {
            cookies.push(CookieBuilder::new(name, "").domain(domain).expires(0));
        }
    }

    /// 将所有待设置的cookies应用到HttpResponse
    pub fn apply_cookies(&self, response: &mut HttpResponse) {
        if let Ok(cookies) = self.response_cookies.read() {
            for cookie in cookies.iter() {
                let cookie_str = if cookie.is_delete_cookie() {
                    // 移除cookie
                    cookie.to_delete_cookie_string()
                } else {
                    // 设置cookie
                    cookie.to_set_cookie_string()
                };
                response.append_header(Cow::Borrowed("Set-Cookie"), Cow::Owned(cookie_str));
            }
        }
    }

    /// 获取值(需要类型实现Clone)
    pub fn get<T: Any + Send + Sync + Clone + 'static>(&self, name: &str) -> Option<T> {
        let key = (name.to_string(), TypeId::of::<T>());
        let data = self.data.read().ok()?;
        let value = data.get(&key).and_then(|boxed| boxed.downcast_ref::<T>())?;
        Some(value.clone())
    }

    /// 获取值并应用函数
    pub fn with_get<T: Any + Send + Sync + 'static, F, R>(&self, name: &str, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        let key = (name.to_string(), TypeId::of::<T>());
        let data = self.data.read().ok()?;
        let value = data.get(&key).and_then(|boxed| boxed.downcast_ref::<T>())?;
        Some(f(value))
    }

    /// 获取可变引用并应用函数
    pub fn with_mut<T: Any + Send + Sync + 'static, F, R>(&self, name: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        let key = (name.to_string(), TypeId::of::<T>());
        let mut data = self.data.write().ok()?;
        let value = data
            .get_mut(&key)
            .and_then(|boxed| boxed.downcast_mut::<T>())?;
        Some(f(value))
    }

    /// 插入或更新值
    pub fn set<T: Any + Send + Sync + 'static>(&self, name: &str, value: T) {
        let key = (name.to_string(), TypeId::of::<T>());
        if let Ok(mut data) = self.data.write() {
            data.insert(key, Box::new(value));
        }
    }

    /// 移除并返回值
    pub fn remove<T: Any + Send + Sync + 'static>(&self, name: &str) -> Option<T> {
        let key = (name.to_string(), TypeId::of::<T>());
        let mut data = self.data.write().ok()?;
        data.remove(&key)
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    // ==================== 静态方法：Session管理 ====================

    /// 设置JWT签发秘钥（与ServerConfig共享同一个密钥）
    pub async fn set_jwt_secret(secret: &[u8]) {
        let secret_str = String::from_utf8_lossy(secret);
        crate::ServerConfig::set_jwt_secret(secret_str).await;
    }

    /// 获取JWT签发秘钥
    async fn get_jwt_secret() -> anyhow::Result<Vec<u8>> {
        Ok(crate::ServerConfig::jwt_secret().await?.into_bytes())
    }

    /// 签发JWT token
    /// 参数:
    /// - user_id: 用户ID
    /// - ttl: token有效期
    ///
    /// 返回: JWT token字符串
    pub async fn generate_token(
        user_id: i64,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize)]
        struct SessionClaims {
            sub: i64,        // user_id
            exp: usize,      // token过期时间戳
            iat: usize,      // 签发时间戳
            sess_exp: usize, // session过期时间戳（固定，不随访问更新）
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as usize;

        let claims = SessionClaims {
            sub: user_id,
            exp: now + ttl.as_secs() as usize,
            iat: now,
            sess_exp: now + ttl.as_secs() as usize,
        };

        let secret = Self::get_jwt_secret()
            .await
            .map_err(|e| SessionCacheError::InternalError(e.to_string()))?;
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(&secret),
        )?;

        Ok(token)
    }

    /// 解析JWT token
    /// 返回: (user_id, session_exp_duration)
    pub async fn parse_token(token: &str) -> Result<(i64, Duration), SessionCacheError> {
        use jsonwebtoken::{decode, DecodingKey, Validation};
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize)]
        struct SessionClaims {
            sub: i64,
            exp: usize,
            iat: usize,
            sess_exp: usize,
        }

        let secret = Self::get_jwt_secret()
            .await
            .map_err(|e| SessionCacheError::InternalError(e.to_string()))?;
        let token_data = decode::<SessionClaims>(
            token,
            &DecodingKey::from_secret(&secret),
            &Validation::default(),
        )
        .map_err(|e| SessionCacheError::InvalidToken(format!("Token decode failed: {e}")))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .ok_or_else(|| {
                SessionCacheError::InternalError("Failed to get system time".to_string())
            })?
            .as_secs() as usize;

        if token_data.claims.exp < now {
            return Err(SessionCacheError::TokenExpired);
        }

        // 使用固定的session过期时间，不随访问更新
        let session_exp = token_data.claims.sess_exp;
        if session_exp < now {
            return Err(SessionCacheError::SessionExpired);
        }

        let remaining_secs = session_exp - now;
        Ok((
            token_data.claims.sub,
            Duration::from_secs(remaining_secs as u64),
        ))
    }

    // ==================== 内部缓存管理器 ====================

    /// 获取或创建Session缓存
    /// 参数:
    /// - token: JWT token
    ///
    /// 返回: SessionCache实例
    pub async fn from_token(token: &str) -> Result<Self, SessionCacheError> {
        use sha2::{Digest, Sha256};

        let (_, ttl) = Self::parse_token(token).await?;
        let token_key: [u8; 32] = Sha256::digest(token.as_bytes()).into();

        let now = Instant::now();
        let expires_at = now + ttl;

        // 使用 DashMap 的 entry API 一次性完成检查和创建，避免竞态条件
        let mut entry = SESSION_CACHE_MANAGER
            .entry(token_key)
            .or_insert_with(|| (SessionCache::new(), expires_at));

        // 检查是否过期，如果过期则重新创建
        if entry.value().1 < now {
            // 已过期,重新创建
            *entry.value_mut() = (SessionCache::new(), expires_at);
        }

        // 返回session的克隆
        Ok(entry.value().0.clone())
    }

    /// 使指定token对应的session失效（用于登出等场景）
    ///
    /// **Security note:** this only removes the server-side session cache;
    /// the JWT itself remains cryptographically valid until `exp`. Callers
    /// needing hard revocation should use short token TTLs plus a refresh
    /// token flow or maintain a `jti` denylist.
    pub fn invalidate(token: &str) {
        use sha2::{Digest, Sha256};

        let token_key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        SESSION_CACHE_MANAGER.remove(&token_key);
    }

    /// 内部方法:清理过期的session缓存（已废弃，使用后台清理任务替代）
    #[allow(dead_code)]
    fn cleanup_expired_internal() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

        // 每100次调用触发一次清理
        if CALL_COUNTER.fetch_add(1, Ordering::Relaxed).is_multiple_of(100) {
            let now = Instant::now();
            SESSION_CACHE_MANAGER.retain(|_, (_, expires_at)| *expires_at > now);
        }
    }

    /// 内部方法:提供对session manager的访问，用于后台清理任务
    /// 注意：这是一个完全私有的内部方法，仅供服务器清理任务使用
    pub(crate) fn cleanup_expired_sessions() {
        let now = Instant::now();
        SESSION_CACHE_MANAGER.retain(|_, (_, expires_at)| *expires_at > now);
    }
}

/// HTTP conditional preflight result
#[derive(Debug, PartialEq)]
pub enum PreflightResult {
    /// Pass preflight check, can continue processing
    Proceed,
    /// Return 304 Not Modified
    NotModified,
    /// Return 412 Precondition Failed
    PreconditionFailed,
}

fn parse_declared_trailer_names(raw: Option<&str>) -> HashSet<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect::<HashSet<_>>()
    })
    .unwrap_or_default()
}

fn is_forbidden_trailer_field(name: &str) -> bool {
    // RFC 9110/9112: trailers must not carry framing or hop-by-hop control fields.
    matches!(
        name,
        "transfer-encoding"
            | "content-length"
            | "trailer"
            | "host"
            | "connection"
            | "keep-alive"
            | "te"
            | "upgrade"
            | "proxy-authenticate"
            | "proxy-authorization"
    )
}

fn parse_trailer_line(line: &[u8]) -> anyhow::Result<(String, String)> {
    let line = str::from_utf8(line)?.trim();
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid trailer field line"))?;
    let name = name.trim();
    if name.is_empty() {
        Err(anyhow!("empty trailer field name"))?;
    }
    Ok((name.to_string(), value.trim().to_string()))
}

/// Strip a wrapping pair of double quotes (`"value"` -> `value`).
///
/// Malformed or empty quoted tokens are returned as-is instead of slicing,
/// so hostile multipart headers can never panic the parser.
fn strip_quoted(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}

/// Find the first offset of `needle` inside `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|part| part == needle)
}

/// Split a multipart body on its boundary delimiters.
///
/// Per RFC 2046 a delimiter only takes effect at the start of a line: an
/// occurrence of `--boundary` that is not at position 0 and not preceded by
/// CRLF is ordinary content, not a separator.
fn split_multipart_at_boundary<'a>(data: &'a [u8], boundary: &[u8]) -> Vec<&'a [u8]> {
    let mut positions = Vec::new();
    let mut i = 0usize;
    while i + boundary.len() <= data.len() {
        if &data[i..i + boundary.len()] == boundary
            && (i == 0 || (i >= 2 && &data[i - 2..i] == b"\r\n"))
        {
            positions.push(i);
            i += boundary.len();
        } else {
            i += 1;
        }
    }
    if positions.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::with_capacity(positions.len() + 1);
    // preamble (usually empty)
    parts.push(&data[..positions[0]]);
    for pair in positions.windows(2) {
        parts.push(&data[pair[0] + boundary.len()..pair[1]]);
    }
    let last = *positions.last().expect("positions is non-empty");
    parts.push(&data[last + boundary.len()..]);
    parts
}

/// Reject header values carrying CR/LF/NUL to prevent response splitting.
/// Returns a sanitized copy only when the input actually contains control bytes.
pub(crate) fn sanitize_header_str(value: &str) -> std::borrow::Cow<'_, str> {
    if value.as_bytes().iter().any(|&b| b == b'\r' || b == b'\n' || b == 0) {
        std::borrow::Cow::Owned(
            value
                .chars()
                .filter(|&c| c != '\r' && c != '\n' && c != '\0')
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn parse_transfer_encoding_tokens(raw: &str) -> anyhow::Result<Vec<String>> {
    let codings = raw
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if codings.is_empty() {
        Err(anyhow!("empty Transfer-Encoding header"))?;
    }
    Ok(codings)
}

/// HTTP date parsing error
#[derive(Debug)]
pub struct HttpDateParseError;

impl std::fmt::Display for HttpDateParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to parse HTTP date")
    }
}

impl std::error::Error for HttpDateParseError {}

/// Parse HTTP date format to Unix timestamp
/// Supports RFC 7231 standard HTTP date formats:
/// - RFC 1123: "Mon, 06 Nov 1994 08:49:37 GMT"
/// - RFC 850: "Monday, 06-Nov-94 08:49:37 GMT"
/// - ANSI C asctime(): "Mon Nov  6 08:49:37 1994"
pub fn parse_http_date(date_str: &str) -> Result<u64, HttpDateParseError> {
    // Use simple manual parsing method to handle RFC 1123 format
    // Format: "Fri, 12 Sep 2025 00:00:00 GMT"
    static DATE_REGEX: std::sync::LazyLock<Result<regex::Regex, regex::Error>> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^\w+, (\d{1,2}) (\w+) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$")
        });

    if let Some(caps) = DATE_REGEX
        .as_ref()
        .ok()
        .and_then(|re| re.captures(date_str))
    {
        let day: u32 = caps[1].parse().map_err(|_| HttpDateParseError)?;
        let month_str = &caps[2];
        let year: i32 = caps[3].parse().map_err(|_| HttpDateParseError)?;
        let hour: u32 = caps[4].parse().map_err(|_| HttpDateParseError)?;
        let minute: u32 = caps[5].parse().map_err(|_| HttpDateParseError)?;
        let second: u32 = caps[6].parse().map_err(|_| HttpDateParseError)?;

        let month = match month_str {
            "Jan" => 1,
            "Feb" => 2,
            "Mar" => 3,
            "Apr" => 4,
            "May" => 5,
            "Jun" => 6,
            "Jul" => 7,
            "Aug" => 8,
            "Sep" => 9,
            "Oct" => 10,
            "Nov" => 11,
            "Dec" => 12,
            _ => return Err(HttpDateParseError),
        };

        if let Some(dt) = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|d| d.and_hms_opt(hour, minute, second))
        {
            let timestamp = dt.and_utc().timestamp() as u64;
            return Ok(timestamp);
        }
    }

    // Try RFC 1123 format
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str, "%a, %d %b %Y %H:%M:%S GMT") {
        let timestamp = dt.timestamp() as u64;
        return Ok(timestamp);
    }

    // Try RFC 850 format
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str, "%A, %d-%b-%y %H:%M:%S GMT") {
        let timestamp = dt.timestamp() as u64;
        return Ok(timestamp);
    }

    // Try ANSI C asctime() format
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%a %b %e %H:%M:%S %Y") {
        let timestamp = dt.and_utc().timestamp() as u64;
        return Ok(timestamp);
    }

    Err(HttpDateParseError)
}

static SERVER_STR: LazyLock<String> =
    LazyLock::new(|| format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));

type AsyncHttpHandler =
    fn(&mut HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>;
type SyncHttpHandler = fn(&mut HttpRequest) -> HttpResponse;

#[derive(Clone, Copy)]
pub enum HttpHandler {
    Async(AsyncHttpHandler),
    Sync(SyncHttpHandler),
}

pub struct RequestHandlerFlagDoc {
    pub show: bool,
    pub auth: bool,
    pub summary: &'static str,
    pub desp: &'static str,
    pub args: &'static str,
    pub tag: &'static str, // Controller 名称，用于 Swagger 分组
}

impl RequestHandlerFlagDoc {
    pub const fn new(
        show: bool,
        auth: bool,
        summary: &'static str,
        desp: &'static str,
        args: &'static str,
        tag: &'static str,
    ) -> Self {
        RequestHandlerFlagDoc {
            show,
            auth,
            summary,
            desp,
            args,
            tag,
        }
    }
}

pub struct RequestHandlerFlag {
    pub method: HttpMethod,
    pub path: &'static str,
    pub handler: HttpHandler,
    pub doc: RequestHandlerFlagDoc,
}

impl RequestHandlerFlag {
    pub const fn new(
        method: HttpMethod,
        path: &'static str,
        handler: HttpHandler,
        doc: RequestHandlerFlagDoc,
    ) -> Self {
        RequestHandlerFlag {
            method,
            path,
            handler,
            doc,
        }
    }
}

inventory::collect!(RequestHandlerFlag);

/// 异步错误处理器类型
pub type AsyncErrorHandler =
    fn(&mut HttpRequest, anyhow::Error) -> Pin<Box<dyn Future<Output = HttpResponse> + Send + '_>>;

/// 同步错误处理器类型
pub type SyncErrorHandler = fn(&mut HttpRequest, anyhow::Error) -> HttpResponse;

/// 错误处理器枚举，支持异步和同步
#[derive(Clone)]
pub enum ErrorHandler {
    Async(AsyncErrorHandler),
    Sync(SyncErrorHandler),
}

/// 错误处理器注册标志
pub struct ErrorHandlerFlag {
    pub handler: ErrorHandler,
}

impl ErrorHandlerFlag {
    pub const fn new(handler: ErrorHandler) -> Self {
        ErrorHandlerFlag { handler }
    }
}

inventory::collect!(ErrorHandlerFlag);

/// Controller 结构体字段信息
pub struct ControllerStructFieldInfo {
    pub has_once_cache: bool,
    pub has_session_cache: bool,
}

/// Controller 结构体标志，用于在 impl 宏中传递字段信息
pub struct ControllerStructFlag {
    pub struct_name: &'static str,
    pub field_info: ControllerStructFieldInfo,
}

impl ControllerStructFlag {
    pub const fn new(struct_name: &'static str, field_info: ControllerStructFieldInfo) -> Self {
        ControllerStructFlag {
            struct_name,
            field_info,
        }
    }
}

inventory::collect!(ControllerStructFlag);

#[derive(Clone, Copy, Debug, Display, Eq, Hash, PartialEq)]
pub enum HttpMethod {
    GET,
    PUT,
    COPY,
    HEAD,
    LOCK,
    MOVE,
    POST,
    MKCOL,
    PATCH,
    TRACE,
    DELETE,
    UNLOCK,
    CONNECT,
    OPTIONS,
    PROPFIND,
    PROPPATCH,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpRequestTargetForm {
    Origin,
    Absolute,
    Authority,
    Asterisk,
}

#[derive(Debug, Eq, PartialEq)]
pub enum CompressMode {
    None,
    Gzip,
}

pub struct Websocket {
    stream: Arc<Mutex<HttpStream>>,
    /// 内部读缓冲（半帧安全垫）：空闲 ping 的 timeout 掐掉 recv future 时，
    /// 已从内核缓冲读出的帧字节留存于此，下次 recv 从断点继续（见
    /// `read_exact_buffered`）。仅读向使用；clone 出的写向句柄恒空。
    rbuf: Vec<u8>,
    /// true when this endpoint dialled out (client role), false for upgraded
    /// server-side connections. Drives RFC 6455 masking direction checks.
    is_client: bool,
}

impl Websocket {
    /// 克隆一个共享同一底层流的新句柄（服务端读写拆分任务场景）：读端在
    /// 独立任务里整帧循环 `recv`（不被取消、天然帧对齐安全），写端在另一
    /// 任务里逐帧 `send_*`；内部 stream 互斥锁按完整帧粒度串行化两个方向
    /// 的 I/O，TCP 全双工下互不干扰。读缓冲按「clone 后只有读向句柄 recv」
    /// 的约定拷贝给新句柄（断点续读；写向句柄的拷贝永不消费，无害）。
    pub fn clone_handle(&self) -> Self {
        Self {
            stream: Arc::clone(&self.stream),
            rbuf: self.rbuf.clone(),
            is_client: self.is_client,
        }
    }

    pub async fn connect(url: &str, args: Vec<Headers>) -> anyhow::Result<Self> {
        let mut sess = Session::new();
        let mut req = sess.new_request(HttpMethod::GET, url).await?;
        for arg in args.into_iter() {
            req.apply_header(arg);
        }
        req.apply_header(Headers::Connection("Upgrade".to_string()));
        req.apply_header(Headers::Upgrade("Websocket".to_string()));
        req.apply_header(Headers::Sec_WebSocket_Version("13".to_string()));
        // RFC 6455 要求 Sec-WebSocket-Key 为 16 字节随机数据的 base64 编码
        let random_bytes: [u8; 16] = rand::random();
        let key = base64::engine::general_purpose::STANDARD.encode(random_bytes);
        req.apply_header(Headers::Sec_WebSocket_Key(key));
        let res = sess.do_request(req).await?;
        if res.http_code != 101 {
            let body_str = match &res.body {
                HttpResponseBody::Data(data) => str::from_utf8(&data[..])?,
                HttpResponseBody::Stream(_) => "stream response",
            };
            Err(anyhow!("Server return code[{}]: {body_str}", res.http_code))?;
        }
        let stream = sess
            .sess_impl
            .ok_or_else(|| anyhow!("session impl is null"))?
            .stream;
        let stream = Arc::new(Mutex::new(stream));
        Ok(Self {
            stream,
            rbuf: Vec::new(),
            is_client: true,
        })
    }

    /// 从内部读缓冲取 n 字节（不足时从底层流补读）。帧字节一律先落入
    /// rbuf 再消费：空闲 ping 的 `timeout(ping_duration, recv_impl())` 掐
    /// 掉 recv future 时，已从内核缓冲读出的半帧字节安全留在 rbuf（rbuf
    /// 挂在 self 上，future drop 不丢），下次 recv 从断点继续——无缓冲直读
    /// 形态下这个竞态会把部分帧永久丢出字节流，后续解析整体错位（表现为
    /// 好端端的连接被误判帧错误/EOF 而关闭）。
    async fn read_exact_buffered(
        rbuf: &mut Vec<u8>,
        stream: &mut HttpStream,
        n: usize,
    ) -> anyhow::Result<Vec<u8>> {
        while rbuf.len() < n {
            let mut tmp = [0u8; 8192];
            let nread = stream.read(&mut tmp).await?;
            if nread == 0 {
                anyhow::bail!("connection closed while reading websocket frame");
            }
            rbuf.extend_from_slice(&tmp[..nread]);
        }
        Ok(rbuf.drain(..n).collect())
    }

    /// 仅向 rbuf 补充字节至 `n`（不消费）：空闲等待阶段专用。超时取消安全
    /// ——已读字节留存 rbuf，重试即续等（调用方在拿到 `n` 字节前绝不开始
    /// 帧解析，见 `recv` 两阶段结构）。
    async fn fill_buffer(
        rbuf: &mut Vec<u8>,
        stream: &mut HttpStream,
        n: usize,
    ) -> anyhow::Result<()> {
        while rbuf.len() < n {
            let mut tmp = [0u8; 8192];
            let nread = stream.read(&mut tmp).await?;
            if nread == 0 {
                anyhow::bail!("connection closed while reading websocket frame");
            }
            rbuf.extend_from_slice(&tmp[..nread]);
        }
        Ok(())
    }

    async fn recv_impl(&mut self) -> anyhow::Result<WsFrameImpl> {
        let mut stream = self.stream.lock().await;
        let buf = {
            let mut buf = [0u8; 2];
            buf.copy_from_slice(
                &Self::read_exact_buffered(&mut self.rbuf, &mut stream, 2).await?,
            );
            buf
        };
        //let fin = buf[0] & 0b1000_0000 != 0;
        let opcode = buf[0] & 0b0000_1111;
        let payload_len = {
            let payload_len = buf[1] & 0b0111_1111;
            match payload_len {
                126 => {
                    let ext = Self::read_exact_buffered(&mut self.rbuf, &mut stream, 2).await?;
                    u16::from_be_bytes([ext[0], ext[1]]) as usize
                }
                127 => {
                    let ext = Self::read_exact_buffered(&mut self.rbuf, &mut stream, 8).await?;
                    let declared = u64::from_be_bytes([
                        ext[0], ext[1], ext[2], ext[3], ext[4], ext[5], ext[6], ext[7],
                    ]);
                    // Never trust the declared 64-bit length: allocating it
                    // blindly lets a remote peer OOM the process.
                    usize::try_from(declared).map_err(|_| {
                        anyhow::Error::msg("WebSocket frame length exceeds address space")
                    })?
                }
                _ => payload_len as usize,
            }
        };
        let max_frame = ServerConfig::get_ws_max_frame_size();
        if payload_len > max_frame {
            anyhow::bail!(
                "WebSocket frame too large: {payload_len} bytes exceeds {max_frame} bytes"
            );
        }
        // RFC 6455 §5.1: client frames MUST be masked, server frames MUST NOT be.
        let masked = buf[1] & 0b1000_0000 != 0;
        if masked == self.is_client {
            anyhow::bail!("WebSocket frame masking violates RFC 6455 direction rules");
        }
        let omask_key = match masked {
            true => {
                let mk = Self::read_exact_buffered(&mut self.rbuf, &mut stream, 4).await?;
                Some([mk[0], mk[1], mk[2], mk[3]])
            }
            false => None,
        };
        let mut payload =
            Self::read_exact_buffered(&mut self.rbuf, &mut stream, payload_len).await?;
        if let Some(mask_key) = omask_key {
            for i in 0..payload.len() {
                payload[i] ^= mask_key[i % 4];
            }
        }
        match opcode {
            0x0 => Ok(WsFrameImpl::PartData(payload)),
            0x1 => Ok(WsFrameImpl::Text(payload)),
            0x2 => Ok(WsFrameImpl::Binary(payload)),
            0x8 => Ok(WsFrameImpl::Close),
            0x9 => Ok(WsFrameImpl::Ping),
            0xA => Ok(WsFrameImpl::Pong),
            _ => Err(anyhow::Error::msg("unsupported opcode")),
        }
    }

    pub async fn recv(&mut self) -> anyhow::Result<WsFrame> {
        let mut tmp = vec![];
        let mut ping_failures = 0u32;
        let max_message = ServerConfig::get_ws_max_message_size();
        loop {
            let timeout = ServerConfig::get_ws_ping_duration().await;
            // 两阶段结构（空闲 ping 超时与帧解析解耦）：空闲 ping 的本意是
            // 探测「无字节到达」的死链，因此超时只允许覆盖**帧首字节到达
            // 之前**的等待（阶段一）；帧头一旦进入 rbuf，帧解析（阶段二）
            // 必须一气呵成——timeout 掐在帧中段会把解析进度连同 future 一
            // 起丢弃，字节虽经 rbuf 保留，下一次调用却会从当前字节位重新
            // 按帧头解析，字节流整体错位（表现为好端端的连接被误判
            // masking violation / unsupported opcode 而关闭；空闲 ping 越
            // 短〔如 100ms 写延迟上界形态〕命中概率越高，实测复现）。
            // 阶段一（可取消）：等待帧头 ≥2 字节进入 rbuf；取消即续等，
            // 已读字节留 rbuf 不丢。
            let header_ready = {
                let mut stream = self.stream.lock().await;
                match tokio::time::timeout(
                    timeout,
                    Self::fill_buffer(&mut self.rbuf, &mut stream, 2),
                )
                .await
                {
                    Ok(Ok(())) => true,
                    Ok(Err(err)) => return Err(err),
                    Err(_) => false,
                }
            };
            if !header_ready {
                // 发送 ping 探测连接状态，带超时保护。
                // 只有收到 Pong 才会清零计数（见下方 Pong 分支）；
                // 发送成功不代表对端存活，不能重置失败计数。
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    self.send_impl(WsFrameImpl::Ping),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        ping_failures += 1;
                        if ping_failures >= 3 {
                            return Err(anyhow::Error::msg(format!(
                                "WebSocket ping failed {} times, last error: {}",
                                ping_failures, e
                            )));
                        }
                    }
                    Err(_) => {
                        ping_failures += 1;
                        if ping_failures >= 3 {
                            return Err(anyhow::Error::msg(
                                "WebSocket ping timed out 3 times, connection appears dead",
                            ));
                        }
                    }
                }
                continue;
            }
            // 阶段二（不可被空闲计时器打断）：帧头已在 rbuf，完整解析该帧。
            // 对端既有能力发出帧头即有能力发完帧（TCP 保证或错）；真死链
            // 由下一轮空闲间隙的 ping 失败计数兜底。调用方若在此前 drop 整
            // 个 recv future（连接关闭语义）则无所谓进度。
            match self.recv_impl().await? {
                WsFrameImpl::Close => return Err(anyhow::Error::msg("close frame")),
                WsFrameImpl::Ping => self.send_impl(WsFrameImpl::Pong).await?,
                WsFrameImpl::Pong => {
                    ping_failures = 0;
                }
                WsFrameImpl::Binary(data) => {
                    tmp.extend(data);
                    if tmp.len() > max_message {
                        anyhow::bail!("WebSocket message too large: exceeds {max_message} bytes");
                    }
                    return Ok(WsFrame::Binary(tmp));
                }
                WsFrameImpl::Text(data) => {
                    tmp.extend(data);
                    if tmp.len() > max_message {
                        anyhow::bail!("WebSocket message too large: exceeds {max_message} bytes");
                    }
                    let ret_str = String::from_utf8(tmp).unwrap_or("".to_string());
                    return Ok(WsFrame::Text(ret_str));
                }
                WsFrameImpl::PartData(data) => {
                    tmp.extend(data);
                    if tmp.len() > max_message {
                        anyhow::bail!("WebSocket message too large: exceeds {max_message} bytes");
                    }
                }
            }
        }
    }

    async fn send_impl(&mut self, frame: WsFrameImpl) -> anyhow::Result<()> {
        let (fin, opcode, payload) = match frame {
            WsFrameImpl::Close => (true, 0x8, vec![]),
            WsFrameImpl::Ping => (true, 0x9, vec![]),
            WsFrameImpl::Pong => (true, 0xA, vec![]),
            WsFrameImpl::Binary(data) => (true, 0x2, data),
            WsFrameImpl::Text(data) => (true, 0x1, data),
            WsFrameImpl::PartData(data) => (false, 0x0, data),
        };
        let payload_len = payload.len();
        // RFC 6455 §5.1: client-to-server frames MUST be masked, server-to-
        // client frames MUST NOT be. Mirror the recv direction check.
        let mask_bit = if self.is_client { 0x80 } else { 0x00 };
        let mut buf = vec![];
        buf.push((fin as u8) << 7 | opcode);
        if payload_len < 126 {
            buf.push((payload_len as u8) | mask_bit);
        } else if payload_len < 65536 {
            buf.push(126 | mask_bit);
            buf.extend((payload_len as u16).to_be_bytes().iter());
        } else {
            buf.push(127 | mask_bit);
            buf.extend((payload_len as u64).to_be_bytes().iter());
        }
        let payload_to_send = if self.is_client {
            let mask_key: [u8; 4] = rand::random();
            buf.extend_from_slice(&mask_key);
            let mut masked_payload = payload.clone();
            for i in 0..masked_payload.len() {
                masked_payload[i] ^= mask_key[i % 4];
            }
            masked_payload
        } else {
            payload
        };
        let mut stream = self.stream.lock().await;
        stream.write_all(&buf).await?;
        stream.write_all(&payload_to_send).await?;
        Ok(())
    }

    pub async fn send_ping(&mut self) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Ping).await
    }

    pub async fn send(&mut self, frame: WsFrame) -> anyhow::Result<()> {
        match frame {
            WsFrame::Binary(data) => self.send_impl(WsFrameImpl::Binary(data)).await,
            WsFrame::Text(text) => {
                self.send_impl(WsFrameImpl::Text(text.as_bytes().to_vec()))
                    .await
            }
        }
    }

    pub async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Binary(data)).await
    }

    pub async fn send_text(&mut self, data: &str) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Text(data.as_bytes().to_vec()))
            .await
    }

    pub async fn send_close(&mut self) -> anyhow::Result<()> {
        self.send_impl(WsFrameImpl::Close).await
    }
}

#[derive(Debug)]
pub enum WsFrame {
    Binary(Vec<u8>),
    Text(String),
}

pub enum WsFrameImpl {
    Close,
    Ping,
    Pong,
    Binary(Vec<u8>),
    Text(Vec<u8>),
    PartData(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct PostFile {
    pub filename: LocalHipStr<'static>,
    pub data: LocalHipByt<'static>,
}

unsafe impl Send for PostFile {}

pub struct HttpRequest {
    pub method: HttpMethod,
    pub target_form: HttpRequestTargetForm,
    pub url_path: LocalHipStr<'static>,
    pub url_query: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    pub version: u8,
    pub headers: FxHashMap<HeaderOrHipStr, LocalHipStr<'static>>,
    pub trailers: FxHashMap<HeaderOrHipStr, LocalHipStr<'static>>,
    pub body: LocalHipByt<'static>,
    pub body_pairs: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    pub body_files: HashMap<LocalHipStr<'static>, PostFile>,
    pub client_addr: Option<SocketAddr>,
    pub exts: HashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>,
    stream: Option<Arc<Mutex<HttpStream>>>,
}

unsafe impl Send for HttpRequest {}
unsafe impl Sync for HttpRequest {}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("target_form", &self.target_form)
            .field("url_path", &self.url_path)
            .field("url_query", &self.url_query)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .field("body", &self.body)
            .field("body_pairs", &self.body_pairs)
            .field("body_files", &self.body_files)
            .field("client_addr", &self.client_addr)
            .field("exts", &self.exts)
            .finish()
    }
}

impl Default for HttpRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRequest {
    fn ensure_header_section_size(buf: &[u8], max_header_bytes: usize) -> anyhow::Result<()> {
        let header_len = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(pos) => pos + 4,
            None => buf.len(),
        };
        if header_len > max_header_bytes {
            anyhow::bail!(
                "request headers too large: {header_len} bytes exceeds {max_header_bytes} bytes"
            );
        }
        Ok(())
    }

    async fn process_expect_header(
        &self,
        stream: &mut HttpStream,
        has_request_body: bool,
    ) -> anyhow::Result<()> {
        let Some(expect) = self.get_header_key(HeaderItem::Expect) else {
            return Ok(());
        };
        if self.version < 11 {
            anyhow::bail!("Expect header is not supported for HTTP versions below 1.1");
        }
        let mut has_100_continue = false;
        for token in expect.split(',').map(|token| token.trim()) {
            if token.is_empty() {
                anyhow::bail!("empty Expect header");
            }
            if token.eq_ignore_ascii_case("100-continue") {
                has_100_continue = true;
                continue;
            }
            anyhow::bail!("unsupported Expect header: {token}");
        }
        if has_100_continue && has_request_body {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self::with_header_capacity(8)
    }

    fn clear_for_reuse(&mut self) {
        self.method = HttpMethod::GET;
        self.target_form = HttpRequestTargetForm::Origin;
        self.url_path = "".into();
        self.url_query.clear();
        self.version = 11;
        self.headers.clear();
        self.trailers.clear();
        self.body = LocalHipByt::new();
        self.body_pairs.clear();
        self.body_files.clear();
        self.client_addr = None;
        self.exts.clear();
        self.stream = None;
    }

    fn with_header_capacity(header_capacity: usize) -> Self {
        Self {
            method: HttpMethod::GET,
            target_form: HttpRequestTargetForm::Origin,
            url_path: LocalHipStr::from("/"),
            url_query: HashMap::new(),
            version: 11,
            headers: FxHashMap::with_capacity_and_hasher(header_capacity, Default::default()),
            trailers: FxHashMap::default(),
            body: LocalHipByt::new(),
            body_pairs: HashMap::new(),
            body_files: HashMap::new(),
            client_addr: None,
            exts: HashMap::new(),
            stream: None,
        }
    }

    pub fn query_string(&self) -> String {
        let mut q = "?".to_string();
        for (k, v) in self.url_query.iter() {
            q.push_str(&k.url_encode());
            q.push('=');
            q.push_str(&v.url_encode());
            q.push('&');
        }
        q.pop();
        q
    }

    fn get_ext<T: Any + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.exts
            .get(&TypeId::of::<T>())
            .and_then(|any| any.clone().downcast().ok())
    }

    fn parse_path_and_query(&mut self, target: &str) {
        self.url_query.clear();
        match target.find('?') {
            Some(p) => {
                self.url_path = LocalHipStr::from(&target[..p]);
                self.url_query = target[p + 1..]
                    .split('&')
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| {
                        (
                            LocalHipStr::from(a.url_decode()),
                            LocalHipStr::from(b.url_decode()),
                        )
                    })
                    .collect();
            }
            None => {
                self.url_path = LocalHipStr::from(target);
            }
        }
    }

    fn request_target(&self) -> String {
        if matches!(
            self.target_form,
            HttpRequestTargetForm::Asterisk | HttpRequestTargetForm::Authority
        ) {
            return self.url_path.to_string();
        }
        let mut target = self.url_path.to_string();
        if !self.url_query.is_empty() {
            target.push_str(&self.query_string());
        }
        target
    }

    fn request_version_line(&self) -> String {
        match self.version {
            10 => "HTTP/1.0".to_string(),
            11 => "HTTP/1.1".to_string(),
            // Request-line serialization is HTTP/1.x only. Non-H1 internal markers
            // (e.g. 20/30 for H2/H3) must not leak into an HTTP/1 request line.
            _ => "HTTP/1.1".to_string(),
        }
    }

    fn parse_request_target(&mut self, target: &str) -> anyhow::Result<()> {
        if target == "*" {
            if self.method != HttpMethod::OPTIONS {
                anyhow::bail!("asterisk-form request-target requires OPTIONS");
            }
            self.target_form = HttpRequestTargetForm::Asterisk;
            self.url_query.clear();
            self.url_path = LocalHipStr::from("*");
            return Ok(());
        }

        if target.starts_with('/') {
            if self.method == HttpMethod::CONNECT {
                anyhow::bail!("CONNECT requires authority-form request-target");
            }
            self.target_form = HttpRequestTargetForm::Origin;
            self.parse_path_and_query(target);
            return Ok(());
        }

        if target.contains("://") {
            if self.method == HttpMethod::CONNECT {
                anyhow::bail!("CONNECT requires authority-form, absolute-form is invalid");
            }
            let uri = target
                .parse::<Uri>()
                .map_err(|_| anyhow!("invalid absolute-form request-target"))?;
            if uri.scheme().is_none() || uri.authority().is_none() {
                anyhow::bail!("absolute-form request-target must include scheme and authority");
            }
            self.target_form = HttpRequestTargetForm::Absolute;
            let path_and_query = uri.path_and_query().map(|v| v.as_str()).unwrap_or("/");
            self.parse_path_and_query(path_and_query);
            return Ok(());
        }

        if http::uri::Authority::from_str(target).is_ok() {
            if self.method != HttpMethod::CONNECT {
                anyhow::bail!("authority-form request-target is only valid for CONNECT");
            }
            self.target_form = HttpRequestTargetForm::Authority;
            self.url_query.clear();
            self.url_path = LocalHipStr::from(target);
            return Ok(());
        }

        anyhow::bail!("unsupported request-target form")
    }

    pub fn get_uri(&self, is_https: bool) -> anyhow::Result<http::Uri> {
        let mut q = self.url_path.to_string();
        q.push_str(&self.query_string());
        Ok(http::uri::Builder::new()
            .scheme(if is_https { "https" } else { "http" })
            .path_and_query(q)
            .build()?)
    }

    pub fn from_url(url: &str, method: HttpMethod) -> anyhow::Result<(Self, bool, u16)> {
        let uri = url.parse::<Uri>()?;
        let mut req = Self::new();
        req.method = method;
        req.target_form = HttpRequestTargetForm::Origin;
        req.parse_path_and_query(uri.path_and_query().map(|v| v.as_str()).unwrap_or("/"));
        req.headers.insert(
            HeaderOrHipStr::from_str("Host"),
            uri.authority()
                .map(|authority| authority.as_str())
                .unwrap_or("localhost")
                .into(),
        );
        let use_ssl = uri.scheme() == Some(&Scheme::HTTPS);
        let port = uri.port_u16().unwrap_or(if use_ssl { 443 } else { 80 });
        Ok((req, use_ssl, port))
    }

    pub fn set_header(
        &mut self,
        key: impl Into<HeaderOrHipStr>,
        value: impl Into<LocalHipStr<'static>>,
    ) {
        self.headers.insert(key.into(), value.into());
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        if let Some(header_item) = HeaderItem::try_from_str(key) {
            if let Some(value) = self.headers.get(&HeaderOrHipStr::HeaderItem(header_item)) {
                return Some(&value[..]);
            }
        }
        self.headers
            .get(&HeaderOrHipStr::HipStr(LocalHipStr::from(key)))
            .map(|a| &a[..])
    }

    pub fn get_header_key(&self, key: HeaderItem) -> Option<&str> {
        self.headers.get(&key.into()).map(|a| &a[..])
    }

    pub fn set_trailer(
        &mut self,
        key: impl Into<HeaderOrHipStr>,
        value: impl Into<LocalHipStr<'static>>,
    ) {
        self.trailers.insert(key.into(), value.into());
    }

    pub fn get_trailer(&self, key: &str) -> Option<&str> {
        if let Some(header_item) = HeaderItem::try_from_str(key) {
            if let Some(value) = self.trailers.get(&HeaderOrHipStr::HeaderItem(header_item)) {
                return Some(&value[..]);
            }
        }
        self.trailers
            .get(&HeaderOrHipStr::HipStr(LocalHipStr::from(key)))
            .map(|a| &a[..])
    }

    pub fn get_header_accept_encoding(&self) -> CompressMode {
        Self::negotiate_accept_encoding(
            self.get_header_key(HeaderItem::Accept_Encoding)
                .unwrap_or(""),
        )
    }

    fn negotiate_accept_encoding(header: &str) -> CompressMode {
        let mut explicit_gzip_q: Option<u16> = None;
        let mut wildcard_q: Option<u16> = None;

        for item in header.split(',') {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut parts = trimmed.split(';');
            let coding = parts.next().unwrap_or("").trim().to_ascii_lowercase();
            if coding.is_empty() {
                continue;
            }

            let mut quality = 1000u16;
            let mut malformed_q = false;
            for param in parts {
                let param = param.trim();
                if param.is_empty() {
                    continue;
                }
                let mut key_val = param.splitn(2, '=');
                let key = key_val.next().unwrap_or("").trim().to_ascii_lowercase();
                if key != "q" {
                    continue;
                }
                let val = key_val.next().unwrap_or("").trim();
                if let Some(parsed_q) = Self::parse_qvalue_thousandths(val) {
                    quality = parsed_q;
                } else {
                    malformed_q = true;
                }
                break;
            }

            if malformed_q {
                continue;
            }

            match coding.as_str() {
                "gzip" => {
                    explicit_gzip_q =
                        Some(explicit_gzip_q.map_or(quality, |prev| prev.max(quality)));
                }
                "*" => {
                    wildcard_q = Some(wildcard_q.map_or(quality, |prev| prev.max(quality)));
                }
                _ => {}
            }
        }

        let selected_q = explicit_gzip_q.or(wildcard_q).unwrap_or(0);
        if selected_q > 0 {
            CompressMode::Gzip
        } else {
            CompressMode::None
        }
    }

    fn parse_qvalue_thousandths(raw: &str) -> Option<u16> {
        let val = raw.trim();
        if val == "1" || val == "1.0" || val == "1.00" || val == "1.000" {
            return Some(1000);
        }
        if val == "0" {
            return Some(0);
        }
        let frac = val.strip_prefix("0.")?;
        if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let mut digits = frac.to_string();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits.parse::<u16>().ok()
    }

    pub fn get_header_host(&self) -> Option<&str> {
        self.get_header_key(HeaderItem::Host)
    }

    pub fn get_header_connection(&self) -> HttpConnection {
        if let Some(conn) = self.get_header_key(HeaderItem::Connection) {
            HttpConnection::from_str(conn).unwrap_or(HttpConnection::Close)
        } else if self.version >= 11 {
            HttpConnection::KeepAlive
        } else {
            HttpConnection::Close
        }
    }

    pub fn get_header_content_length(&self) -> usize {
        self.get_header_key(HeaderItem::Content_Length)
            .map_or(0, |val| val.parse::<usize>().unwrap_or(0))
    }

    fn parse_header_content_length(&self) -> anyhow::Result<Option<usize>> {
        let Some(raw_val) = self.get_header_key(HeaderItem::Content_Length) else {
            return Ok(None);
        };
        let value = raw_val.trim();
        if value.is_empty() {
            Err(anyhow!("empty Content-Length header"))?
        }
        Ok(Some(value.parse::<usize>()?))
    }

    fn has_chunked_transfer_encoding(&self) -> anyhow::Result<bool> {
        let Some(raw_val) = self.get_header_key(HeaderItem::Transfer_Encoding) else {
            return Ok(false);
        };
        let codings: Vec<String> = raw_val
            .split(',')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect();
        if codings.is_empty() {
            anyhow::bail!("empty Transfer-Encoding header");
        }
        if codings.len() == 1 && codings[0] == "chunked" {
            return Ok(true);
        }
        anyhow::bail!("unsupported Transfer-Encoding: {raw_val}")
    }

    async fn read_chunked_body(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        hdr_len: usize,
        allowed_trailers: &HashSet<String>,
        max_body_bytes: Option<usize>,
    ) -> anyhow::Result<(
        LocalHipByt<'static>,
        FxHashMap<HeaderOrHipStr, LocalHipStr<'static>>,
        usize,
    )> {
        let mut cursor = hdr_len;
        let mut body = Vec::new();
        let mut trailers = FxHashMap::with_capacity_and_hasher(4, Default::default());
        let mut tmp_buf = [0u8; 4096];
        const MAX_METADATA_LINE_BYTES: usize = 16 * 1024;

        loop {
            let line_end = loop {
                if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n") {
                    break cursor + pos;
                }
                if buf.len() - cursor >= MAX_METADATA_LINE_BYTES {
                    anyhow::bail!("chunk metadata line too large");
                }
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            };

            let chunk_size = {
                let size_line = str::from_utf8(&buf[cursor..line_end])?;
                let size_token = size_line
                    .split_once(';')
                    .map_or(size_line, |(size, _)| size)
                    .trim();
                if size_token.is_empty() {
                    Err(anyhow!("invalid chunk size"))?;
                }
                usize::from_str_radix(size_token, 16)?
            };
            cursor = line_end + 2;

            if chunk_size == 0 {
                let trailer_end = loop {
                    let line_start = cursor;
                    let line_end = loop {
                        if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n")
                        {
                            break cursor + pos;
                        }
                        if buf.len() - cursor >= MAX_METADATA_LINE_BYTES {
                            anyhow::bail!("chunk trailer line too large");
                        }
                        let n = stream.read(&mut tmp_buf).await?;
                        if n == 0 {
                            Err(anyhow::Error::msg("connection closed"))?;
                        }
                        buf.extend(&tmp_buf[..n]);
                    };
                    cursor = line_end + 2;
                    if line_end == line_start {
                        break cursor;
                    }

                    let (name, value) = parse_trailer_line(&buf[line_start..line_end])?;
                    let name_lower = name.to_ascii_lowercase();
                    if is_forbidden_trailer_field(&name_lower) {
                        Err(anyhow!("forbidden trailer field: {name}"))?;
                    }
                    if !allowed_trailers.contains(&name_lower) {
                        Err(anyhow!("unexpected trailer field: {name}"))?;
                    }
                    trailers.insert(HeaderOrHipStr::from_str(&name), value.into());
                };
                cursor = trailer_end;
                break;
            }

            if let Some(max_body_bytes) = max_body_bytes {
                if body.len().saturating_add(chunk_size) > max_body_bytes {
                    anyhow::bail!(
                        "request body too large: chunked body exceeds {max_body_bytes} bytes"
                    );
                }
            }

            let Some(chunk_end) = cursor.checked_add(chunk_size).and_then(|v| v.checked_add(2))
            else {
                anyhow::bail!("chunk size overflow");
            };
            while buf.len() < chunk_end {
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            }
            body.extend_from_slice(&buf[cursor..cursor + chunk_size]);
            if &buf[cursor + chunk_size..cursor + chunk_size + 2] != b"\r\n" {
                Err(anyhow!("invalid chunk terminator"))?;
            }
            cursor += chunk_size + 2;
        }

        Ok((LocalHipByt::from(body), trailers, cursor - hdr_len))
    }

    pub fn get_header_content_type<'a>(&'a self) -> Option<HttpContentType<'a>> {
        HttpContentType::from_str(self.get_header_key(HeaderItem::Content_Type).unwrap_or(""))
    }

    pub fn is_websocket(&self) -> bool {
        if self.method != HttpMethod::GET {
            return false;
        }
        if self.get_header_connection() != HttpConnection::Upgrade {
            return false;
        }
        // RFC 6455 §4.2.1: Upgrade MUST be "websocket", version MUST be 13 and
        // a non-empty base64 Sec-WebSocket-Key MUST be present.
        if self
            .get_header_key(HeaderItem::Upgrade)
            .is_none_or(|val| val.to_lowercase() != "websocket")
        {
            return false;
        }
        if self.get_header("Sec-WebSocket-Version").map(|val| val != "13") != Some(false) {
            return false;
        }
        match self.get_header("Sec-WebSocket-Key") {
            Some(val) => {
                if val.len() != 24 || !val.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=') {
                    return false;
                }
                true
            }
            None => false,
        }
    }

    pub async fn upgrade_websocket(&mut self) -> anyhow::Result<Websocket> {
        if !self.is_websocket() {
            Err(anyhow!("it is not a websocket request"))?;
        }
        let ws_key = self
            .get_header("Sec-WebSocket-Key")
            .unwrap_or("")
            .to_string();
        // let ws_ext = req.get_header("Sec-WebSocket-Extensions").unwrap_or("".to_string());
        let stream = match self.stream.take() {
            Some(stream) => stream,
            None => Err(anyhow!("connot get stream"))?,
        };
        {
            let mut stream = stream.lock().await;
            let res = HttpResponse::from_websocket(&ws_key);
            stream.write_all(&res.as_bytes(CompressMode::None)).await?;
        }
        Ok(Websocket {
            stream,
            rbuf: Vec::new(),
            is_client: false,
        })
    }

    pub async fn get_client_addr(&self) -> anyhow::Result<SocketAddr> {
        if let Some(addr) = self.client_addr {
            return Ok(addr);
        }
        match self.get_ext::<SocketAddr>() {
            Some(addr) => Ok(*addr),
            None => Err(anyhow!("no addr info")),
        }
    }

    async fn from_stream_impl_into(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        limits: (Option<usize>, Option<usize>),
        req: &mut HttpRequest,
    ) -> anyhow::Result<usize> {
        let mut tmp_buf = [0u8; 4096];
        let max_header_bytes = limits
            .0
            .map(|limit| limit.min(ServerConfig::get_max_header_bytes()))
            .unwrap_or_else(ServerConfig::get_max_header_bytes);
        let hdr_len = loop {
            Self::ensure_header_section_size(buf, max_header_bytes)?;
            match HttpRequest::from_headers_part_with_limits_into(&buf[..], max_header_bytes, req)?
            {
                Some(hdr_len) => break hdr_len,
                None => {
                    let n = stream.read(&mut tmp_buf).await?;
                    if n == 0 {
                        return Err(anyhow::Error::msg("connection closed"));
                    }
                    buf.extend(&tmp_buf[0..n]);
                }
            }
        };
        let has_chunked_transfer_encoding = req.has_chunked_transfer_encoding()?;
        let mut content_length = 0usize;
        if has_chunked_transfer_encoding {
            if req.get_header_key(HeaderItem::Content_Length).is_some() {
                anyhow::bail!("conflicting headers: Transfer-Encoding and Content-Length");
            }
        } else {
            content_length = req.parse_header_content_length()?.unwrap_or(0);
        }

        // Body limit: per-request limit narrows the global default; the global
        // default always applies so unbounded request bodies are impossible.
        let max_body_bytes = limits
            .1
            .map(|limit| limit.min(ServerConfig::get_max_body_bytes()))
            .unwrap_or_else(ServerConfig::get_max_body_bytes);
        if content_length > max_body_bytes {
            anyhow::bail!(
                "request body too large: {content_length} bytes exceeds {max_body_bytes} bytes"
            );
        }

        let has_request_body = has_chunked_transfer_encoding || content_length > 0;
        req.process_expect_header(stream, has_request_body).await?;

        let bdy_len = if has_chunked_transfer_encoding {
            let allowed_trailers =
                parse_declared_trailer_names(req.get_header_key(HeaderItem::Trailer));
            let (body, trailers, consumed_len) =
                Self::read_chunked_body(buf, stream, hdr_len, &allowed_trailers, Some(max_body_bytes))
                    .await?;
            req.body = body;
            req.trailers = trailers;
            consumed_len
        } else {
            while hdr_len + content_length > buf.len() {
                let t = stream.read(&mut tmp_buf).await?;
                if t == 0 {
                    return Err(anyhow::Error::msg("connection closed"));
                }
                buf.extend(&tmp_buf[0..t]);
            }
            if content_length > 0 {
                req.body = LocalHipByt::from(&buf[hdr_len..hdr_len + content_length]);
            }
            content_length
        };

        // 按 Content-Type 解析 body 到 body_pairs/body_files
        Self::parse_body_content(req);
        Ok(hdr_len + bdy_len)
    }

    /// Parse `req.body` according to the request's Content-Type header,
    /// filling `body_pairs` / `body_files`. Shared by the HTTP/1.1 parser and
    /// the HTTP/2 & HTTP/3 server paths so typed handler parameters work on
    /// every protocol version.
    pub(crate) fn parse_body_content(req: &mut HttpRequest) {
        // 先获取Content-Type的字符串值，避免借用冲突
        let content_type_str = {
            req.get_header_key(HeaderItem::Content_Type)
                .map(|s| s.to_string())
        };

        // 根据内容类型字符串处理请求体
        if let Some(content_type_str) = content_type_str {
            // 解析内容类型
            let content_type_parsed = HttpContentType::from_str(&content_type_str);

            match content_type_parsed {
                Some(HttpContentType::ApplicationJson) => {
                    if let Ok(body_str) = std::str::from_utf8(&req.body) {
                        if let Ok(serde_json::Value::Object(obj)) =
                            serde_json::from_str::<serde_json::Value>(body_str)
                        {
                            for (k, v) in obj {
                                req.body_pairs
                                    .insert(LocalHipStr::from(k), LocalHipStr::from(v.to_string()));
                            }
                        }
                    }
                }
                Some(HttpContentType::ApplicationXWwwFormUrlencoded) => {
                    if let Ok(body_str) = std::str::from_utf8(&req.body) {
                        body_str.split('&').for_each(|s| {
                            if let Some((a, b)) = s.split_once('=') {
                                req.body_pairs
                                    .insert(a.url_decode().into(), b.url_decode().into());
                            }
                        });
                    }
                }
                Some(HttpContentType::MultipartFormData(boundary)) => {
                    // 按字节切分以支持二进制文件上传；仅头解析区做文本转换。
                    // 分隔符必须位于行首（RFC 2046），内容中间出现的
                    // boundary 文本不会切分。
                    let split_seq = ssformat!(64, "--{boundary}");
                    let split_seq = split_seq.as_bytes();
                    let body_bytes: &[u8] = &req.body[..];
                    for mut seg in split_multipart_at_boundary(body_bytes, split_seq) {
                        if seg == b"--" || seg.starts_with(b"--") {
                            break;
                        }
                        if seg.starts_with(b"\r\n") {
                            seg = &seg[2..];
                        }
                        if seg.ends_with(b"\r\n") {
                            seg = &seg[..seg.len() - 2];
                        }
                        let Some(hdr_end) = find_subslice(seg, b"\r\n\r\n") else {
                            continue;
                        };
                        let (key_part, content) = seg.split_at(hdr_end);
                        let content = &content[4..];
                        let key_str = String::from_utf8_lossy(key_part);
                        let keys: Vec<&str> = key_str
                            .split("\r\n")
                            .map(|p| p.split(";").collect::<Vec<_>>())
                            .collect::<Vec<_>>()
                            .into_iter()
                            .flatten()
                            .collect();
                        let mut name = None;
                        let mut filename = None;
                        for key in keys.into_iter() {
                            if let Some((k, v)) = key.trim().split_once('=') {
                                let v = strip_quoted(v.trim());
                                if v.is_empty() {
                                    continue;
                                }
                                if k == "name" {
                                    name = Some(LocalHipStr::from(v));
                                } else if k == "filename" {
                                    filename = Some(LocalHipStr::from(v));
                                }
                            }
                        }
                        if let Some(name) = name {
                            if let Some(filename) = filename {
                                let data = LocalHipByt::from(content.to_vec());
                                req.body_files.insert(name, PostFile { filename, data });
                            } else {
                                let text = String::from_utf8_lossy(content).to_string();
                                req.body_pairs.insert(name, LocalHipStr::from(text));
                            }
                        }
                    }
                }
                None => {}
            }
        }
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: Arc<Mutex<HttpStream>>,
    ) -> anyhow::Result<(Self, usize)> {
        let mut stream = stream.lock().await;
        let mut req = Self::new();
        let consumed =
            Self::from_stream_impl_into(buf, &mut stream, (None, None), &mut req).await?;
        Ok((req, consumed))
    }

    pub async fn from_stream_with_limits(
        buf: &mut Vec<u8>,
        stream: Arc<Mutex<HttpStream>>,
        max_header_bytes: Option<usize>,
        max_body_bytes: Option<usize>,
    ) -> anyhow::Result<(Self, usize)> {
        let mut stream = stream.lock().await;
        let mut req = Self::new();
        let consumed = Self::from_stream_impl_into(
            buf,
            &mut stream,
            (max_header_bytes, max_body_bytes),
            &mut req,
        )
        .await?;
        Ok((req, consumed))
    }

    pub(crate) async fn from_stream_with_limits_into(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        max_header_bytes: Option<usize>,
        max_body_bytes: Option<usize>,
        req: &mut HttpRequest,
    ) -> anyhow::Result<usize> {
        Self::from_stream_impl_into(buf, stream, (max_header_bytes, max_body_bytes), req).await
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        Self::from_headers_part_with_limits(buf, ServerConfig::get_max_header_bytes())
    }

    fn from_headers_part_with_limits_into(
        buf: &[u8],
        max_header_bytes: usize,
        req: &mut HttpRequest,
    ) -> anyhow::Result<Option<usize>> {
        let max_header_count = ServerConfig::get_max_header_count();
        Self::ensure_header_section_size(buf, max_header_bytes)?;

        let mut stack_headers = [httparse::EMPTY_HEADER; 48];
        let mut heap_headers: Vec<httparse::Header<'_>>;
        let headers: &mut [httparse::Header<'_>] = if max_header_count <= stack_headers.len() {
            &mut stack_headers
        } else {
            heap_headers = vec![httparse::EMPTY_HEADER; max_header_count];
            &mut heap_headers
        };
        let (rreq, n) = {
            let mut req = httparse::Request::new(headers);
            let n = match httparse::ParserConfig::default().parse_request(&mut req, buf) {
                Ok(httparse::Status::Complete(n)) => n,
                Ok(httparse::Status::Partial) => return Ok(None),
                Err(httparse::Error::TooManyHeaders) => {
                    anyhow::bail!(
                        "too many request headers: exceeds configured limit {max_header_count}"
                    )
                }
                Err(err) => Err(anyhow!(err))?,
            };
            (req, n)
        };
        let parsed_header_count = rreq.headers.iter().filter(|h| !h.name.is_empty()).count();
        if parsed_header_count > max_header_count {
            anyhow::bail!(
                "too many request headers: {parsed_header_count} exceeds {max_header_count}"
            );
        }
        req.clear_for_reuse();
        req.headers.reserve(parsed_header_count);
        let mut content_length_seen: Option<String> = None;
        let mut transfer_encoding_seen = false;
        let mut host_header_count = 0usize;
        let mut has_valid_host = false;
        req.method = {
            let method = rreq
                .method
                .ok_or_else(|| anyhow!("Missing HTTP method in request"))?;
            match method.len() {
                3 if method == "GET" => HttpMethod::GET,
                3 if method == "PUT" => HttpMethod::PUT,
                4 if method == "COPY" => HttpMethod::COPY,
                4 if method == "HEAD" => HttpMethod::HEAD,
                4 if method == "LOCK" => HttpMethod::LOCK,
                4 if method == "MOVE" => HttpMethod::MOVE,
                4 if method == "POST" => HttpMethod::POST,
                5 if method == "MKCOL" => HttpMethod::MKCOL,
                5 if method == "PATCH" => HttpMethod::PATCH,
                5 if method == "TRACE" => HttpMethod::TRACE,
                6 if method == "DELETE" => HttpMethod::DELETE,
                6 if method == "UNLOCK" => HttpMethod::UNLOCK,
                7 if method == "OPTIONS" => HttpMethod::OPTIONS,
                7 if method == "CONNECT" => HttpMethod::CONNECT,
                8 if method == "PROPFIND" => HttpMethod::PROPFIND,
                9 if method == "PROPPATCH" => HttpMethod::PROPPATCH,
                _ => anyhow::bail!("unsupported method: {method}"),
            }
        };
        let target = rreq
            .path
            .ok_or_else(|| anyhow!("Missing HTTP path in request"))?;
        req.parse_request_target(target)?;
        req.version = rreq.version.unwrap_or(1) + 10;
        for h in rreq.headers.iter() {
            if h.name.is_empty() {
                break;
            }
            let header_value = str::from_utf8(h.value)?;
            let normalized_header_value = header_value.trim();
            if h.name.eq_ignore_ascii_case("Content-Length") {
                let cl = normalized_header_value;
                if cl.is_empty() {
                    Err(anyhow!("empty Content-Length header"))?;
                }
                if let Some(prev) = &content_length_seen {
                    if prev != cl {
                        Err(anyhow!("conflicting duplicate Content-Length headers"))?;
                    }
                } else {
                    content_length_seen = Some(cl.to_string());
                }
            }
            if h.name.eq_ignore_ascii_case("Transfer-Encoding") {
                // RFC 9112: a recipient MUST reject messages containing more
                // than one Transfer-Encoding header field (request smuggling).
                if transfer_encoding_seen {
                    anyhow::bail!("duplicate Transfer-Encoding headers are not allowed");
                }
                if normalized_header_value.is_empty() {
                    anyhow::bail!("empty Transfer-Encoding header");
                }
                transfer_encoding_seen = true;
            }
            if h.name.eq_ignore_ascii_case("Host") {
                host_header_count += 1;
                if host_header_count > 1 {
                    anyhow::bail!("multiple Host headers are not allowed");
                }
                if normalized_header_value.is_empty() {
                    anyhow::bail!("empty Host header");
                }
                if http::uri::Authority::from_str(normalized_header_value).is_err() {
                    anyhow::bail!("invalid Host header");
                }
                has_valid_host = true;
            }
            if h.name.eq_ignore_ascii_case("Expect") {
                let expect_key: HeaderOrHipStr = HeaderItem::Expect.into();
                if let Some(existing) = req.headers.get(&expect_key) {
                    req.headers.insert(
                        expect_key,
                        LocalHipStr::from(format!(
                            "{}, {normalized_header_value}",
                            existing.as_str()
                        )),
                    );
                } else {
                    req.headers
                        .insert(expect_key, LocalHipStr::from(normalized_header_value));
                }
            } else {
                req.headers.insert(
                    HeaderOrHipStr::from_str(h.name),
                    LocalHipStr::from(normalized_header_value),
                );
            }
        }
        if req.version >= 11 && !has_valid_host {
            anyhow::bail!("missing required Host header");
        }
        Ok(Some(n))
    }

    fn from_headers_part_with_limits(
        buf: &[u8],
        max_header_bytes: usize,
    ) -> anyhow::Result<Option<(Self, usize)>> {
        let mut req = Self::new();
        let consumed = Self::from_headers_part_with_limits_into(buf, max_header_bytes, &mut req)?;
        Ok(consumed.map(|consumed| (req, consumed)))
    }

    /// Check HTTP conditional preflight headers to determine if special status codes should be returned
    ///
    /// This method handles the following HTTP conditional headers:
    /// - If-Modified-Since: Return 304 if resource is not modified
    /// - If-None-Match: Return 304 if ETag matches
    /// - If-Match: Return 412 if ETag doesn't match
    /// - If-Unmodified-Since: Return 412 if resource is modified
    ///
    /// # Parameters
    /// - `meta`: File metadata (optional)
    /// - `etag`: Resource's ETag value (optional)
    ///
    /// # Return Values
    /// - `PreflightResult::Proceed`: Pass preflight check, can continue processing
    /// - `PreflightResult::NotModified`: Should return 304 status code
    /// - `PreflightResult::PreconditionFailed`: Should return 412 status code
    pub fn check_precondition_headers(
        &self,
        meta: Option<&Metadata>,
        etag: Option<&str>,
    ) -> PreflightResult {
        use crate::utils::refstr::HeaderItem;
        use std::time::UNIX_EPOCH;

        let is_get_or_head = matches!(self.method, HttpMethod::GET | HttpMethod::HEAD);

        // Get file's last modified time (Unix timestamp in seconds)
        let last_modified_timestamp = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        // Check If-Match header (if exists and doesn't match, return 412)
        if let Some(if_match) = self.get_header_key(HeaderItem::If_Match) {
            if if_match != "*" {
                if let Some(current_etag) = etag {
                    // Parse ETag list in If-Match
                    let match_found = if_match
                        .split(',')
                        .map(|s| s.trim())
                        .any(|expected_etag| expected_etag == current_etag);

                    if !match_found {
                        return PreflightResult::PreconditionFailed;
                    }
                } else {
                    // No ETag but client requires match, return 412
                    return PreflightResult::PreconditionFailed;
                }
            }
        }

        // Check If-Unmodified-Since header (if resource is modified, return 412)
        if let Some(if_unmodified_since) = self.get_header_key(HeaderItem::If_Unmodified_Since) {
            if let Some(last_modified) = last_modified_timestamp {
                if let Ok(since_timestamp) = parse_http_date(if_unmodified_since) {
                    if last_modified > since_timestamp {
                        return PreflightResult::PreconditionFailed;
                    }
                }
            }
        }

        // Check If-None-Match header
        if let Some(if_none_match) = self.get_header_key(HeaderItem::If_None_Match) {
            if if_none_match == "*" {
                // If resource exists, GET/HEAD -> 304, others -> 412.
                if etag.is_some() || meta.is_some() {
                    return if is_get_or_head {
                        PreflightResult::NotModified
                    } else {
                        PreflightResult::PreconditionFailed
                    };
                }
            } else if let Some(current_etag) = etag {
                // Check if ETag is in If-None-Match list.
                let match_found = if_none_match
                    .split(',')
                    .map(|s| s.trim())
                    .any(|expected_etag| expected_etag == current_etag);

                if match_found {
                    return if is_get_or_head {
                        PreflightResult::NotModified
                    } else {
                        PreflightResult::PreconditionFailed
                    };
                }
            }
        }

        // Check If-Modified-Since header (if resource is not modified, return 304)
        // Note: only applies to GET/HEAD and only when there's no If-None-Match header.
        if is_get_or_head && self.get_header_key(HeaderItem::If_None_Match).is_none() {
            if let Some(if_modified_since) = self.get_header_key(HeaderItem::If_Modified_Since) {
                if let Some(last_modified) = last_modified_timestamp {
                    if let Ok(since_timestamp) = parse_http_date(if_modified_since) {
                        if last_modified <= since_timestamp {
                            return PreflightResult::NotModified;
                        }
                    }
                }
            }
        }

        PreflightResult::Proceed
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let use_chunked = self
            .get_header_key(HeaderItem::Transfer_Encoding)
            .map(|encodings| {
                encodings
                    .split(',')
                    .map(|coding| coding.trim())
                    .any(|coding| coding.eq_ignore_ascii_case("chunked"))
            })
            .unwrap_or(false);

        let declared_trailer_names =
            parse_declared_trailer_names(self.get_header_key(HeaderItem::Trailer));
        let mut outbound_trailers: Vec<(String, String)> = Vec::with_capacity(self.trailers.len());
        for (key, value) in self.trailers.iter() {
            let key_str = key.to_str();
            let lower = key_str.to_ascii_lowercase();
            if is_forbidden_trailer_field(&lower) {
                continue;
            }
            if !declared_trailer_names.is_empty() && !declared_trailer_names.contains(&lower) {
                continue;
            }
            outbound_trailers.push((key_str.to_string(), value.to_string()));
        }

        let mut req_str = format!(
            "{} {} {}\r\n",
            self.method,
            self.request_target(),
            self.request_version_line()
        );
        for (k, v) in self.headers.iter() {
            if let HeaderOrHipStr::HeaderItem(HeaderItem::Content_Length) = k {
                continue;
            }
            req_str.push_str(&format!("{}: {v}\r\n", k.to_str()));
        }
        if use_chunked {
            if declared_trailer_names.is_empty() && !outbound_trailers.is_empty() {
                let trailer_names = outbound_trailers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                req_str.push_str(&format!("Trailer: {trailer_names}\r\n"));
            }
            req_str.push_str("\r\n");
            let mut ret = req_str.as_bytes().to_vec();
            if self.body.is_empty() {
                if outbound_trailers.is_empty() {
                    ret.extend_from_slice(b"0\r\n\r\n");
                } else {
                    ret.extend_from_slice(b"0\r\n");
                    for (name, value) in outbound_trailers.iter() {
                        ret.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
                    }
                    ret.extend_from_slice(b"\r\n");
                }
            } else {
                ret.extend_from_slice(format!("{:x}\r\n", self.body.len()).as_bytes());
                ret.extend(&self.body[..]);
                if outbound_trailers.is_empty() {
                    ret.extend_from_slice(b"\r\n0\r\n\r\n");
                } else {
                    ret.extend_from_slice(b"\r\n0\r\n");
                    for (name, value) in outbound_trailers.iter() {
                        ret.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
                    }
                    ret.extend_from_slice(b"\r\n");
                }
            }
            ret
        } else {
            req_str.push_str(&format!(
                "{}: {}\r\n",
                HeaderItem::Content_Length.to_str(),
                self.body.len()
            ));
            req_str.push_str("\r\n");
            let mut ret = req_str.as_bytes().to_vec();
            ret.extend(&self.body[..]);
            ret
        }
    }
}

#[derive(Debug)]
pub enum HttpResponseBody {
    Data(Vec<u8>),
    Stream(Receiver<Vec<u8>>),
}

pub struct HttpResponseBodyStream<'a> {
    body: &'a mut HttpResponseBody,
    data_consumed: bool,
}

impl HttpResponseBody {
    pub async fn data(&mut self) -> &[u8] {
        if let HttpResponseBody::Stream(rx) = self {
            let mut buf = vec![];
            while let Some(chunk) = rx.recv().await {
                buf.extend_from_slice(&chunk);
            }
            *self = HttpResponseBody::Data(buf);
        }

        match self {
            HttpResponseBody::Data(data) => data.as_slice(),
            HttpResponseBody::Stream(_) => &[], // Should not reach here
        }
    }

    pub async fn into_data(mut self) -> Vec<u8> {
        self.data().await;
        match self {
            HttpResponseBody::Data(data) => data,
            HttpResponseBody::Stream(_) => vec![],
        }
    }

    pub fn stream_data(&mut self) -> HttpResponseBodyStream<'_> {
        HttpResponseBodyStream {
            body: self,
            data_consumed: false,
        }
    }
}

impl HttpResponseBodyStream<'_> {
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        match self.body {
            HttpResponseBody::Data(data) => {
                if self.data_consumed {
                    None
                } else {
                    self.data_consumed = true;
                    Some(data.clone())
                }
            }
            HttpResponseBody::Stream(rx) => rx.recv().await,
        }
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub version: Cow<'static, str>,
    pub http_code: u16,
    pub headers: FxHashMap<Cow<'static, str>, Cow<'static, str>>,
    /// Extra values for repeated header fields (e.g. multiple `Set-Cookie`).
    /// The main `headers` map keeps only the first value of each field; every
    /// later occurrence is appended here and serialized after the map values.
    pub multi_headers: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    pub trailers: FxHashMap<Cow<'static, str>, Cow<'static, str>>,
    pub body: HttpResponseBody,
}
unsafe impl Send for HttpResponse {}
unsafe impl Sync for HttpResponse {}
impl Clone for HttpResponse {
    fn clone(&self) -> Self {
        Self {
            version: self.version.clone(),
            http_code: self.http_code,
            headers: self.headers.clone(),
            multi_headers: self.multi_headers.clone(),
            trailers: self.trailers.clone(),
            body: match &self.body {
                HttpResponseBody::Data(data) => HttpResponseBody::Data(data.clone()),
                HttpResponseBody::Stream(_) => HttpResponseBody::Data(vec![]), // Clone stream as empty data
            },
        }
    }
}

macro_rules! make_resp_by_text {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: impl Into<String>) -> Self {
            let body = body.into();
            Self {
                version: Cow::Borrowed("HTTP/1.1"),
                http_code: 200,
                headers: Self::default_headers($cnt_type),
                multi_headers: Vec::new(),
                trailers: FxHashMap::default(),
                body: HttpResponseBody::Data(body.into_bytes()),
            }
        }
    };
}

macro_rules! make_resp_by_binary {
    ($fn_name:ident, $cnt_type:expr) => {
        pub fn $fn_name(body: &[u8]) -> Self {
            Self {
                version: Cow::Borrowed("HTTP/1.1"),
                http_code: 200,
                headers: Self::default_headers($cnt_type),
                multi_headers: Vec::new(),
                trailers: FxHashMap::default(),
                body: HttpResponseBody::Data(body.to_vec()),
            }
        }
    };
}

impl Default for HttpResponse {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpResponse {
    make_resp_by_text!(html, "text/html");
    make_resp_by_text!(css, "text/css");
    make_resp_by_text!(csv, "text/csv");
    make_resp_by_text!(js, "text/javascript");
    make_resp_by_text!(text, "text/plain");
    make_resp_by_text!(json, "application/json");
    make_resp_by_text!(xml, "application/xml");
    make_resp_by_binary!(png, "image/png");

    fn default_headers(
        cnt_type: impl Into<Cow<'static, str>>,
    ) -> FxHashMap<Cow<'static, str>, Cow<'static, str>> {
        let current_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|timestamp| timestamp.as_secs() as i64)
            .unwrap_or_default();

        static TL_TIMESTAMP: ThreadLocal<RefCell<(i64, Cow<'static, str>)>> = ThreadLocal::new();
        let mut tl_timestamp = TL_TIMESTAMP.get_or_default().borrow_mut();
        let date_str = if current_ts == tl_timestamp.0 {
            tl_timestamp.1.clone()
        } else {
            let new_date: Cow<'_, str> = Utc::now()
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string()
                .into();
            *tl_timestamp = (current_ts, new_date.clone());
            new_date
        };

        [
            ("Date".into(), date_str),
            ("Server".into(), Cow::Borrowed(SERVER_STR.as_str())),
            ("Connection".into(), "keep-alive".into()),
            ("Content-Type".into(), cnt_type.into()),
            ("Pragma".into(), "no-cache".into()),
            ("Cache-Control".into(), "no-cache".into()),
        ]
        .into_iter()
        .collect()
    }

    pub fn new() -> Self {
        Self {
            version: Cow::Borrowed(""),
            http_code: 0,
            headers: FxHashMap::with_capacity_and_hasher(6, Default::default()),
            multi_headers: Vec::new(),
            trailers: FxHashMap::default(),
            body: HttpResponseBody::Data(vec![]),
        }
    }

    pub fn add_header(&mut self, key: Cow<'static, str>, value: Cow<'static, str>) {
        self.headers.insert(key, value);
    }

    /// Append an additional value for a header field, keeping any existing
    /// value. Use this for fields that may repeat (e.g. `Set-Cookie`).
    pub fn append_header(&mut self, key: Cow<'static, str>, value: Cow<'static, str>) {
        if !self.headers.contains_key(&key) {
            self.headers.insert(key.clone(), value.clone());
        } else {
            self.multi_headers.push((key, value));
        }
    }

    pub fn not_found() -> Self {
        let mut ret = Self::html("404 not found");
        ret.http_code = 404;
        ret
    }

    pub fn error(payload: impl Into<String>) -> Self {
        let mut ret = Self::html(payload);
        ret.http_code = 500;
        ret
    }

    pub fn bad_request(payload: impl Into<String>) -> Self {
        let mut ret = Self::html(payload);
        ret.http_code = 400;
        ret
    }

    pub fn empty() -> Self {
        Self::html("")
    }

    pub fn chunked(rx: Receiver<Vec<u8>>) -> Self {
        Self {
            version: Cow::Borrowed("HTTP/1.1"),
            http_code: 200,
            headers: [
                ("Transfer-Encoding".into(), "chunked".into()),
                ("Content-Type".into(), "application/octet-stream".into()),
                ("Cache-Control".into(), "no-cache".into()),
                ("Connection".into(), "keep-alive".into()),
            ]
            .into_iter()
            .collect(),
            multi_headers: Vec::new(),
            trailers: FxHashMap::default(),
            body: HttpResponseBody::Stream(rx),
        }
    }

    /// Create a SSE response with chunked transfer encoding
    pub fn sse(rx: Receiver<Vec<u8>>) -> Self {
        let mut res = Self::chunked(rx);
        res.add_header("Content-Type".into(), "text/event-stream".into());
        res
    }

    /// Synchronously read the whole file into memory.
    ///
    /// **Performance note:** this blocks the async worker and buffers the
    /// entire file; prefer the built-in static routes (which stream large
    /// files) or keep it to small files only.
    pub fn from_file(path: &str, download: bool, meta: Option<Metadata>) -> Self {
        let mut buffer = vec![];
        let mut response_error = None;
        match File::open(path) {
            Ok(mut file) => {
                if let Err(err) = file.read_to_end(&mut buffer) {
                    eprintln!("[potato] file read error for {path}: {err}");
                    response_error = Some("failed to read file".to_string());
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                response_error = Some("file not found".to_string());
            }
            Err(err) => {
                eprintln!("[potato] file open error for {path}: {err}");
                response_error = Some("failed to open file".to_string());
            }
        }
        let mut response = Self::from_mem_file(path, buffer, download, meta);
        if let Some(message) = response_error {
            let mut error = Self::error(message);
            error.http_code = response.http_code.max(500);
            response = error;
        }
        response
    }

    pub fn from_mem_file(
        path: &str,
        data: Vec<u8>,
        download: bool,
        meta: Option<Metadata>,
    ) -> Self {
        if let Some(meta) = meta {
            let mut ret = Self::from_mem_file(path, data, download, None);
            // Add Last-Modified header
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_time =
                        chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH + duration);
                    ret.add_header(
                        "Last-Modified".into(),
                        modified_time
                            .format("%a, %d %b %Y %H:%M:%S GMT")
                            .to_string()
                            .into(),
                    );
                }
            }

            // Add ETag header (format: "hex-file-modified-time-hex-file-size")
            if let Ok(modified) = meta.modified() {
                if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                    let modified_secs = duration.as_secs();
                    let file_size = meta.len();
                    let etag = format!("\"{:x}-{:x}\"", modified_secs, file_size);
                    ret.add_header("ETag".into(), etag.into());
                }
            }
            ret
        } else {
            let mut ret = Self::empty();
            let mime_type = match path.split('.').next_back() {
                Some("css") => "text/css",
                Some("csv") => "text/csv",
                Some("htm") => "text/html",
                Some("html") => "text/html",
                Some("js") => "application/javascript",
                Some("json") => "application/json",
                Some("pdf") => "application/pdf",
                Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                Some("xml") => "application/xml",
                _ if path.ends_with('/') => "text/html",
                _ => "application/octet-stream",
            };
            ret.add_header("Content-Type".into(), mime_type.into());
            if download {
                let file = match path.rfind('/') {
                    Some(p) => &path[p + 1..],
                    None => path,
                };
                if !file.is_empty() {
                    ret.add_header(
                        "Content-Disposition".into(),
                        crate::serde_json::to_string(file)
                            .unwrap_or_else(|_| format!("\"{file}\""))
                            .into(),
                    );
                }
            }
            ret.body = HttpResponseBody::Data(data);
            ret
        }
    }

    pub fn from_websocket(ws_key: &str) -> Self {
        #[allow(deprecated)]
        let ws_accept = {
            let mut sha1 = Sha1::default();
            sha1.update(ws_key);
            sha1.update(&b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"[..]);
            base64::encode(sha1.finalize())
        };
        Self {
            version: Cow::Borrowed("HTTP/1.1"),
            http_code: 101,
            headers: [
                (
                    "Date".into(),
                    Utc::now()
                        .format("%a, %d %b %Y %H:%M:%S GMT")
                        .to_string()
                        .into(),
                ),
                ("Server".into(), SERVER_STR.clone().into()),
                ("Connection".into(), "Upgrade".into()),
                ("Upgrade".into(), "websocket".into()),
                ("Sec-WebSocket-Accept".into(), ws_accept.into()),
            ]
            .into_iter()
            .collect(),
            multi_headers: Vec::new(),
            trailers: FxHashMap::default(),
            body: HttpResponseBody::Data(vec![]),
        }
    }

    pub fn add_trailer(&mut self, key: Cow<'static, str>, value: Cow<'static, str>) {
        self.trailers.insert(key, value);
    }

    pub fn get_trailer(&self, key: &str) -> Option<&str> {
        self.trailers.get(key).map(|v| v.as_ref())
    }

    fn status_disallows_response_body(status: u16) -> bool {
        (100..200).contains(&status) || status == 204 || status == 304
    }

    fn method_disallows_response_body(request_method: Option<HttpMethod>) -> bool {
        request_method == Some(HttpMethod::HEAD)
    }

    fn transfer_encoding_has_chunked(raw: &str) -> anyhow::Result<bool> {
        let codings = parse_transfer_encoding_tokens(raw)?;
        if codings.iter().any(|coding| coding == "chunked") {
            if codings.last().is_some_and(|coding| coding == "chunked") {
                return Ok(true);
            }
            Err(anyhow!(
                "invalid Transfer-Encoding order: chunked must be final coding"
            ))?;
        }
        Err(anyhow!("unsupported Transfer-Encoding: {raw}"))
    }

    async fn read_chunked_body(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        hdr_len: usize,
        allowed_trailers: &HashSet<String>,
    ) -> anyhow::Result<(
        Vec<u8>,
        FxHashMap<Cow<'static, str>, Cow<'static, str>>,
        usize,
    )> {
        let mut cursor = hdr_len;
        let mut body = Vec::new();
        let mut trailers = FxHashMap::with_capacity_and_hasher(4, Default::default());
        let mut tmp_buf = [0u8; 4096];
        // Client side: a hostile server can stream chunks forever, so bound
        // the assembled body the same way request bodies are bounded.
        let max_body_bytes = ServerConfig::get_max_body_bytes();
        const MAX_METADATA_LINE_BYTES: usize = 16 * 1024;

        loop {
            let line_end = loop {
                if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n") {
                    break cursor + pos;
                }
                if buf.len() - cursor >= MAX_METADATA_LINE_BYTES {
                    anyhow::bail!("chunk metadata line too large");
                }
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            };

            let chunk_size = {
                let size_line = str::from_utf8(&buf[cursor..line_end])?;
                let size_token = size_line
                    .split_once(';')
                    .map_or(size_line, |(size, _)| size)
                    .trim();
                if size_token.is_empty() {
                    Err(anyhow!("invalid chunk size"))?;
                }
                usize::from_str_radix(size_token, 16)?
            };
            cursor = line_end + 2;

            if chunk_size == 0 {
                loop {
                    let line_start = cursor;
                    let line_end = loop {
                        if let Some(pos) = buf[cursor..].windows(2).position(|part| part == b"\r\n")
                        {
                            break cursor + pos;
                        }
                        if buf.len() - cursor >= MAX_METADATA_LINE_BYTES {
                            anyhow::bail!("chunk trailer line too large");
                        }
                        let n = stream.read(&mut tmp_buf).await?;
                        if n == 0 {
                            Err(anyhow::Error::msg("connection closed"))?;
                        }
                        buf.extend(&tmp_buf[..n]);
                    };
                    cursor = line_end + 2;
                    if line_end == line_start {
                        break;
                    }

                    let (name, value) = parse_trailer_line(&buf[line_start..line_end])?;
                    let name_lower = name.to_ascii_lowercase();
                    if is_forbidden_trailer_field(&name_lower) {
                        Err(anyhow!("forbidden trailer field: {name}"))?;
                    }
                    if !allowed_trailers.contains(&name_lower) {
                        Err(anyhow!("unexpected trailer field: {name}"))?;
                    }
                    trailers.insert(name.into(), value.into());
                }
                break;
            }

            if body.len().saturating_add(chunk_size) > max_body_bytes {
                anyhow::bail!(
                    "response body too large: chunked body exceeds {max_body_bytes} bytes"
                );
            }

            let Some(chunk_end) = cursor.checked_add(chunk_size).and_then(|v| v.checked_add(2))
            else {
                anyhow::bail!("chunk size overflow");
            };
            while buf.len() < chunk_end {
                let n = stream.read(&mut tmp_buf).await?;
                if n == 0 {
                    Err(anyhow::Error::msg("connection closed"))?;
                }
                buf.extend(&tmp_buf[..n]);
            }
            body.extend_from_slice(&buf[cursor..cursor + chunk_size]);
            if &buf[cursor + chunk_size..cursor + chunk_size + 2] != b"\r\n" {
                Err(anyhow!("invalid chunk terminator"))?;
            }
            cursor += chunk_size + 2;
        }

        Ok((body, trailers, cursor - hdr_len))
    }

    pub fn as_bytes(&self, mut cmode: CompressMode) -> Vec<u8> {
        match &self.body {
            HttpResponseBody::Data(data) => {
                let suppress_body = Self::status_disallows_response_body(self.http_code);
                let mut payload_tmp: Vec<u8> = vec![];
                if cmode == CompressMode::Gzip
                    && data.len() >= 32
                    && self.get_header("Content-Encoding").is_none()
                    && !suppress_body
                {
                    if let Ok(compressed_data) = data.compress() {
                        payload_tmp = compressed_data;
                    }
                }
                let mut payload_ref = if payload_tmp.is_empty() {
                    cmode = CompressMode::None;
                    data.as_slice()
                } else {
                    payload_tmp.as_slice()
                };
                if suppress_body {
                    cmode = CompressMode::None;
                    payload_ref = &[];
                }
                use std::io::Write as _;
                let mut ret = Vec::with_capacity(payload_ref.len().saturating_add(512));
                let status_str = self.http_code.http_code_to_desp();
                let _ = write!(ret, "{} {} {status_str}\r\n", self.version, self.http_code);
                for (key, value) in self.headers.iter() {
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    let _ = write!(
                        ret,
                        "{}: {}\r\n",
                        sanitize_header_str(key),
                        sanitize_header_str(value)
                    );
                }
                for (key, value) in self.multi_headers.iter() {
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    let _ = write!(
                        ret,
                        "{}: {}\r\n",
                        sanitize_header_str(key),
                        sanitize_header_str(value)
                    );
                }
                if !suppress_body {
                    let _ = write!(ret, "Content-Length: {}\r\n", payload_ref.len());
                }
                if cmode == CompressMode::Gzip && !suppress_body {
                    ret.extend_from_slice(b"Content-Encoding: gzip\r\n");
                }
                // 头部终止空行（v2.28 拍板修复的还原）：101 Switching Protocols
                // 走本函数序列化（suppress_body 无 Content-Length 行），缺此
                // 空行即畸形 101，RFC6455 客户端（浏览器/wslite）握手永久
                // 挂死。与 write_to_stream 的线上形态对齐。
                ret.extend_from_slice(b"\r\n");
                ret.extend(payload_ref);
                ret
            }
            HttpResponseBody::Stream(_) => vec![], // Stream responses are handled separately
        }
    }

    /// Write response to stream, handling both Data and Stream body types
    pub async fn write_to_stream(
        &mut self,
        stream: &mut crate::utils::tcp_stream::HttpStream,
        cmode: CompressMode,
        request_method: Option<HttpMethod>,
    ) -> anyhow::Result<()> {
        let suppress_body_by_status = Self::status_disallows_response_body(self.http_code);
        let suppress_body_by_method = Self::method_disallows_response_body(request_method);
        let suppress_body = suppress_body_by_status || suppress_body_by_method;
        let no_content_encoding = self.get_header("Content-Encoding").is_none();
        let declared_trailer_names = parse_declared_trailer_names(self.get_header("Trailer"));
        let mut outbound_stream_trailers: Vec<(String, String)> =
            Vec::with_capacity(self.trailers.len());
        for (key, value) in self.trailers.iter() {
            let lower = key.to_ascii_lowercase();
            if is_forbidden_trailer_field(&lower) {
                continue;
            }
            if !declared_trailer_names.is_empty() && !declared_trailer_names.contains(&lower) {
                continue;
            }
            outbound_stream_trailers.push((key.to_string(), value.to_string()));
        }
        match &mut self.body {
            HttpResponseBody::Data(data) => {
                let mut payload_tmp: Vec<u8> = vec![];
                let mut cmode = cmode;
                if cmode == CompressMode::Gzip
                    && data.len() >= 32
                    && no_content_encoding
                    && !suppress_body_by_status
                {
                    if let Ok(compressed_data) = data.compress() {
                        payload_tmp = compressed_data;
                    }
                }
                let mut payload_ref = if payload_tmp.is_empty() {
                    cmode = CompressMode::None;
                    data.as_slice()
                } else {
                    payload_tmp.as_slice()
                };
                if suppress_body_by_status {
                    cmode = CompressMode::None;
                    payload_ref = &[];
                }

                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                ret.push_str(&ssformat!(
                    64,
                    "{} {} {status_str}\r\n",
                    self.version,
                    self.http_code
                ));
                if self.headers.len() == 6 && self.multi_headers.is_empty() {
                    if let (
                        Some(date),
                        Some(server),
                        Some(connection),
                        Some(content_type),
                        Some(pragma),
                        Some(cache_control),
                    ) = (
                        self.headers.get("Date"),
                        self.headers.get("Server"),
                        self.headers.get("Connection"),
                        self.headers.get("Content-Type"),
                        self.headers.get("Pragma"),
                        self.headers.get("Cache-Control"),
                    ) {
                        use std::fmt::Write as _;
                        let _ = write!(ret, "Date: {}\r\n", sanitize_header_str(date));
                        let _ = write!(ret, "Server: {}\r\n", sanitize_header_str(server));
                        let _ = write!(ret, "Connection: {}\r\n", sanitize_header_str(connection));
                        let _ =
                            write!(ret, "Content-Type: {}\r\n", sanitize_header_str(content_type));
                        let _ = write!(ret, "Pragma: {}\r\n", sanitize_header_str(pragma));
                        let _ = write!(
                            ret,
                            "Cache-Control: {}\r\n",
                            sanitize_header_str(cache_control)
                        );
                    } else {
                        for (key, value) in self.headers.iter() {
                            if key == "Content-Length"
                                || key.eq_ignore_ascii_case("Transfer-Encoding")
                            {
                                continue;
                            }
                            ret.push_str(&ssformat!(
                                512,
                                "{}: {}\r\n",
                                sanitize_header_str(key),
                                sanitize_header_str(value)
                            ));
                        }
                    }
                } else {
                    for (key, value) in self.headers.iter() {
                        if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding")
                        {
                            continue;
                        }
                        ret.push_str(&ssformat!(
                            512,
                            "{}: {}\r\n",
                            sanitize_header_str(key),
                            sanitize_header_str(value)
                        ));
                    }
                }
                for (key, value) in self.multi_headers.iter() {
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    ret.push_str(&ssformat!(
                        512,
                        "{}: {}\r\n",
                        sanitize_header_str(key),
                        sanitize_header_str(value)
                    ));
                }
                if !suppress_body_by_status {
                    ret.push_str(&ssformat!(64, "Content-Length: {}\r\n", payload_ref.len()));
                }
                if cmode == CompressMode::Gzip && !suppress_body_by_status {
                    ret.push_str("Content-Encoding: gzip\r\n");
                }
                ret.push_str("\r\n");

                if suppress_body || payload_ref.is_empty() {
                    stream.write_all(ret.as_bytes()).await?;
                } else {
                    stream
                        .write_all_vectored2(ret.as_bytes(), payload_ref)
                        .await?;
                }
            }
            HttpResponseBody::Stream(rx) => {
                // For Stream body, send headers first, then chunks
                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                ret.push_str(&ssformat!(
                    64,
                    "{} {} {status_str}\r\n",
                    self.version,
                    self.http_code
                ));
                for (key, value) in self.headers.iter() {
                    // 流式响应统一由写出处补 TE: chunked，headers 里预置的
                    // （如 HttpResponse::chunked）必须跳过，避免重复头
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    ret.push_str(&ssformat!(
                        512,
                        "{}: {}\r\n",
                        sanitize_header_str(key),
                        sanitize_header_str(value)
                    ));
                }
                for (key, value) in self.multi_headers.iter() {
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    ret.push_str(&ssformat!(
                        512,
                        "{}: {}\r\n",
                        sanitize_header_str(key),
                        sanitize_header_str(value)
                    ));
                }
                if !suppress_body {
                    ret.push_str("Transfer-Encoding: chunked\r\n");
                }
                if declared_trailer_names.is_empty() && !outbound_stream_trailers.is_empty() {
                    let trailer_names = outbound_stream_trailers
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    ret.push_str(&ssformat!(512, "Trailer: {trailer_names}\r\n"));
                }
                ret.push_str("\r\n");
                let header_bytes: Vec<u8> = ret.as_bytes().to_vec();
                stream.write_all(&header_bytes).await?;

                if suppress_body {
                    return Ok(());
                }

                // Send chunks
                while let Some(chunk) = rx.recv().await {
                    if chunk.is_empty() {
                        break;
                    }
                    // Write chunked encoding: length in hex, \r\n, data, \r\n
                    let chunk_len_hex = format!("{:x}\r\n", chunk.len());
                    stream.write_all(chunk_len_hex.as_bytes()).await?;
                    stream.write_all(&chunk).await?;
                    stream.write_all(b"\r\n").await?;
                }
                // Send final chunk (length 0)
                if outbound_stream_trailers.is_empty() {
                    stream.write_all(b"0\r\n\r\n").await?;
                } else {
                    stream.write_all(b"0\r\n").await?;
                    for (name, value) in outbound_stream_trailers.iter() {
                        let trailer_line = format!(
                            "{}: {}\r\n",
                            sanitize_header_str(name),
                            sanitize_header_str(value)
                        );
                        stream.write_all(trailer_line.as_bytes()).await?;
                    }
                    stream.write_all(b"\r\n").await?;
                }
            }
        }
        Ok(())
    }

    pub async fn from_stream(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
    ) -> anyhow::Result<(Self, usize)> {
        Self::from_stream_with_request_method(buf, stream, None).await
    }

    pub async fn from_stream_with_request_method(
        buf: &mut Vec<u8>,
        stream: &mut HttpStream,
        request_method: Option<HttpMethod>,
    ) -> anyhow::Result<(Self, usize)> {
        let (mut res, hdr_len) = loop {
            match Self::from_headers_part(&buf[..])? {
                Some((res, hdr_len)) => {
                    if (100..200).contains(&res.http_code) && res.http_code != 101 {
                        // Interim response (e.g. 100 Continue): discard it and
                        // keep waiting for the final response. 101 marks a
                        // protocol switch and IS the final response here.
                        buf.copy_within(hdr_len.., 0);
                        buf.truncate(buf.len() - hdr_len);
                        continue;
                    }
                    break (res, hdr_len);
                }
                None => {
                    // Hostile server can withhold the header terminator:
                    // bound the header section the same way requests are.
                    if buf.len() > ServerConfig::get_max_header_bytes() {
                        anyhow::bail!("response header section too large");
                    }
                    buf.extend_by_streams(stream).await?;
                }
            }
        };
        let mut bdy_len = 0;
        let skip_body = request_method == Some(HttpMethod::HEAD)
            || Self::status_disallows_response_body(res.http_code);
        if skip_body {
            return Ok((res, hdr_len));
        }
        let has_chunked_transfer_encoding =
            if let Some(raw_te) = res.headers.get("Transfer-Encoding") {
                Self::transfer_encoding_has_chunked(raw_te)?
            } else {
                false
            };
        if has_chunked_transfer_encoding && res.headers.contains_key("Content-Length") {
            Err(anyhow!(
                "conflicting headers: Transfer-Encoding and Content-Length"
            ))?;
        }
        if has_chunked_transfer_encoding {
            let allowed_trailers = parse_declared_trailer_names(res.get_header("Trailer"));
            let (chunked_body, trailers, consumed_len) =
                Self::read_chunked_body(buf, stream, hdr_len, &allowed_trailers).await?;
            bdy_len = consumed_len;
            res.trailers = trailers;
            res.body = HttpResponseBody::Data(chunked_body);
        } else if let Some(cnt_len) = res.headers.get("Content-Length") {
            bdy_len = cnt_len.parse::<usize>().unwrap_or(0);
            let max_body_bytes = ServerConfig::get_max_body_bytes();
            if bdy_len > max_body_bytes {
                anyhow::bail!(
                    "response body too large: {bdy_len} bytes exceeds {max_body_bytes} bytes"
                );
            }
            while hdr_len + bdy_len > buf.len() {
                buf.extend_by_streams(stream).await?;
            }
            res.body = HttpResponseBody::Data(buf[hdr_len..hdr_len + bdy_len].to_vec());
        }

        Ok((res, hdr_len + bdy_len))
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        let mut headers = [httparse::EMPTY_HEADER; 96];
        let (rres, n) = {
            let mut res = httparse::Response::new(&mut headers);
            let n = match httparse::ParserConfig::default().parse_response(&mut res, buf)? {
                httparse::Status::Complete(n) => n,
                httparse::Status::Partial => return Ok(None),
            };
            (res, n)
        };

        let mut req = HttpResponse::new();
        let mut content_length_seen: Option<String> = None;
        let mut transfer_encoding_seen = false;
        req.version = format!("HTTP/1.{}", rres.version.unwrap_or(0)).into();
        req.http_code = rres.code.unwrap_or(0);
        for h in rres.headers.iter() {
            if h.name.is_empty() {
                break;
            }
            let header_value = str::from_utf8(h.value)?.trim();
            if h.name.eq_ignore_ascii_case("Content-Length") {
                if header_value.is_empty() {
                    Err(anyhow!("empty Content-Length header"))?;
                }
                if let Some(prev) = &content_length_seen {
                    if prev != header_value {
                        Err(anyhow!("conflicting duplicate Content-Length headers"))?;
                    }
                } else {
                    content_length_seen = Some(header_value.to_string());
                    req.headers
                        .insert("Content-Length".into(), header_value.to_string().into());
                }
                continue;
            }
            if h.name.eq_ignore_ascii_case("Transfer-Encoding") {
                if header_value.is_empty() {
                    Err(anyhow!("empty Transfer-Encoding header"))?;
                }
                transfer_encoding_seen = true;
                if let Some(existing) = req.headers.get_mut("Transfer-Encoding") {
                    *existing = format!("{}, {header_value}", existing.as_ref()).into();
                } else {
                    req.headers
                        .insert("Transfer-Encoding".into(), header_value.to_string().into());
                }
                continue;
            }
            let name_std = h.name.http_std_case();
            if req.headers.contains_key(name_std.as_str()) {
                // Repeated field (e.g. multiple Set-Cookie lines): keep every
                // value instead of letting the hash map overwrite the earlier
                // ones.
                req.multi_headers
                    .push((name_std.into(), header_value.to_string().into()));
            } else {
                req.headers.insert(
                    name_std.into(),
                    header_value.to_string().into(),
                );
            }
        }
        if transfer_encoding_seen && content_length_seen.is_some() {
            Err(anyhow!(
                "conflicting headers: Transfer-Encoding and Content-Length"
            ))?;
        }
        Ok(Some((req, n)))
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        let header_key = key.http_std_case();
        self.headers.get(header_key.as_str()).map(|a| a.as_ref())
    }
}

pub fn load_embed<T: Embed>() -> HashMap<String, Cow<'static, [u8]>> {
    let mut ret = HashMap::with_capacity(16);
    for name in T::iter() {
        if let Some(file) = T::get(&name) {
            if name.ends_with("index.htm") || name.ends_with("index.html") {
                if let Some(path) = Path::new(&name[..]).parent() {
                    if let Some(path) = path.to_str() {
                        ret.insert(path.to_string(), file.data.clone());
                    }
                }
            }
            ret.insert(name.to_string(), file.data);
        }
    }
    ret
}

/// WebTransport 客户端连接宏；仅用于实验性 feature。
///
/// 用于快速连接到 WebTransport 服务器。
///
/// # 示例
///
/// ```rust,no_run
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // 基本连接
///     let mut wt = potato::webtransport!("https://server.com/wt").await?;
///
///     let mut wt = potato::webtransport!("https://server.com/wt").await?;
///     Ok(())
/// }
/// ```
#[cfg(feature = "webtransport")]
#[macro_export]
macro_rules! webtransport {
    ($url:expr) => {
        $crate::WebTransport::connect($url)
    };
}

#[cfg(test)]
mod tests {
    use super::{CompressMode, HttpMethod, HttpRequest, HttpResponse};

    #[test]
    fn accept_encoding_supports_simple_gzip_token() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_supports_qvalue_for_gzip() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "br;q=1, gzip;q=0.3");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_uses_wildcard_when_gzip_not_listed() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "br;q=1, *;q=0.8");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);
    }

    #[test]
    fn accept_encoding_respects_explicit_gzip_zero_over_wildcard() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip;q=0, *;q=1");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
    }

    #[test]
    fn accept_encoding_ignores_invalid_qvalue() {
        let mut req = HttpRequest::new();
        req.set_header("Accept-Encoding", "gzip;q=xyz");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
    }

    #[test]
    fn request_parser_rejects_too_many_headers() {
        let mut raw = String::from("GET / HTTP/1.1\r\nHost: example.com\r\n");
        for i in 0..64 {
            raw.push_str(&format!("X-Header-{i}: value\r\n"));
        }
        raw.push_str("\r\n");

        let err = HttpRequest::from_headers_part(raw.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("too many request headers"));
    }

    #[test]
    fn request_parser_rejects_oversized_header_section() {
        let oversized = "a".repeat(20 * 1024);
        let raw = format!("GET / HTTP/1.1\r\nHost: example.com\r\nX-Large: {oversized}\r\n\r\n");

        let err = HttpRequest::from_headers_part(raw.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("request headers too large"));
    }

    #[test]
    fn request_serialization_includes_query_and_version() {
        let (mut req, _, _) =
            HttpRequest::from_url("http://example.com/search?q=rust", HttpMethod::GET).unwrap();
        req.version = 10;
        let serialized = String::from_utf8(req.as_bytes()).unwrap();

        assert!(serialized.starts_with("GET /search?q=rust HTTP/1.0\r\n"));
    }

    #[test]
    fn request_serialization_reencodes_decoded_query() {
        let (req, _, _) = HttpRequest::from_url(
            "http://example.com/datetime?start_time=2025-05-21+08%3A03%3A01&token=%2B",
            HttpMethod::GET,
        )
        .unwrap();
        let serialized = String::from_utf8(req.as_bytes()).unwrap();

        assert!(serialized.starts_with("GET /datetime?"));
        assert!(serialized.contains("start_time=2025-05-21+08%3A03%3A01"));
        assert!(serialized.contains("token=%2B"));
        assert!(serialized.contains(" HTTP/1.1\r\n"));
    }

    #[test]
    fn request_serialization_falls_back_to_http11_for_non_h1_versions() {
        let (mut req, _, _) =
            HttpRequest::from_url("http://example.com/", HttpMethod::GET).unwrap();
        req.version = 20;
        let serialized = String::from_utf8(req.as_bytes()).unwrap();

        assert!(serialized.starts_with("GET / HTTP/1.1\r\n"));
    }

    #[test]
    fn from_url_host_header_keeps_non_default_port() {
        let (req, _, _) =
            HttpRequest::from_url("http://example.com:8080/demo", HttpMethod::GET).unwrap();
        assert_eq!(req.get_header("Host"), Some("example.com:8080"));
    }

    #[test]
    fn response_parser_rejects_conflicting_duplicate_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n";
        assert!(HttpResponse::from_headers_part(raw).is_err());
    }

    #[test]
    fn response_parser_rejects_transfer_encoding_content_length_conflict() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n";
        assert!(HttpResponse::from_headers_part(raw).is_err());
    }

    #[test]
    fn response_transfer_encoding_supports_case_insensitive_lists() {
        assert!(HttpResponse::transfer_encoding_has_chunked("gzip, Chunked").unwrap());
    }

    #[test]
    fn response_data_serialization_omits_transfer_encoding() {
        let mut res = HttpResponse::text("hello");
        res.add_header("Transfer-Encoding".into(), "chunked".into());

        let serialized = String::from_utf8(res.as_bytes(CompressMode::None)).unwrap();
        assert!(serialized.contains("Content-Length: 5\r\n"));
        assert!(!serialized.contains("Transfer-Encoding:"));
    }
}
