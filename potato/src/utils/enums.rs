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
        let mut parsed = None;
        for token in val
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match token.len() {
                5 if token.eq_ignore_ascii_case("Close") => return Some(Self::Close),
                7 if token.eq_ignore_ascii_case("Upgrade") => parsed = Some(Self::Upgrade),
                10 if token.eq_ignore_ascii_case("Keep-Alive") && parsed.is_none() => {
                    parsed = Some(Self::KeepAlive);
                }
                _ => {}
            }
        }
        parsed
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
