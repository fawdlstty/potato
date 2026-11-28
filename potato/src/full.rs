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

/// gRPC / gRPC-Web 运行时（feature `grpc`；生成代码只依赖本模块的
/// 运行时类型，不依赖 potato-build 本体）
#[cfg(feature = "grpc")]
pub mod grpc;

pub use client::*;
pub use global_config::*;
pub use hipstr;
pub use inventory;
pub use potato_macro::*;
pub use regex;
pub use rust_embed;
pub use serde;
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
use crate::utils::header_store::{RequestHeaders, ResponseHeaders};
use sha1::{Digest, Sha1};
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::{Cell, UnsafeCell};
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

/// 无符号整数转 ASCII 数字串（写入 tmp 并返回数字段）。
/// 输出与 `u64` 的 Display 恒等，供热路径序列化绕开 fmt 机制开销。
fn u64_ascii_str(v: u64, tmp: &mut [u8; 20]) -> &str {
    let mut i = tmp.len();
    let mut v = v;
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    std::str::from_utf8(&tmp[i..]).expect("u64 ascii digits are valid utf8")
}

/// 热路径头行序列化宏（`Vec<u8>` 缓冲版，键值均为 `&str`，动态键）：
/// `sanitize(k) + ": " + sanitize(v) + CRLF` 直写，与
/// `write!("{}: {}\r\n", sanitize_header_str(k), sanitize_header_str(v))`
/// 字节形态一致。
macro_rules! push_header_kv_bytes {
    ($ret:expr, $key:expr, $value:expr) => {{
        let __k = sanitize_header_str($key);
        $ret.extend_from_slice(__k.as_bytes());
        $ret.extend_from_slice(b": ");
        let __v = sanitize_header_str($value);
        $ret.extend_from_slice(__v.as_bytes());
        $ret.extend_from_slice(b"\r\n");
    }};
}

/// URI 合法字符表：条目为 0 表示非法，否则为字节自身。
/// 移植自 http-1.5.0 `uri::URI_CHARS`（判定语义一致；>127 全非法，
/// 保证合法条目段必为合法单字节 UTF-8）。
#[rustfmt::skip]
const URI_CHARS: [u8; 256] = [
    //  0      1      2      3      4      5      6      7      8      9
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, //   x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, //  1x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, //  2x
        0,     0,     0,  b'!',     0,  b'#',  b'$',     0,  b'&', b'\'', //  3x
     b'(',  b')',  b'*',  b'+',  b',',  b'-',  b'.',  b'/',  b'0',  b'1', //  4x
     b'2',  b'3',  b'4',  b'5',  b'6',  b'7',  b'8',  b'9',  b':',  b';', //  5x
        0,  b'=',     0,  b'?',  b'@',  b'A',  b'B',  b'C',  b'D',  b'E', //  6x
     b'F',  b'G',  b'H',  b'I',  b'J',  b'K',  b'L',  b'M',  b'N',  b'O', //  7x
     b'P',  b'Q',  b'R',  b'S',  b'T',  b'U',  b'V',  b'W',  b'X',  b'Y', //  8x
     b'Z',  b'[',     0,  b']',     0,  b'_',     0,  b'a',  b'b',  b'c', //  9x
     b'd',  b'e',  b'f',  b'g',  b'h',  b'i',  b'j',  b'k',  b'l',  b'm', // 10x
     b'n',  b'o',  b'p',  b'q',  b'r',  b's',  b't',  b'u',  b'v',  b'w', // 11x
     b'x',  b'y',  b'z',     0,     0,     0,  b'~',     0,     0,     0, // 12x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 13x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 14x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 15x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 16x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 17x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 18x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 19x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 20x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 21x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 22x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 23x
        0,     0,     0,     0,     0,     0,     0,     0,     0,     0, // 24x
        0,     0,     0,     0,     0,     0                              // 25x
];

/// Host 头 authority 语法校验（仅 accept/reject 判定）。
/// 与 `http::uri::Authority::from_str(..).is_ok()` 判定逐分支等价：
/// http 的 `TryFrom<&[u8]>` 校验通过后会 `Bytes::copy_from_slice` 拷贝一份
/// （depth=1 每请求一次堆分配 + 拷贝 + 释放），校验-only 场景纯浪费，
/// 故移植 `validate_authority_bytes` + `create_authority` 的截断拒绝规则：
/// 首个 `/`、`?`、`#` 处截断后要求恰好消费全部输入。
fn validate_authority_str(s: &str) -> bool {
    let s = s.as_bytes();
    if s.is_empty() {
        return false;
    }

    let mut colon_cnt: u32 = 0;
    let mut start_bracket = false;
    let mut end_bracket = false;
    let mut has_percent = false;
    let mut end = s.len();
    let mut at_sign_pos: usize = s.len();
    const MAX_COLONS: u32 = 8; // e.g., [FEDC:BA98:7654:3210:FEDC:BA98:7654:3210]:80

    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        let ch = URI_CHARS[b as usize];

        if ch == b'/' || ch == b'?' || ch == b'#' {
            end = i;
            break;
        }

        if ch == 0 {
            if b == b'%' {
                // userinfo/zone-id 的百分号编码：仅在它属于 userinfo 时合法，
                // http 同款规则——末尾统一按 has_percent 拒绝（除非已被
                // `]`/`@` 清零）
                has_percent = true;
            } else {
                return false;
            }
        } else if ch == b':' {
            if colon_cnt >= MAX_COLONS {
                return false;
            }
            colon_cnt += 1;
        } else if ch == b'[' {
            if has_percent || start_bracket {
                return false;
            }
            start_bracket = true;
        } else if ch == b']' {
            if !start_bracket || end_bracket {
                return false;
            }
            end_bracket = true;
            // IPv6 字面量内部的冒号/百分号不算数
            colon_cnt = 0;
            has_percent = false;
        } else if ch == b'@' {
            at_sign_pos = i;
            // userinfo 的冒号/百分号不算数
            colon_cnt = 0;
            has_percent = false;
        }

        i += 1;
    }

    if start_bracket != end_bracket {
        return false;
    }
    if colon_cnt > 1 {
        // 'localhost:8080:3030' 之类拒绝
        return false;
    }
    if end > 0 && at_sign_pos == end - 1 {
        // `@` 后无内容
        return false;
    }
    if has_percent {
        // host 段（非 userinfo/IPv6 zone）出现 `%`
        return false;
    }
    // create_authority：截断点未消费到串尾即拒绝（"host/path" 形态）
    if end != s.len() {
        return false;
    }
    true
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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

// SAFETY: 字段为 hipstr Rc 后端句柄（非原子引用计数，本身 !Send）。框架内部
// 只对值整体 move（move 期间无引用计数操作），不做跨线程 clone/drop。注意
// 本类型带 derive(Clone)：用户若跨线程克隆句柄并与其他线程并发 clone/drop
// 同一 Rc 分配，会破坏引用计数（UB），此约定无法由框架强制，须由用户保证。
unsafe impl Send for PostFile {}

pub struct HttpRequest {
    pub method: HttpMethod,
    pub target_form: HttpRequestTargetForm,
    pub url_path: LocalHipStr<'static>,
    pub url_query: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    /// 原始查询串（`?` 后的未解码原文），无查询时为 None。
    /// url_query 是按参数名折叠的 HashMap，识别重复参数名/原始转义需用 raw_query()
    pub url_query_raw: Option<LocalHipStr<'static>>,
    /// 动态路由 `{name}` 段捕获值：(参数名, url_path 起始偏移, 结束偏移)，
    /// 区间指向原始（未解码）路径字节，解码延迟到取值时按需进行
    pub url_path_params: Vec<(&'static str, u32, u32)>,
    pub version: u8,
    pub headers: RequestHeaders,
    pub trailers: RequestHeaders,
    pub body: LocalHipByt<'static>,
    pub body_pairs: HashMap<LocalHipStr<'static>, LocalHipStr<'static>>,
    pub body_files: HashMap<LocalHipStr<'static>, PostFile>,
    /// multipart 重复上传的字段名（同名真实文件段出现多次时记录，HashMap 折叠前捕获）
    pub body_dup_files: Vec<LocalHipStr<'static>>,
    /// multipart 中 filename="" 的未选中文件输入的字段名（此类段同时按原语义落入 body_pairs）
    pub body_unselected_files: Vec<LocalHipStr<'static>>,
    /// multipart 存在头残缺段（缺 \r\n\r\n 头结束符，无法按部件解析）
    pub body_malformed_multipart: bool,
    pub client_addr: Option<SocketAddr>,
    pub exts: HashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>,
    stream: Option<Arc<Mutex<HttpStream>>>,
    /// 解析期特判头缓存（仅 `from_headers_part_with_limits_into` 在解析
    /// 头表时填充，`clear_for_reuse` / 构造器重置为 None）：d1/d8 热路径
    /// 的 5 次特判头线性扫描降为 0 次。None = 未缓存（手动构造的请求、
    /// 或解析遇到非精确覆盖的边界值——多段 TE/Expect、CL 溢出等），
    /// getter 回落头表扫描，语义与原实现一致。值语义与头表同步：
    /// 头表 insert 为同名覆盖（last-wins），缓存条目在解析循环内同样
    /// 按最后出现的值覆盖；解析后改动头表不在缓存跟踪范围内（框架内
    /// 解析→取值链路无中间改动）。
    cache_accept_encoding: Option<CompressMode>,
    cache_connection: Option<HttpConnection>,
    /// CL 解析值（缺席=Some(0)）；溢出等无法精确复刻 parse 语义的值记
    /// None 回落（回落路径照常产出原错误）
    cache_content_length: Option<usize>,
    /// TE 单段 "chunked" 判定（缺席=Some(false)）；多段/未认知值记 None
    /// 回落完整校验（照常产出 unsupported/empty 等原错误）
    cache_transfer_encoding: Option<bool>,
    /// Expect 单段 "100-continue" 判定（缺席=Some(false)）；其余值记 None
    /// 回落完整校验（照常产出原错误）
    cache_expect: Option<bool>,
}

// SAFETY: 本类型含 LocalHipStr<'static>/LocalHipByt<'static>（hipstr Rc 后端，
// 非原子引用计数，本身 !Send），此断言承重。安全性依赖不变量：同一 Rc 分配
// 的两个句柄不得在不同线程并发 clone/drop。框架侧已保证——请求只整体 move
//（move 期间无引用计数操作），无 Clone 实现，字段克隆全部发生在单个请求的
// 单线程作用域内（解析、handler 调用、响应生成）；tokio 多线程调度器下任务
// 线程迁移属纯 move，不违规。字段均为 pub：用户若把克隆出的字段句柄 move 进
// 其他并发任务并与原请求的存活期重叠，即可违反不变量（UB），须由用户保证。
unsafe impl Send for HttpRequest {}

// SAFETY: 承重断言，不可删——server 会 spawn 持 `&HttpRequest` 跨 await 的
// future（如 from_static_file），`&T: Send` 要求 `T: Sync`。安全性依赖与
// Send 相同的不变量：框架侧同一时刻只有单一 future 持引用，不存在两个线程
// 经不同引用并发 clone 字段的路径；用户若把 `&req` 复制进多个任务并并发
// clone Rc 句柄字段，即可违反不变量（UB），须由用户保证。
unsafe impl Sync for HttpRequest {}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("target_form", &self.target_form)
            .field("url_path", &self.url_path)
            .field("url_query", &self.url_query)
            .field("url_query_raw", &self.url_query_raw)
            .field("url_path_params", &self.url_path_params)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .field("body", &self.body)
            .field("body_pairs", &self.body_pairs)
            .field("body_files", &self.body_files)
            .field("body_dup_files", &self.body_dup_files)
            .field("body_unselected_files", &self.body_unselected_files)
            .field("body_malformed_multipart", &self.body_malformed_multipart)
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
        batch: &mut Option<&mut crate::server::ResponseBatcher>,
    ) -> anyhow::Result<()> {
        // 解析期缓存命中（单段 100-continue / 缺席）；其余值回落完整校验
        //（照常产出 empty/unsupported/版本错误，has_100_continue 判定一致）
        let has_100_continue = match self.cache_expect {
            Some(v) => v,
            None => {
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
                has_100_continue
            }
        };
        if has_100_continue && has_request_body {
            // 100-Continue 插写前冲刷（插写是写操作，批内响应必须先落盘，
            // 顺序不能乱）
            Self::flush_pending_before_read(batch, stream).await?;
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
        self.url_query_raw = None;
        self.url_path_params.clear();
        self.version = 11;
        self.headers.clear();
        self.trailers.clear();
        self.body = LocalHipByt::new();
        self.body_pairs.clear();
        self.body_files.clear();
        self.body_dup_files.clear();
        self.body_unselected_files.clear();
        self.body_malformed_multipart = false;
        self.client_addr = None;
        self.exts.clear();
        self.stream = None;
        self.cache_accept_encoding = None;
        self.cache_connection = None;
        self.cache_content_length = None;
        self.cache_transfer_encoding = None;
        self.cache_expect = None;
    }

    fn with_header_capacity(header_capacity: usize) -> Self {
        Self {
            method: HttpMethod::GET,
            target_form: HttpRequestTargetForm::Origin,
            url_path: LocalHipStr::from("/"),
            url_query: HashMap::new(),
            url_query_raw: None,
            url_path_params: Vec::with_capacity(4),
            version: 11,
            headers: RequestHeaders::with_capacity(header_capacity),
            trailers: RequestHeaders::new(),
            body: LocalHipByt::new(),
            body_pairs: HashMap::new(),
            body_files: HashMap::new(),
            body_dup_files: Vec::new(),
            body_unselected_files: Vec::new(),
            body_malformed_multipart: false,
            client_addr: None,
            exts: HashMap::new(),
            stream: None,
            cache_accept_encoding: None,
            cache_connection: None,
            cache_content_length: None,
            cache_transfer_encoding: None,
            cache_expect: None,
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

    /// 原始查询串：`?` 之后、未解码的原文；无查询串时返回 None（`?` 单独出现
    /// 返回 Some("")）。url_query 已按参数名折叠进 HashMap，同名参数（含
    /// %xx 转义后同名的）会互相覆盖，无法识别重复参数名；需要区分重复、
    /// 顺序或自行控制转义语义时用本方法取原文。
    pub fn raw_query(&self) -> Option<&str> {
        self.url_query_raw.as_ref().map(|s| &s[..])
    }

    /// 取动态路由 `{name}` 段的解码值。
    /// 无 %xx 转义时零拷贝借用 url_path 切片，有转义才分配解码；
    /// path 段语义中 '+' 是字面量，不做空格转换。
    pub fn path_param(&self, name: &str) -> Option<std::borrow::Cow<'_, str>> {
        let (_, start, end) = self
            .url_path_params
            .iter()
            .find(|(param_name, _, _)| *param_name == name)?;
        let seg = &self.url_path[..][*start as usize..*end as usize];
        if seg.as_bytes().contains(&b'%') {
            Some(std::borrow::Cow::Owned(Self::decode_path_segment(seg)))
        } else {
            Some(std::borrow::Cow::Borrowed(seg))
        }
    }

    /// 仅解码 %xx 转义；非法转义原样保留，非法 UTF-8 走有损替换
    fn decode_path_segment(seg: &str) -> String {
        let bytes = seg.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4 | lo) as u8);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn get_ext<T: Any + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.exts
            .get(&TypeId::of::<T>())
            .and_then(|any| any.clone().downcast().ok())
    }

    fn parse_path_and_query(&mut self, target: &str) {
        self.url_query.clear();
        self.url_query_raw = None;
        match target.find('?') {
            Some(p) => {
                self.url_path = LocalHipStr::from(&target[..p]);
                self.url_query_raw = Some(LocalHipStr::from(&target[p + 1..]));
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
            self.url_query_raw = None;
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
            LocalHipStr::from(
                uri.authority()
                    .map(|authority| authority.as_str())
                    .unwrap_or("localhost"),
            ),
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
        self.headers.get_by_str(key).map(|v| &v[..])
    }

    pub fn get_header_key(&self, key: HeaderItem) -> Option<&str> {
        self.headers.get_header_item(key).map(|v| &v[..])
    }

    pub fn set_trailer(
        &mut self,
        key: impl Into<HeaderOrHipStr>,
        value: impl Into<LocalHipStr<'static>>,
    ) {
        self.trailers.insert(key.into(), value.into());
    }

    pub fn get_trailer(&self, key: &str) -> Option<&str> {
        self.trailers.get_by_str(key).map(|v| &v[..])
    }

    pub fn get_header_accept_encoding(&self) -> CompressMode {
        // 解析期缓存命中（d1/d8 热路径）：免头表扫描 + negotiate 开销
        if let Some(mode) = self.cache_accept_encoding {
            return mode;
        }
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
        // 解析期缓存命中（d1/d8 热路径，is_websocket 亦经此）：免头表扫描
        if let Some(conn) = self.cache_connection.clone() {
            return conn;
        }
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
        // 解析期缓存命中：Some(0) 表示缺席（与「存 0」下游取值同为 0 等价）
        if let Some(cl) = self.cache_content_length {
            return Ok(if cl == 0 { None } else { Some(cl) });
        }
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
        // 解析期缓存命中（单段 chunked / 缺席）；其余值回落完整校验
        if let Some(chunked) = self.cache_transfer_encoding {
            return Ok(chunked);
        }
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
        RequestHeaders,
        usize,
    )> {
        let mut cursor = hdr_len;
        let mut body = Vec::new();
        let mut trailers = RequestHeaders::with_capacity(4);
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
                val.len() == 24
                    && val.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
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
            stream
                .write_all(&res.as_bytes(CompressMode::None, None))
                .await?;
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

    /// 解析读路径的冲刷点：在任何可能阻塞的 socket 读、或 100-Continue
    /// 插写之前冲刷批内待发响应（写序安全红线：响应滞留至阻塞读之后会
    /// 死锁——客户端发大请求体等响应先到）。无批（手动构造的解析入口
    /// 传 None）或批空时零成本跳过。
    async fn flush_pending_before_read(
        batch: &mut Option<&mut crate::server::ResponseBatcher>,
        stream: &mut HttpStream,
    ) -> anyhow::Result<()> {
        if let Some(batch) = batch {
            if batch.has_pending() {
                batch
                    .flush(stream)
                    .await
                    .map_err(|e| anyhow!("flush pending responses before read: {e:#}"))?;
            }
        }
        Ok(())
    }

    async fn from_stream_impl_into(
        cursor: &mut HttpBufCursor<'_>,
        stream: &mut HttpStream,
        limits: (Option<usize>, Option<usize>),
        req: &mut HttpRequest,
        mut batch: Option<&mut crate::server::ResponseBatcher>,
        slots: &mut [httparse::Header<'static>],
    ) -> anyhow::Result<usize> {
        let max_header_bytes = limits
            .0
            .map(|limit| limit.min(ServerConfig::get_max_header_bytes()))
            .unwrap_or_else(ServerConfig::get_max_header_bytes);
        let hdr_len = loop {
            // 游标语义：buf[read_pos..] 是未消费请求字节（d8 流水线残留），
            // 解析与切片全部相对 read_pos；消费量以本函数返回值表达，由调用
            // 方推进游标（水位 compact 见连接循环），免除每请求 copy_within
            // memmove。空游标时解析必然 Partial（httparse 对空输入不产出请
            // 求行），跳过解析直接读一轮。
            if cursor.len() > cursor.pos() {
                if let Some(hdr_len) =
                    HttpRequest::from_headers_part_with_limits_into(cursor.slice_from_pos(), max_header_bytes, req, slots)?
                {
                    break hdr_len;
                }
            }
            // 阻塞读前冲刷（先解析后 flush 重构）：走到这里即 buf 不含完整
            // 请求头、下一步 read 必阻塞，先冲掉批内待发响应。buf 已含完整
            // 请求时解析直接成功——零冲刷零探测，取代原连接循环顶的
            // buf_holds_complete_request 探测 + 完整 httparse 扫描。
            Self::flush_pending_before_read(&mut batch, stream).await?;
            // 直读 buf 备用容量（read_into_vec 零拷贝零清零）：省掉 4KB 栈
            // 缓冲清零与请求字节的中转 memcpy；keep-alive 稳态 buf 恒有
            // ≥3KB 备用容量，单次 read 系统调用语义不变。
            let n = stream.read_into_vec(cursor.buf_mut()).await?;
            if n == 0 {
                return Err(anyhow::Error::msg("connection closed"));
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
        req.process_expect_header(stream, has_request_body, &mut batch)
            .await?;

        let bdy_len = if has_chunked_transfer_encoding {
            // chunked 体/尾读取循环可能阻塞：先冲刷（Expect 插写场景上一句
            // 已冲过，此处 has_pending=false 直接跳过）
            Self::flush_pending_before_read(&mut batch, stream).await?;
            let allowed_trailers =
                parse_declared_trailer_names(req.get_header_key(HeaderItem::Trailer));
            let chunk_start = cursor.pos() + hdr_len;
            let (body, trailers, consumed_len) = Self::read_chunked_body(
                cursor.buf_mut(),
                stream,
                chunk_start,
                &allowed_trailers,
                Some(max_body_bytes),
            )
            .await?;
            req.body = body;
            req.trailers = trailers;
            consumed_len
        } else {
            if cursor.pos() + hdr_len + content_length > cursor.len() {
                // body 不足需阻塞补读：先冲刷（同上，写序安全红线）
                Self::flush_pending_before_read(&mut batch, stream).await?;
            }
            while cursor.pos() + hdr_len + content_length > cursor.len() {
                let t = stream.read_into_vec(cursor.buf_mut()).await?;
                if t == 0 {
                    return Err(anyhow::Error::msg("connection closed"));
                }
            }
            if content_length > 0 {
                let body_start = cursor.pos() + hdr_len;
                req.body =
                    LocalHipByt::from(&cursor.buf()[body_start..body_start + content_length]);
            }
            content_length
        };

        // 按 Content-Type 解析 body 到 body_pairs/body_files。
        // 无 body 的请求（d1 keep-alive GET 常态）直接跳过：空体解析是
        // 纯 no-op（json/form/multipart 对空输入均不产出条目），
        // 省去每请求一次 Content-Type 头查找。
        if has_request_body {
            Self::parse_body_content(req);
        }
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
                            // 前导区（首边界前的空段）为空属正常形态；非空段
                            // 缺头结束符即残缺部件，打标供上层判 400
                            if !seg.is_empty() {
                                req.body_malformed_multipart = true;
                            }
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
                        let mut filename_empty = false;
                        for key in keys.into_iter() {
                            if let Some((k, v)) = key.trim().split_once('=') {
                                let v = strip_quoted(v.trim());
                                if v.is_empty() {
                                    // filename="" 表示未选中的文件输入，与
                                    // 无 filename 的文本字段语义不同，单独记录
                                    if k == "filename" {
                                        filename_empty = true;
                                    }
                                    continue;
                                }
                                if k == "name" {
                                    name = Some(LocalHipStr::from(v));
                                } else if k == "filename" {
                                    filename = Some(LocalHipStr::from(v));
                                }
                            }
                        }
                        if let Some(name) = &name {
                            if let Some(filename) = filename {
                                let data = LocalHipByt::from(content.to_vec());
                                if req.body_files.contains_key(name) {
                                    req.body_dup_files.push(name.clone());
                                }
                                req.body_files
                                    .insert(name.clone(), PostFile { filename, data });
                            } else {
                                let text = String::from_utf8_lossy(content).to_string();
                                req.body_pairs.insert(name.clone(), LocalHipStr::from(text));
                                if filename_empty {
                                    req.body_unselected_files.push(name.clone());
                                }
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
        let mut slots = [httparse::EMPTY_HEADER; 48];
        let mut cursor = HttpBufCursor::new(buf);
        let consumed =
            Self::from_stream_impl_into(&mut cursor, &mut stream, (None, None), &mut req, None, &mut slots)
                .await?;
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
        let mut slots = [httparse::EMPTY_HEADER; 48];
        let mut cursor = HttpBufCursor::new(buf);
        let consumed = Self::from_stream_impl_into(
            &mut cursor,
            &mut stream,
            (max_header_bytes, max_body_bytes),
            &mut req,
            None,
            &mut slots,
        )
        .await?;
        Ok((req, consumed))
    }

    pub(crate) async fn from_stream_with_limits_into(
        cursor: &mut HttpBufCursor<'_>,
        stream: &mut HttpStream,
        max_header_bytes: Option<usize>,
        max_body_bytes: Option<usize>,
        req: &mut HttpRequest,
        batch: Option<&mut crate::server::ResponseBatcher>,
        slots: &mut [httparse::Header<'static>],
    ) -> anyhow::Result<usize> {
        Self::from_stream_impl_into(
            cursor,
            stream,
            (max_header_bytes, max_body_bytes),
            req,
            batch,
            slots,
        )
        .await
    }

    pub fn from_headers_part(buf: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        Self::from_headers_part_with_limits(buf, ServerConfig::get_max_header_bytes())
    }

    fn from_headers_part_with_limits_into(
        buf: &[u8],
        max_header_bytes: usize,
        req: &mut HttpRequest,
        slots: &mut [httparse::Header<'static>],
    ) -> anyhow::Result<Option<usize>> {
        let max_header_count = ServerConfig::get_max_header_count();
        Self::ensure_header_section_size(buf, max_header_bytes)?;

        // 头槽复用（连接循环级）：httparse 1.10 官方保证解析内部把槽按
        // MaybeUninit 对待——旧值从不被读、解析后切片收缩到本次写入数
        // （ShrinkOnDrop），因此跨请求复用脏槽安全且免去每请求 ~1.5KB 栈
        // 初始化。头计数改用收缩后的切片长度（take_while name 非空在脏槽
        // 下会误判）。
        // SAFETY（协变缩短）：Header<'static> 是 Header<'buf> 的子类型（生命
        // 周期参数为纯类型级标注，值表示不变），此 transmute 只把槽元素类型
        // 从 'static 缩短到本次 buf 借用期；解析期间槽唯一可变借用归解析器
        // 所有（写不读），解析器归还后即不可达。与 ResponseBatcher::flush
        // 的 IoSlice transmute 同款先例。
        let slots: &mut [httparse::Header<'_>] = unsafe {
            std::mem::transmute::<
                &mut [httparse::Header<'static>],
                &mut [httparse::Header<'_>],
            >(slots)
        };
        let mut heap_headers: Vec<httparse::Header<'_>>;
        let headers: &mut [httparse::Header<'_>] = if max_header_count <= slots.len() {
            slots
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
        let parsed_header_count = rreq.headers.len();
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
        // 特判头缓存缺席默认值（version 已定，头循环内命中即按 last-wins
        // 覆盖；见 cache_* 字段注释）。缺席结果与 getter 原语义一致：
        // AE→negotiate("")、Connection→按 version、CL→0、TE/Expect→false
        req.cache_accept_encoding = Some(CompressMode::None);
        req.cache_connection = Some(if req.version >= 11 {
            HttpConnection::KeepAlive
        } else {
            HttpConnection::Close
        });
        req.cache_content_length = Some(0);
        req.cache_transfer_encoding = Some(false);
        req.cache_expect = Some(false);
        for h in rreq.headers.iter() {
            if h.name.is_empty() {
                break;
            }
            let header_value = str::from_utf8(h.value)?;
            let normalized_header_value = header_value.trim();
            // 特判头单次判别值分派（d1 热路径）：from_str 内部已完成一次
            // (首字符,长度) 预筛 + 大小写不敏感全名校验，据其判别值分派，
            // 免去此前每头最多 4 次大小写不敏感字符串比较。命中集合与原
            // eq_ignore_ascii_case 四连判完全一致（try_from_str 同为
            // eq_ignore_ascii_case 全名校验）。
            enum SpecialHeader {
                ContentLength,
                TransferEncoding,
                Host,
                Expect,
                AcceptEncoding,
                Connection,
                Other,
            }
            let key = HeaderOrHipStr::from_str(h.name);
            let special = match &key {
                HeaderOrHipStr::HeaderItem(HeaderItem::Content_Length) => {
                    SpecialHeader::ContentLength
                }
                HeaderOrHipStr::HeaderItem(HeaderItem::Transfer_Encoding) => {
                    SpecialHeader::TransferEncoding
                }
                HeaderOrHipStr::HeaderItem(HeaderItem::Host) => SpecialHeader::Host,
                HeaderOrHipStr::HeaderItem(HeaderItem::Expect) => SpecialHeader::Expect,
                HeaderOrHipStr::HeaderItem(HeaderItem::Accept_Encoding) => {
                    SpecialHeader::AcceptEncoding
                }
                HeaderOrHipStr::HeaderItem(HeaderItem::Connection) => SpecialHeader::Connection,
                _ => SpecialHeader::Other,
            };
            match special {
                SpecialHeader::ContentLength => {
                    let cl = normalized_header_value;
                    if cl.is_empty() {
                        Err(anyhow!("empty Content-Length header"))?;
                    }
                    // RFC 9112: Content-Length 值必须为纯数字。符号/溢出值在头
                    // 解析阶段就拒绝，避免被下游按不同口径解读（请求走私向量）。
                    if !cl.bytes().all(|b| b.is_ascii_digit()) {
                        Err(anyhow!("invalid Content-Length header"))?;
                    }
                    if let Some(prev) = &content_length_seen {
                        if prev != cl {
                            Err(anyhow!("conflicting duplicate Content-Length headers"))?;
                        }
                    } else {
                        content_length_seen = Some(cl.to_string());
                    }
                    // 解析期缓存（值已在手上免二次扫描）；溢出值记 None 回落，
                    // 由 parse_header_content_length 照常产出原 parse 错误
                    req.cache_content_length = cl.parse::<usize>().ok();
                    // 校验通过后照常入库：body 读取（parse_header_content_length）
                    // 与下游业务都按存储里的 Content-Length 取值
                    req.headers.insert(key, LocalHipStr::from(cl));
                }
                SpecialHeader::TransferEncoding => {
                    // RFC 9112: a recipient MUST reject messages containing more
                    // than one Transfer-Encoding header field (request smuggling).
                    if transfer_encoding_seen {
                        anyhow::bail!("duplicate Transfer-Encoding headers are not allowed");
                    }
                    if normalized_header_value.is_empty() {
                        anyhow::bail!("empty Transfer-Encoding header");
                    }
                    transfer_encoding_seen = true;
                    // 解析期缓存：仅「单段 chunked」精确命中（与
                    // has_chunked_transfer_encoding 的 split/trim/filter 判定
                    // 等价）；多段/其他值记 None 回落完整校验（照常产出
                    // unsupported/empty 等原错误）
                    let mut codings = normalized_header_value
                        .split(',')
                        .map(|part| part.trim())
                        .filter(|part| !part.is_empty());
                    req.cache_transfer_encoding = match codings.next() {
                        Some(c) if c.eq_ignore_ascii_case("chunked") && codings.next().is_none() => {
                            Some(true)
                        }
                        _ => None,
                    };
                    // 校验通过后入库：has_chunked_transfer_encoding 按存储取值
                    req.headers
                        .insert(key, LocalHipStr::from(normalized_header_value));
                }
                SpecialHeader::Host => {
                    host_header_count += 1;
                    if host_header_count > 1 {
                        anyhow::bail!("multiple Host headers are not allowed");
                    }
                    if normalized_header_value.is_empty() {
                        anyhow::bail!("empty Host header");
                    }
                    if !validate_authority_str(normalized_header_value) {
                        anyhow::bail!("invalid Host header");
                    }
                    has_valid_host = true;
                    // 校验通过后入库：get_header_host / 反代等下游按存储取值
                    req.headers
                        .insert(key, LocalHipStr::from(normalized_header_value));
                }
                SpecialHeader::Expect => {
                    if let Some(existing) = req.headers.get(&key) {
                        // 多段合并：合成值语义需完整分词校验，缓存记 None 回落
                        req.cache_expect = None;
                        req.headers.insert(
                            key,
                            LocalHipStr::from(format!(
                                "{}, {normalized_header_value}",
                                existing.as_str()
                            )),
                        );
                    } else {
                        // 解析期缓存：仅「单段 100-continue」精确命中；其他值、
                        // version<1.1 记 None 回落完整校验（照常产出
                        // unsupported/empty/版本错误）
                        req.cache_expect = if req.version >= 11
                            && normalized_header_value.eq_ignore_ascii_case("100-continue")
                        {
                            Some(true)
                        } else {
                            None
                        };
                        req.headers
                            .insert(key, LocalHipStr::from(normalized_header_value));
                    }
                }
                SpecialHeader::AcceptEncoding => {
                    // 解析期缓存（negotiate 结果，缺席默认 None 模式），重复头
                    // last-wins 与头表一致
                    req.cache_accept_encoding =
                        Some(Self::negotiate_accept_encoding(normalized_header_value));
                    req.headers.insert(key, LocalHipStr::from(normalized_header_value));
                }
                SpecialHeader::Connection => {
                    // 解析期缓存（from_str 判别 + Close 兜底，与 getter 一致），
                    // 重复头 last-wins 与头表一致
                    req.cache_connection = Some(
                        HttpConnection::from_str(normalized_header_value)
                            .unwrap_or(HttpConnection::Close),
                    );
                    req.headers.insert(key, LocalHipStr::from(normalized_header_value));
                }
                SpecialHeader::Other => {
                    req.headers.insert(key, LocalHipStr::from(normalized_header_value));
                }
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
        let mut slots = [httparse::EMPTY_HEADER; 48];
        let consumed =
            Self::from_headers_part_with_limits_into(buf, max_header_bytes, &mut req, &mut slots)?;
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
/// 每线程 Date 头固定槽（见 `HttpResponse::default_headers`）。
/// 线程本地泄漏一份（Box::leak，容量有界），秒切换时整体重写缓冲内容；
/// 借用安全性依赖 UnsafeCell 内部可变性契约，详见 default_headers 注释。
struct DateSlot {
    ts: Cell<i64>,
    buf: UnsafeCell<[u8; 32]>,
    len: Cell<usize>,
}
// Safety: DateSlot 只经 ThreadLocal 从属线程访问（ThreadLocal 以 &'static
// 共享故要求 Sync）；内部可变性由线程亲和保证无并发，且 DateSlot 无 Drop。
unsafe impl Sync for DateSlot {}
impl DateSlot {
    fn new() -> Self {
        Self {
            ts: Cell::new(i64::MIN),
            buf: UnsafeCell::new([0u8; 32]),
            len: Cell::new(0),
        }
    }
    fn timestamp(&self) -> i64 {
        self.ts.get()
    }
    fn write(&self, ts: i64) {
        let s = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let bytes = s.as_bytes();
        assert!(bytes.len() <= 32, "HTTP date exceeds fixed date slot");
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.buf.get() as *mut u8, bytes.len());
        }
        self.len.set(bytes.len());
        self.ts.set(ts);
    }
    /// 返回槽内容的 &'static str：指向启动期泄漏的槽内存，内容随秒切换
    /// 原地更新（槽恒为合法 UTF-8，长度恒 ≤ 32）。
    fn as_str(&self) -> &'static str {
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                (*self.buf.get()).as_ptr(),
                self.len.get(),
            ))
        }
    }
}

/// 每线程 Date 固定槽：秒切换时原地重写，同秒内全部响应共享同一 &'static str。
fn thread_date_str(current_ts: i64) -> &'static str {
    static TL_DATE_SLOT: ThreadLocal<&'static DateSlot> = ThreadLocal::new();
    let slot: &'static DateSlot = TL_DATE_SLOT.get_or(|| Box::leak(Box::new(DateSlot::new())));
    if current_ts != slot.timestamp() {
        slot.write(current_ts);
    }
    slot.as_str()
}

/// 连接级游标缓冲：buf[pos..] 为未消费请求字节（d8 流水线残留）。
/// `advance` 统一消费推进——残留耗尽 clear（零 memmove）、超水位 drain
/// 一次搬移——替代旧版每请求 copy_within 的逐请求 memmove。
pub(crate) struct HttpBufCursor<'a> {
    buf: &'a mut Vec<u8>,
    pos: usize,
}

/// 游标 compact 水位：未消费残留超过该值才做一次 drain 搬移
/// （d8 深流水线残留一般 <2KB，常态零 memmove；水位同时约束 buf 尺寸上限）
const BUF_COMPACT_WATERMARK: usize = 16 * 1024;

impl<'a> HttpBufCursor<'a> {
    pub(crate) fn new(buf: &'a mut Vec<u8>) -> Self {
        Self { buf, pos: 0 }
    }

    fn pos(&self) -> usize {
        self.pos
    }

    fn len(&self) -> usize {
        self.buf.len()
    }

    /// 消费 consumed 字节后是否残留耗尽（direct_write 判定）
    fn consumed_to_end(&self, consumed: usize) -> bool {
        self.pos + consumed >= self.len()
    }

    fn buf(&self) -> &Vec<u8> {
        self.buf
    }

    fn buf_mut(&mut self) -> &mut Vec<u8> {
        self.buf
    }

    fn slice_from_pos(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// 推进游标（consumed=0 时零动作）；残留耗尽 clear 保容量，超水位 drain
    fn advance(&mut self, consumed: usize) {
        if consumed == 0 {
            return;
        }
        self.pos += consumed;
        if self.pos >= self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        } else if self.pos >= BUF_COMPACT_WATERMARK {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }
}

/// 当前 HTTP 日期字节（Date 槽内容，RFC1123 恒 29 字节）——静态响应模板快路径用。
pub(crate) fn now_http_date_bytes() -> &'static [u8] {
    thread_date_str(cached_date_secs()).as_bytes()
}

/// 全局 Date 秒戳缓存：后台线程每 250ms 刷新一次原子秒值，`default_headers`
/// / 模板快路径读原子替代每响应一次 `SystemTime::now`（vDSO ~20ns/响应，
/// d8 批内逐响应递增）。Date 是秒粒度头，缓存滞后 ≤250ms 与秒粒度语义相容
/// （秒边界处的响应 Date 归属相邻秒，与原实现「构造时刻取整秒」仅在边界
/// 半秒窗口内不同，actix DateService 周期刷新同款口径）。
static DATE_TICK_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static DATE_TICKER: LazyLock<()> = LazyLock::new(|| {
    // 初始化即同步写一次，覆盖首个 tick 前的空窗（免 1970 Date）
    DATE_TICK_SECS.store(now_secs_u64(), std::sync::atomic::Ordering::Relaxed);
    // std 线程而非 tokio task：default_headers 在任意线程（含非 runtime
    // 上下文的单测/客户端路径）首次调用时触发初始化，进程级常驻一根轻线程
    let _ = std::thread::Builder::new()
        .name("potato-date-tick".into())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            DATE_TICK_SECS.store(now_secs_u64(), std::sync::atomic::Ordering::Relaxed);
        });
});

fn now_secs_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .unwrap_or(0)
}

fn cached_date_secs() -> i64 {
    LazyLock::force(&DATE_TICKER);
    DATE_TICK_SECS.load(std::sync::atomic::Ordering::Relaxed) as i64
}

#[derive(Debug)]
pub struct HttpResponse {
    pub version: Cow<'static, str>,
    pub http_code: u16,
    pub headers: ResponseHeaders,
    /// Extra values for repeated header fields (e.g. multiple `Set-Cookie`).
    /// The main `headers` map keeps only the first value of each field; every
    /// later occurrence is appended here and serialized after the map values.
    pub multi_headers: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    pub trailers: ResponseHeaders,
    pub body: HttpResponseBody,
}
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

/// 响应内存线程本地池（depth=1 热路径）：复用响应头表（头表
/// 一次 ~768B 分配 + 清零 + 释放）与二进制体缓冲，免去每响应的分配/
/// 释放往返。仅 potato 自身可用——公开接口零变化：无 `Drop`（加 Drop
/// 会禁止用户按值移出 `HttpResponse` 字段，破坏既有用法），构造器取池、
/// 服务端序列化完成后经 `recycle_response` 显式归还（`mem::take` 换出，
/// 原响应以空默认成员 drop）。用户构造后未发送即丢弃的响应走正常释放，
/// 只是池不回血，无泄漏无别名。
///
/// 安全性：池内对象全程值语义独占传递（pop 出去 / push 回来），map 的
/// 键值为 `Cow<'static, str>`，clear 仅释放 Owned 值（Borrowed 指向
/// 'static 数据），无悬垂别名。tokio 工作窃取导致的跨线程取还安全：
/// 池是纯缓冲，所有权单传，glibc malloc 天然支持跨线程释放。
struct ResponsePool {
    maps: Vec<ResponseHeaders>,
    body_vecs: Vec<Vec<u8>>,
}
impl ResponsePool {
    fn new() -> Self {
        Self {
            maps: Vec::new(),
            body_vecs: Vec::new(),
        }
    }
}
thread_local! {
    static RESP_POOL: std::cell::RefCell<ResponsePool> = std::cell::RefCell::new(ResponsePool::new());
}
/// 池深度上限（12 工作线程 × 上限成员 ≈ 百 KB 级，内存有界）
const RESP_POOL_MAX_MAPS: usize = 16;
const RESP_POOL_MAX_VECS: usize = 16;
/// 超大响应体不入池（防大缓冲驻留），直接走正常释放
const RESP_POOL_MAX_VEC_CAPACITY: usize = 256 * 1024;

fn take_pooled_headers() -> ResponseHeaders {
    RESP_POOL.with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if let Some(mut map) = pool.maps.pop() {
                map.clear();
                return map;
            }
        }
        ResponseHeaders::with_capacity(8)
    })
}

fn recycle_headers(map: ResponseHeaders) {
    let mut map = map;
    map.clear();
    RESP_POOL.with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if pool.maps.len() < RESP_POOL_MAX_MAPS {
                pool.maps.push(map);
            }
        }
    });
}

fn take_pooled_body_vec(data: &[u8]) -> Vec<u8> {
    RESP_POOL.with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if let Some(mut vec) = pool.body_vecs.pop() {
                vec.clear();
                vec.extend_from_slice(data);
                return vec;
            }
        }
        data.to_vec()
    })
}

fn recycle_body_vec(vec: Vec<u8>) {
    // 零容量缓冲无复用价值（下次 extend_from_slice 必然重分配），直接释放
    if vec.capacity() == 0 || vec.capacity() > RESP_POOL_MAX_VEC_CAPACITY {
        return;
    }
    let mut vec = vec;
    vec.clear();
    RESP_POOL.with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if pool.body_vecs.len() < RESP_POOL_MAX_VECS {
                pool.body_vecs.push(vec);
            }
        }
    });
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
                trailers: ResponseHeaders::new(),
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
                trailers: ResponseHeaders::new(),
                body: HttpResponseBody::Data(take_pooled_body_vec(body)),
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
    ) -> ResponseHeaders {
        // ultraspeed 模式（opt-in，须在服务开始前设置）：跳过全部默认附加
        // 头，仅保留协议必需的 Connection（Content-Length 由序列化层统一
        // 补齐）。用户显式 add_header 的头不受影响。
        if crate::global_config::ServerConfig::get_ultraspeed() {
            let mut headers = take_pooled_headers();
            headers.insert("Connection".into(), "keep-alive".into());
            return headers;
        }

        // Date 秒戳读全局缓存（后台线程 250ms 刷新），免每响应一次
        // SystemTime::now；秒粒度语义见 cached_date_secs 注释
        let current_ts = cached_date_secs();

        // depth=1 热路径：每响应一条 Date 头。旧实现 Cow::Owned(String).clone()
        // 每响应一次堆分配；改为每线程一根启动期泄漏的固定槽（Box::leak，
        // 约 48B/线程，容量有界），秒切换时原地重写内容，同秒内全部响应
        // 零分配共享同一 &'static str。
        // UnsafeCell 内部可变性依据：ThreadLocal 保证仅本线程访问；重写只在
        // 秒切换发生，整体替换且恒为合法 UTF-8。持有旧借用、跨秒边界后才被
        // 序列化的响应会读到新秒的 Date——内容陈旧但安全，语义上是把 Date
        // 粒度从“响应构造时刻”放宽到“槽内容所属秒”。
        let date_str = thread_date_str(current_ts);

        // 预留容量一次性建表：6 个标准头一表装下，避免 collect 途中按
        // 增长策略多次 rehash 重分配（depth=1 每响应热路径）。
        // 优先从线程本地响应池取回收表（零分配），池空则原地建表。
        let mut headers = take_pooled_headers();
        headers.insert("Date".into(), Cow::Borrowed(date_str));
        headers.insert("Server".into(), Cow::Borrowed(SERVER_STR.as_str()));
        headers.insert("Connection".into(), "keep-alive".into());
        headers.insert("Content-Type".into(), cnt_type.into());
        headers.insert("Pragma".into(), "no-cache".into());
        headers.insert("Cache-Control".into(), "no-cache".into());
        headers
    }

    pub fn new() -> Self {
        Self {
            version: Cow::Borrowed(""),
            http_code: 0,
            headers: take_pooled_headers(),
            multi_headers: Vec::new(),
            trailers: ResponseHeaders::new(),
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
            trailers: ResponseHeaders::new(),
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
            trailers: ResponseHeaders::new(),
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
        ResponseHeaders,
        usize,
    )> {
        let mut cursor = hdr_len;
        let mut body = Vec::new();
        let mut trailers = ResponseHeaders::with_capacity(4);
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

    pub fn as_bytes(&self, cmode: CompressMode, request_method: Option<HttpMethod>) -> Vec<u8> {
        let mut ret = Vec::with_capacity(576);
        self.as_bytes_into(&mut ret, cmode, request_method);
        ret
    }

    /// 序列化进调用方提供的缓冲（d8 批合并 piece 池复用专用）：序列化代码
    /// 与 [`Self::as_bytes`] 同源同序，线上字节形态完全一致，仅目标缓冲
    /// 由调用方提供，便于 clear 后复用容量免去每响应一次序列化堆分配。
    pub(crate) fn as_bytes_into(
        &self,
        ret: &mut Vec<u8>,
        mut cmode: CompressMode,
        request_method: Option<HttpMethod>,
    ) {
        match &self.body {
            HttpResponseBody::Data(data) => {
                let suppress_body = Self::status_disallows_response_body(self.http_code);
                // 方法级抑制（HEAD）：与 write_to_stream 对齐——Content-Length
                // 照常声明，但 body 字节不落盘
                let suppress_body_by_method = Self::method_disallows_response_body(request_method);
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
                let status_str = self.http_code.http_code_to_desp();
                // 单遍直写序列化：常量前缀 extend + 手写整数十进制，绕开
                // write! 的 fmt 机制调度开销（d8 批合并热路径每响应约
                // 150~300ns）。字节形态与原 write! 版逐字节一致：状态行
                // `"{version} {code} {desp}\r\n"`、头行 `"k: v\r\n"`、
                // CL 行 `"Content-Length: {n}\r\n"` 均无宽度/精度格式。
                // version 非 HTTP/1.1 时保留 write! 通用分支兜底。
                if self.version == "HTTP/1.1" {
                    ret.extend_from_slice(b"HTTP/1.1 ");
                    let mut tmp = [0u8; 20];
                    ret.extend_from_slice(u64_ascii_str(self.http_code as u64, &mut tmp).as_bytes());
                    ret.extend_from_slice(b" ");
                    ret.extend_from_slice(status_str.as_bytes());
                    ret.extend_from_slice(b"\r\n");
                } else {
                    let _ = write!(ret, "{} {} {status_str}\r\n", self.version, self.http_code);
                }
                // 6 标准头快速路径：与 write_to_stream 的固定顺序输出对齐，
                // 保证 as_bytes 与 write_to_stream 线上字节形态一致。
                // get 内部 std_slot_of (长度,首字节) 预筛 + 槽直达，逐键取值。
                if self.headers.len() == 6 && self.multi_headers.is_empty() {
                    if let (Some(date), Some(server), Some(connection), Some(content_type), Some(pragma), Some(cache_control)) = (
                        self.headers.get("Date"),
                        self.headers.get("Server"),
                        self.headers.get("Connection"),
                        self.headers.get("Content-Type"),
                        self.headers.get("Pragma"),
                        self.headers.get("Cache-Control"),
                    ) {
                        push_header_kv_bytes!(ret, "Date", date);
                        push_header_kv_bytes!(ret, "Server", server);
                        push_header_kv_bytes!(ret, "Connection", connection);
                        push_header_kv_bytes!(ret, "Content-Type", content_type);
                        push_header_kv_bytes!(ret, "Pragma", pragma);
                        push_header_kv_bytes!(ret, "Cache-Control", cache_control);
                    } else {
                        for (key, value) in self.headers.iter() {
                            if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                                continue;
                            }
                            push_header_kv_bytes!(ret, key, value);
                        }
                    }
                } else {
                    for (key, value) in self.headers.iter() {
                        if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                            continue;
                        }
                        push_header_kv_bytes!(ret, key, value);
                    }
                }
                for (key, value) in self.multi_headers.iter() {
                    if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding") {
                        continue;
                    }
                    push_header_kv_bytes!(ret, key, value);
                }
                if !suppress_body {
                    ret.extend_from_slice(b"Content-Length: ");
                    let mut tmp = [0u8; 20];
                    ret.extend_from_slice(u64_ascii_str(payload_ref.len() as u64, &mut tmp).as_bytes());
                    ret.extend_from_slice(b"\r\n");
                }
                if cmode == CompressMode::Gzip && !suppress_body {
                    ret.extend_from_slice(b"Content-Encoding: gzip\r\n");
                }
                // 头部终止空行（v2.28 拍板修复的还原）：101 Switching Protocols
                // 走本函数序列化（suppress_body 无 Content-Length 行），缺此
                // 空行即畸形 101，RFC6455 客户端（浏览器/wslite）握手永久
                // 挂死。与 write_to_stream 的线上形态对齐。
                ret.extend_from_slice(b"\r\n");
                if !suppress_body_by_method && !payload_ref.is_empty() {
                    ret.extend(payload_ref);
                }
            }
            HttpResponseBody::Stream(_) => {} // Stream responses are handled separately
        }
    }

    /// 服务端热路径专用：响应序列化完成后把头表与 Data 体缓冲归还线程
    /// 本地池（`mem::take` 换出，self 留空默认成员，后续 drop 零成本）。
    /// 仅 Data 体回收；Stream 体生命周期不受控（泵活多久不确定）不回收。
    /// 非 `pub`——用户侧构造的响应不经此路径，行为与直接 drop 一致。
    pub(crate) fn recycle_into_pool(&mut self) {
        if matches!(self.body, HttpResponseBody::Data(_)) {
            let headers = std::mem::take(&mut self.headers);
            recycle_headers(headers);
            if let HttpResponseBody::Data(vec) = std::mem::replace(
                &mut self.body,
                HttpResponseBody::Data(Vec::new()),
            ) {
                recycle_body_vec(vec);
            }
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

                // 单遍直写序列化：直接写字节进目标缓冲。此前每头经
                // ssformat! 先格式化进临时 SmallString 再 push_str 拷贝，
                // 双重格式化在 depth=1 热路径实测比批处理 as_bytes 路径
                // 慢约 3%（2026-11-24 ABBA 配对实锤）。热路径常量前缀
                // extend + 手写整数十进制，字节形态与 write! 版一致。
                use std::fmt::Write as _;
                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                if self.version == "HTTP/1.1" {
                    ret.push_str("HTTP/1.1 ");
                    let mut tmp = [0u8; 20];
                    ret.push_str(u64_ascii_str(self.http_code as u64, &mut tmp));
                    ret.push_str(" ");
                    ret.push_str(status_str);
                    ret.push_str("\r\n");
                } else {
                    let _ = write!(ret, "{} {} {status_str}\r\n", self.version, self.http_code);
                }
                if self.headers.len() == 6 && self.multi_headers.is_empty() {
                    if let (Some(date), Some(server), Some(connection), Some(content_type), Some(pragma), Some(cache_control)) = (
                        self.headers.get("Date"),
                        self.headers.get("Server"),
                        self.headers.get("Connection"),
                        self.headers.get("Content-Type"),
                        self.headers.get("Pragma"),
                        self.headers.get("Cache-Control"),
                    ) {
                        let _ = write!(ret, "Date: {}\r\n", sanitize_header_str(date));
                        let _ = write!(ret, "Server: {}\r\n", sanitize_header_str(server));
                        let _ = write!(ret, "Connection: {}\r\n", sanitize_header_str(connection));
                        let _ = write!(ret, "Content-Type: {}\r\n", sanitize_header_str(content_type));
                        let _ = write!(ret, "Pragma: {}\r\n", sanitize_header_str(pragma));
                        let _ = write!(ret, "Cache-Control: {}\r\n", sanitize_header_str(cache_control));
                    } else {
                        for (key, value) in self.headers.iter() {
                            if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding")
                            {
                                continue;
                            }
                            let _ = write!(
                                ret,
                                "{}: {}\r\n",
                                sanitize_header_str(key),
                                sanitize_header_str(value)
                            );
                        }
                    }
                } else {
                    for (key, value) in self.headers.iter() {
                        if key == "Content-Length" || key.eq_ignore_ascii_case("Transfer-Encoding")
                        {
                            continue;
                        }
                        let _ = write!(
                            ret,
                            "{}: {}\r\n",
                            sanitize_header_str(key),
                            sanitize_header_str(value)
                        );
                    }
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
                if !suppress_body_by_status {
                    ret.push_str("Content-Length: ");
                    let mut tmp = [0u8; 20];
                    ret.push_str(u64_ascii_str(payload_ref.len() as u64, &mut tmp));
                    ret.push_str("\r\n");
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
                use std::fmt::Write as _;
                let mut ret = smallstr::SmallString::<[u8; 4096]>::new();
                let status_str = self.http_code.http_code_to_desp();
                let _ = write!(ret, "{} {} {status_str}\r\n", self.version, self.http_code);
                for (key, value) in self.headers.iter() {
                    // 流式响应统一由写出处补 TE: chunked，headers 里预置的
                    // （如 HttpResponse::chunked）必须跳过，避免重复头
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
                    ret.push_str("Transfer-Encoding: chunked\r\n");
                }
                if declared_trailer_names.is_empty() && !outbound_stream_trailers.is_empty() {
                    let trailer_names = outbound_stream_trailers
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = write!(ret, "Trailer: {trailer_names}\r\n");
                }
                ret.push_str("\r\n");
                stream.write_all(ret.as_bytes()).await?;

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
            // 非法值直接报错：按 0 处理会让剩余字节被误当下一跳响应
            // （解析不同步向量）。
            bdy_len = cnt_len
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid Content-Length header"))?;
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
                // RFC 9112: Content-Length 值必须为纯数字，与请求侧同口径。
                if !header_value.bytes().all(|b| b.is_ascii_digit()) {
                    Err(anyhow!("invalid Content-Length header"))?;
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
                // RFC 9112: a recipient MUST reject messages containing more
                // than one Transfer-Encoding header field (request smuggling).
                // 与请求侧同口径：重复头直接拒绝，不做逗号合并。
                if transfer_encoding_seen {
                    anyhow::bail!("duplicate Transfer-Encoding headers are not allowed");
                }
                if header_value.is_empty() {
                    Err(anyhow!("empty Transfer-Encoding header"))?;
                }
                transfer_encoding_seen = true;
                req.headers
                    .insert("Transfer-Encoding".into(), header_value.to_string().into());
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
        // 头表 get 本身大小写不敏感（标准头槽位掩码直达 + find_pos
        // eq_ignore_ascii_case 线性扫描），此前先 http_std_case() 构造规范
        // 键 String 纯属每次调用一次堆分配（depth=1 热路径每响应 2 次调用
        // = 每请求 3 次 malloc/realloc，采样回溯实锤），直接传原始键。
        self.headers.get(key).map(|a| a.as_ref())
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
    use super::{
        CompressMode, HttpConnection, HttpMethod, HttpRequest, HttpResponse, HttpResponseBody,
    };
    use crate::utils::bytes::CompressExt;
    use crate::utils::header_store::ResponseHeaders;

    /// as_bytes_into 序列化字节形态回归锚：6 标准头快速路径（含 CL 手写
    /// 十进制），期望串为逐字节精确形态。
    #[test]
    fn as_bytes_into_exact_bytes_six_header_fast_path() {
        let mut headers = ResponseHeaders::new();
        headers.insert("Date".into(), "Thu, 01 Jan 2026 00:00:00 GMT".into());
        headers.insert("Server".into(), "srv".into());
        headers.insert("Connection".into(), "keep-alive".into());
        headers.insert("Content-Type".into(), "text/html".into());
        headers.insert("Pragma".into(), "no-cache".into());
        headers.insert("Cache-Control".into(), "no-cache".into());
        let res = HttpResponse {
            version: std::borrow::Cow::Borrowed("HTTP/1.1"),
            http_code: 200,
            headers,
            multi_headers: Vec::new(),
            trailers: ResponseHeaders::new(),
            body: HttpResponseBody::Data(b"Hello, World!".to_vec()),
        };
        let out = res.as_bytes(CompressMode::None, None);
        let expected = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Date: Thu, 01 Jan 2026 00:00:00 GMT\r\n",
            "Server: srv\r\n",
            "Connection: keep-alive\r\n",
            "Content-Type: text/html\r\n",
            "Pragma: no-cache\r\n",
            "Cache-Control: no-cache\r\n",
            "Content-Length: 13\r\n",
            "\r\n",
            "Hello, World!",
        );
        assert_eq!(out, expected.as_bytes());
    }

    /// as_bytes_into 通用头循环路径（非 6 标准头组合）+ 头值净化
    ///（CR/LF 剔除）字节形态回归锚。
    #[test]
    fn as_bytes_into_exact_bytes_generic_path_sanitized() {
        let mut headers = ResponseHeaders::new();
        headers.insert("Date".into(), "Thu, 01 Jan 2026 00:00:00 GMT".into());
        headers.insert("Content-Type".into(), "text/html".into());
        headers.insert("X-Custom".into(), "a\rb\nc d".into());
        let res = HttpResponse {
            version: std::borrow::Cow::Borrowed("HTTP/1.1"),
            http_code: 200,
            headers,
            multi_headers: Vec::new(),
            trailers: ResponseHeaders::new(),
            body: HttpResponseBody::Data(b"payload".to_vec()),
        };
        let out = res.as_bytes(CompressMode::None, None);
        let expected = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Date: Thu, 01 Jan 2026 00:00:00 GMT\r\n",
            "Content-Type: text/html\r\n",
            "X-Custom: abc d\r\n",
            "Content-Length: 7\r\n",
            "\r\n",
            "payload",
        );
        assert_eq!(out, expected.as_bytes());
    }

    /// as_bytes_into gzip 分支：≥32B 可压体压缩出 Content-Encoding 行 +
    /// 压缩后 CL + 压缩字节尾；<32B 保持原样不压缩。
    #[test]
    fn as_bytes_into_gzip_branch_boundaries() {
        let make = |data: Vec<u8>| {
            let mut headers = ResponseHeaders::new();
            headers.insert("Date".into(), "Thu, 01 Jan 2026 00:00:00 GMT".into());
            headers.insert("Content-Type".into(), "text/plain".into());
            HttpResponse {
                version: std::borrow::Cow::Borrowed("HTTP/1.1"),
                http_code: 200,
                headers,
                multi_headers: Vec::new(),
                trailers: ResponseHeaders::new(),
                body: HttpResponseBody::Data(data),
            }
        };
        // ≥32B：压缩分支
        let data = vec![b'a'; 256];
        let out = make(data.clone()).as_bytes(CompressMode::Gzip, None);
        let compressed = data.compress().expect("gzip compress");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("Content-Encoding: gzip\r\n"));
        assert!(text.contains(&format!("Content-Length: {}\r\n", compressed.len())));
        assert!(out.ends_with(&compressed));
        // <32B：不压缩，无 Content-Encoding 行，CL 为原始长度
        let small = b"tiny".to_vec();
        let out = make(small).as_bytes(CompressMode::Gzip, None);
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("Content-Encoding"));
        assert!(text.contains("Content-Length: 4\r\n"));
    }

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

    /// 头槽复用假设钉测：httparse 1.10 把槽按 MaybeUninit 对待（旧值从不
    /// 被读、解析后切片收缩到本次写入数），连接循环跨请求复用脏槽数组时
    /// 解析结果必须与干净槽完全一致。若未来 httparse 行为变化（读旧值），
    /// 本测试率先失败拦截。
    #[test]
    fn header_slots_reuse_parses_garbage_slots_correctly() {
        let raw = b"GET /hello HTTP/1.1
Host: a
Connection: keep-alive

";
        // 干净基线
        let mut clean_slots = [httparse::EMPTY_HEADER; 48];
        let mut req_clean = HttpRequest::new();
        let consumed_clean = HttpRequest::from_headers_part_with_limits_into(
            raw,
            usize::MAX,
            &mut req_clean,
            &mut clean_slots,
        )
        .expect("clean parse ok")
        .expect("complete");
        // 脏槽：全部填充非 EMPTY 的垃圾头
        let mut dirty_slots = [httparse::Header {
            name: "garbage-header-name",
            value: b"garbage-value",
        }; 48];
        let mut req_dirty = HttpRequest::new();
        let consumed_dirty = HttpRequest::from_headers_part_with_limits_into(
            raw,
            usize::MAX,
            &mut req_dirty,
            &mut dirty_slots,
        )
        .expect("dirty parse ok")
        .expect("complete");
        assert_eq!(consumed_clean, consumed_dirty);
        assert_eq!(req_clean.headers.len(), req_dirty.headers.len());
        assert_eq!(
            req_clean.get_header("Host").map(|v| v.to_string()),
            req_dirty.get_header("Host").map(|v| v.to_string())
        );
        assert_eq!(req_clean.headers.len(), 2, "expected exactly 2 headers");
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

    fn parse_req(raw: &[u8]) -> HttpRequest {
        HttpRequest::from_headers_part(raw)
            .expect("parse headers")
            .expect("complete request")
            .0
    }

    /// 解析期特判头缓存：AE 解析路径命中缓存（negotiate 结果一致），
    /// 重复头 last-wins 与头表同名覆盖语义一致。
    #[test]
    fn parse_cache_accept_encoding_and_last_wins() {
        let req = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::Gzip);

        let req = parse_req(
            b"GET / HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\nAccept-Encoding: identity;q=0\r\n\r\n",
        );
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
    }

    /// 解析期特判头缓存：AE/Connection 缺席默认值与 getter 原语义一致
    ///（negotiate("")=None 模式；HTTP/1.1 默认 keep-alive、1.0 默认 close）。
    #[test]
    fn parse_cache_absent_defaults_match_table_semantics() {
        let req = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(req.get_header_accept_encoding(), CompressMode::None);
        assert_eq!(req.get_header_connection(), HttpConnection::KeepAlive);

        let req = parse_req(b"GET / HTTP/1.0\r\n\r\n");
        assert_eq!(req.get_header_connection(), HttpConnection::Close);
    }

    /// 解析期特判头缓存：Connection 判别值（close/keep-alive/未知 token
    /// Close 兜底）与 getter 原语义一致。
    #[test]
    fn parse_cache_connection_values() {
        let req = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(req.get_header_connection(), HttpConnection::Close);

        let req = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");
        assert_eq!(req.get_header_connection(), HttpConnection::KeepAlive);

        let req = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: whatever\r\n\r\n");
        assert_eq!(req.get_header_connection(), HttpConnection::Close);
    }

    /// 解析期特判头缓存：TE 单段 chunked（含尾逗号变体）缓存命中；
    /// 多段值记 None 回落，产出原 unsupported 错误。
    #[test]
    fn parse_cache_transfer_encoding_chunked_and_fallback() {
        let req = parse_req(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n");
        assert!(req.has_chunked_transfer_encoding().expect("valid TE"));

        let req =
            parse_req(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked,\r\n\r\n");
        assert!(req.has_chunked_transfer_encoding().expect("valid TE"));

        let req = parse_req(
            b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
        );
        let err = req.has_chunked_transfer_encoding().unwrap_err();
        assert!(err.to_string().contains("unsupported Transfer-Encoding"));
    }

    /// 解析期特判头缓存：CL 正常值命中；溢出值记 None 回落
    ///（pub getter 按 unwrap_or(0) 兜底，私有 parse 路径照常产出原错误）。
    #[test]
    fn parse_cache_content_length_and_overflow_fallback() {
        let req = parse_req(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 41\r\n\r\n");
        assert_eq!(req.get_header_content_length(), 41);

        let oversized = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}9999\r\n\r\n",
            u64::MAX
        );
        let req = parse_req(oversized.as_bytes());
        assert_eq!(req.get_header_content_length(), 0);
    }

    /// 解析期特判头缓存：Expect 单段 100-continue 与多段合并的表值形态。
    #[test]
    fn parse_cache_expect_values() {
        let req = parse_req(b"POST / HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\n\r\n");
        assert_eq!(req.get_header("Expect"), Some("100-continue"));

        let req = parse_req(
            b"POST / HTTP/1.1\r\nHost: x\r\nExpect: 100-continue\r\nExpect: 100-continue\r\n\r\n",
        );
        assert_eq!(req.get_header("Expect"), Some("100-continue, 100-continue"));
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
    fn response_parser_rejects_duplicate_transfer_encoding() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: identity\r\nTransfer-Encoding: chunked\r\n\r\n";
        let err = HttpResponse::from_headers_part(raw).unwrap_err();
        assert!(err.to_string().contains("duplicate Transfer-Encoding"));
    }

    #[test]
    fn request_parser_rejects_non_numeric_content_length() {
        // 非数字（含符号）在头解析阶段即拒绝
        for raw in [
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: +5\r\n\r\n".as_slice(),
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5x\r\n\r\n".as_slice(),
        ] {
            let err = HttpRequest::from_headers_part(raw).unwrap_err();
            assert!(err.to_string().contains("invalid Content-Length"));
        }
    }

    #[test]
    fn response_parser_rejects_non_numeric_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: xyz\r\n\r\n";
        let err = HttpResponse::from_headers_part(raw).unwrap_err();
        assert!(err.to_string().contains("invalid Content-Length"));
    }

    #[test]
    fn response_transfer_encoding_supports_case_insensitive_lists() {
        assert!(HttpResponse::transfer_encoding_has_chunked("gzip, Chunked").unwrap());
    }

    #[test]
    fn response_data_serialization_omits_transfer_encoding() {
        let mut res = HttpResponse::text("hello");
        res.add_header("Transfer-Encoding".into(), "chunked".into());

        let serialized = String::from_utf8(res.as_bytes(CompressMode::None, None)).unwrap();
        assert!(serialized.contains("Content-Length: 5\r\n"));
        assert!(!serialized.contains("Transfer-Encoding:"));
    }

    /// ultraspeed 模式响应形态锚：默认附加头（Date/Server/Content-Type/
    /// Pragma/Cache-Control）全部不发，仅保留协议必需的 Connection；用户
    /// 显式 add_header 的头照常输出，序列化按插入序收敛为极简字节形态。
    /// 开关在响应构造后立即还原，最小化对并发测试的可见窗口（并发方依赖
    /// 的唯一构造点断言为 contains 形式，两种形态下均成立）。
    #[test]
    fn ultraspeed_mode_strips_default_headers() {
        // 翻开关窗口取测试锁串行化，防并行测试的全局态交叉污染
        let _guard = crate::global_config::ULTRASPEED_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::global_config::ServerConfig::set_ultraspeed(true);
        let mut res = HttpResponse::html("Hello, World!");
        crate::global_config::ServerConfig::set_ultraspeed(false);
        drop(_guard);
        res.add_header("X-Explicit".into(), "keep".into());

        let out = res.as_bytes(CompressMode::None, None);
        let expected = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Connection: keep-alive\r\n",
            "X-Explicit: keep\r\n",
            "Content-Length: 13\r\n",
            "\r\n",
            "Hello, World!",
        );
        assert_eq!(out, expected.as_bytes());
    }
}

#[cfg(test)]
mod authority_validator_equiv_tests {
    use super::validate_authority_str;
    use std::str::FromStr;

    /// 校验器与 http::uri::Authority::from_str 判定逐例等价。
    /// H 优化把 Host 校验换成本地零分配移植版，此测试钉死两套判定，
    /// http crate 升版若改变语义会在此暴露。
    #[test]
    fn host_validator_matches_http_crate_authority() {
        let named: &[&str] = &[
            "",
            "x",
            "localhost",
            "localhost:80",
            "localhost:",
            "127.0.0.1:18083",
            "[::1]:443",
            "[::1]",
            "[fe80::1%25eth0]:80",
            "::1",
            "[::1",
            "::1]",
            "host[",
            "host]",
            "[[::1]]",
            "example.com/path",
            "example.com?q=1",
            "example.com#frag",
            "user:pass@example.com:8080",
            "user:pass:extra@example.com",
            "user@name@example.com",
            "user@",
            "@example.com",
            "local%20host",
            "localhost:8080:3030",
            "host_name",
            "-a-.b.c",
            "h%!ost",
            "h~o.s_t",
            "exa[mple.com",
            "a b",
            "example.com:99999",
            "1.2.3.4:0",
            "very.long.subdomain.example.com",
            "host:00000008",
        ];
        for case in named {
            assert_eq!(
                validate_authority_str(case),
                http::uri::Authority::from_str(case).is_ok(),
                "mismatch for {case:?}"
            );
        }

        // 2 字符组合遍历：覆盖截断字符、括号、%、@、: 的两两组合语义
        let alphabet: Vec<char> = "abz09:@[ ]/?#.%_-~!$&'()*+,;=".chars().collect();
        for a in &alphabet {
            for b in &alphabet {
                let s: String = [*a, *b].iter().collect();
                assert_eq!(
                    validate_authority_str(&s),
                    http::uri::Authority::from_str(&s).is_ok(),
                    "mismatch for {s:?}"
                );
            }
        }
    }
}
