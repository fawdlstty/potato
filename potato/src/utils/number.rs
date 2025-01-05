pub trait HttpCodeExt {
    fn http_code_to_desp(&self) -> &'static str;
}

impl HttpCodeExt for u16 {
    fn http_code_to_desp(&self) -> &'static str {
        http::StatusCode::from_u16(*self)
            .map(|c| c.canonical_reason())
            .ok()
            .flatten()
            .unwrap_or("UNKNOWN")
    }
}
