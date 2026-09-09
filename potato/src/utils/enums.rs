use hipstr::LocalHipStr;

#[derive(Clone, Debug, PartialEq)]
pub enum HttpConnection {
    KeepAlive,
    Close,
    Upgrade,
}

impl HttpConnection {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(val: &str) -> Option<Self> {
        // 优先级：Upgrade > Close > Keep-Alive。
        // 升级期间连接必须保持（Upgrade 压过 Close）；只要出现 close
        // 该连接最终就会关闭（Close 压过 keep-alive）。
        let mut saw_close = false;
        let mut saw_keep_alive = false;
        for token in val
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            if token.eq_ignore_ascii_case("Upgrade") {
                return Some(Self::Upgrade);
            }
            if token.eq_ignore_ascii_case("Close") {
                saw_close = true;
            }
            if token.eq_ignore_ascii_case("Keep-Alive") {
                saw_keep_alive = true;
            }
        }
        if saw_close {
            return Some(Self::Close);
        }
        if saw_keep_alive {
            return Some(Self::KeepAlive);
        }
        None
    }
}

#[derive(PartialEq)]
pub enum HttpContentType<'a> {
    ApplicationJson,
    ApplicationXWwwFormUrlencoded,
    MultipartFormData(LocalHipStr<'a>),
}

impl<'a> HttpContentType<'a> {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(val: &str) -> Option<Self> {
        // 取 media type 本体（忽略 ;charset=utf-8 等参数）再匹配
        let media_type = val.split(';').next()?.trim();
        match media_type.len() {
            16 if media_type.eq_ignore_ascii_case("application/json") => {
                Some(Self::ApplicationJson)
            }
            19 if media_type.starts_with("multipart/form-data") => {
                let boundary = val.split_once("boundary=")?.1;
                let boundary = boundary.split(';').next()?.trim();
                // boundary 允许带引号（boundary="abc"），剥掉；空 boundary
                // 会退化成按 "--" 乱切，直接拒绝
                let boundary = boundary
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                    .unwrap_or(boundary);
                if boundary.is_empty() {
                    return None;
                }
                Some(Self::MultipartFormData(LocalHipStr::from(boundary)))
            }
            33 if media_type.eq_ignore_ascii_case("application/x-www-form-urlencoded") => {
                Some(Self::ApplicationXWwwFormUrlencoded)
            }
            _ => None,
        }
    }
}
