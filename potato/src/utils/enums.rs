use super::refstr::{RefStr, ToRefStrExt};

#[derive(Clone, Debug, PartialEq)]
pub enum HttpConnection {
    KeepAlive,
    Close,
    Upgrade,
}

impl HttpConnection {
    pub fn from_str(val: &str) -> Option<Self> {
        match val.len() {
            7 if val.eq_ignore_ascii_case("Upgrade") => Some(Self::Upgrade),
            10 if val.eq_ignore_ascii_case("Keep-Alive") => Some(Self::KeepAlive),
            _ => None,
        }
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
        match val.len() {
            16 if val.eq_ignore_ascii_case("application/json") => Some(Self::ApplicationJson),
            19 if val.starts_with("multipart/form-data") => val
                .split_once("boundary=")
                .map(|(_, boundary)| Self::MultipartFormData(boundary.to_ref_str())),
            33 if val.eq_ignore_ascii_case("application/x-www-form-urlencoded") => {
                Some(Self::ApplicationXWwwFormUrlencoded)
            }
            _ => None,
        }
    }
}
