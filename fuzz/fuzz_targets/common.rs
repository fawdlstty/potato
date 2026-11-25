//! 供三个 fuzz target 共享的原始帧扫描工具。
//!
//! 用 httparse 在 fuzz target 内独立复扫输入，提取分帧相关事实
//! （Content-Length 值列表、Transfer-Encoding 令牌链、Host 数、Expect、
//! 版本、状态码），用于与 potato 解析器的歧义帧拒绝语义做交叉比对：
//! 凡原始扫描可见的歧义帧被解析器接受，断言即失败。
//! 此处刻意不复用 potato 的解析代码，保证比对独立。
#![allow(dead_code)]

/// 与请求侧头解析默认上限（ServerConfig max_header_count=48）一致。
pub const MAX_REQUEST_HEADERS: usize = 48;

/// 与响应侧头解析上限（full.rs 中 96 头数组）一致。
pub const MAX_RESPONSE_HEADERS: usize = 96;

/// 请求侧原始扫描事实。
pub struct RequestFacts {
    /// httparse 头解析完整（错误/不完整一律 false）
    pub complete: bool,
    /// 完整时为头段长度（httparse Complete(n) 的 n）
    pub hdr_len: usize,
    /// HTTP/1.1（version 段为 1）
    pub version_11: bool,
    /// 各 Content-Length 头 trim 后的原值（按出现序）
    pub content_lengths: Vec<String>,
    /// 各 Transfer-Encoding 头：值按 ',' 切 token、trim、小写、滤空
    pub transfer_encoding_headers: Vec<Vec<String>>,
    pub host_count: usize,
    pub expect_seen: bool,
}

impl RequestFacts {
    /// 存在值不同的多个 Content-Length 头。
    pub fn cl_conflicting(&self) -> bool {
        match self.content_lengths.split_first() {
            Some((first, rest)) => rest.iter().any(|v| v != first),
            None => false,
        }
    }

    pub fn has_te(&self) -> bool {
        !self.transfer_encoding_headers.is_empty()
    }

    pub fn has_cl(&self) -> bool {
        !self.content_lengths.is_empty()
    }

    /// potato 请求侧语义：TE 头恰一个，且值切分后恰为 ["chunked"]。
    pub fn te_plain_chunked(&self) -> bool {
        self.transfer_encoding_headers.len() == 1 && self.transfer_encoding_headers[0] == ["chunked"]
    }
}

/// 响应侧原始扫描事实。
pub struct ResponseFacts {
    pub complete: bool,
    pub status: u16,
    pub content_lengths: Vec<String>,
    pub transfer_encoding_headers: Vec<Vec<String>>,
}

impl ResponseFacts {
    /// 存在值不同的多个 Content-Length 头。
    pub fn cl_conflicting(&self) -> bool {
        match self.content_lengths.split_first() {
            Some((first, rest)) => rest.iter().any(|v| v != first),
            None => false,
        }
    }

    pub fn has_te(&self) -> bool {
        !self.transfer_encoding_headers.is_empty()
    }

    pub fn has_cl(&self) -> bool {
        !self.content_lengths.is_empty()
    }

    /// potato 响应侧语义：被当作 chunked 分帧接受要求令牌链最后一节为
    /// chunked（与 transfer_encoding_has_chunked 的判定同构）。
    pub fn te_chunked_final(&self) -> bool {
        self.transfer_encoding_headers.len() == 1
            && self.transfer_encoding_headers[0]
                .last()
                .is_some_and(|t| t == "chunked")
    }
}

fn te_tokens(raw: &[u8]) -> Option<Vec<String>> {
    let raw = std::str::from_utf8(raw).ok()?;
    Some(
        raw.split(',')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect(),
    )
}

pub fn scan_request(buf: &[u8]) -> RequestFacts {
    let mut headers = [httparse::EMPTY_HEADER; MAX_REQUEST_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    let hdr_len = match httparse::ParserConfig::default().parse_request(&mut req, buf) {
        Ok(httparse::Status::Complete(n)) => n,
        _ => 0,
    };
    let complete = hdr_len > 0;
    let mut facts = RequestFacts {
        complete,
        hdr_len,
        version_11: complete && req.version == Some(1),
        content_lengths: Vec::new(),
        transfer_encoding_headers: Vec::new(),
        host_count: 0,
        expect_seen: false,
    };
    if !complete {
        return facts;
    }
    for h in req.headers.iter() {
        if h.name.is_empty() {
            break;
        }
        if h.name.eq_ignore_ascii_case("Content-Length") {
            if let Ok(v) = std::str::from_utf8(h.value) {
                facts.content_lengths.push(v.trim().to_string());
            }
        } else if h.name.eq_ignore_ascii_case("Transfer-Encoding") {
            facts
                .transfer_encoding_headers
                .push(te_tokens(h.value).unwrap_or_default());
        } else if h.name.eq_ignore_ascii_case("Host") {
            facts.host_count += 1;
        } else if h.name.eq_ignore_ascii_case("Expect") {
            facts.expect_seen = true;
        }
    }
    facts
}

pub fn scan_response(buf: &[u8]) -> ResponseFacts {
    let mut headers = [httparse::EMPTY_HEADER; MAX_RESPONSE_HEADERS];
    let mut res = httparse::Response::new(&mut headers);
    let complete = matches!(
        httparse::ParserConfig::default().parse_response(&mut res, buf),
        Ok(httparse::Status::Complete(_))
    );
    let mut facts = ResponseFacts {
        complete,
        status: complete.then(|| res.code).flatten().unwrap_or(0),
        content_lengths: Vec::new(),
        transfer_encoding_headers: Vec::new(),
    };
    if !complete {
        return facts;
    }
    for h in res.headers.iter() {
        if h.name.is_empty() {
            break;
        }
        if h.name.eq_ignore_ascii_case("Content-Length") {
            if let Ok(v) = std::str::from_utf8(h.value) {
                facts.content_lengths.push(v.trim().to_string());
            }
        } else if h.name.eq_ignore_ascii_case("Transfer-Encoding") {
            facts
                .transfer_encoding_headers
                .push(te_tokens(h.value).unwrap_or_default());
        }
    }
    facts
}
