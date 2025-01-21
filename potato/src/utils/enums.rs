use super::refstr::{RefStr, ToRefStrExt};

#[derive(Clone, Debug, PartialEq)]
pub enum HttpConnection {
    KeepAlive,
    Close,
    Upgrade,
}

impl HttpConnection {
    pub fn from_str(val: &str) -> Option<Self> {
        if val.eq_ignore_ascii_case("Keep-Alive") {
            return Some(Self::KeepAlive);
        }
        if val.eq_ignore_ascii_case("Upgrade") {
            return Some(Self::Upgrade);
        }
        None
    }
}

#[derive(PartialEq)]
pub enum HttpContentType {
    ApplicationJson,
    ApplicationXWwwFormUrlencoded,
    MultipartFormData(RefStr),
}

impl HttpContentType {
    pub fn from_str(val: &str) -> Option<Self> {
        if val.eq_ignore_ascii_case("application/json") {
            return Some(Self::ApplicationJson);
        }
        if val.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
            return Some(Self::ApplicationXWwwFormUrlencoded);
        }
        if val.starts_with("multipart/form-data") {
            if let Some((_, boundary)) = val.split_once("boundary=") {
                return Some(Self::MultipartFormData(boundary.to_ref_str()));
            }
        }
        None
    }
}
