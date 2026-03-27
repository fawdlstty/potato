use hipstr::LocalHipStr;

#[derive(Clone, Debug, PartialEq)]
pub enum HttpConnection {
    KeepAlive,
    Close,
    Upgrade,
}

impl HttpConnection {
    pub fn from_str(val: &str) -> Option<Self> {
        match val.len() {
            5 if val.eq_ignore_ascii_case("Close") => Some(Self::Close),
            7 if val.eq_ignore_ascii_case("Upgrade") => Some(Self::Upgrade),
            10 if val.eq_ignore_ascii_case("Keep-Alive") => Some(Self::KeepAlive),
            _ => None,
        }
    }
}

#[derive(PartialEq)]
pub enum HttpContentType<'a> {
    ApplicationJson,
    ApplicationXWwwFormUrlencoded,
    MultipartFormData(LocalHipStr<'a>),
}

impl<'a> HttpContentType<'a> {
    pub fn from_str(val: &str) -> Option<Self> {
        match val.len() {
            16 if val.eq_ignore_ascii_case("application/json") => Some(Self::ApplicationJson),
            19 if val.starts_with("multipart/form-data") => val
                .split_once("boundary=")
                .map(|(_, boundary)| Self::MultipartFormData(LocalHipStr::from(boundary))),
            33 if val.eq_ignore_ascii_case("application/x-www-form-urlencoded") => {
                Some(Self::ApplicationXWwwFormUrlencoded)
            }
            _ => None,
        }
    }
}
