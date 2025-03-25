#![allow(non_camel_case_types)]

#[derive(Clone, Eq)]
pub struct RefStr {
    ptr: *const u8,
    len: usize,
}

impl RefStr {
    pub fn from_slice(data: &[u8], start: usize, len: usize) -> Self {
        Self {
            ptr: unsafe { data.get_unchecked(start) },
            len,
        }
    }

    pub fn from_str(data: &str) -> Self {
        Self {
            ptr: data.as_ptr(),
            len: data.len(),
        }
    }

    pub fn to_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(self.to_bytes()) }
    }

    pub fn to_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

unsafe impl Send for RefStr {}

impl std::fmt::Display for RefStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_str().fmt(f)
    }
}

impl std::fmt::Debug for RefStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefStr")
            .field("val", &self.to_str())
            .finish()
    }
}

impl std::hash::Hash for RefStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for &ch in self.to_bytes() {
            state.write_u8(match ch >= b'A' && ch <= b'Z' {
                true => ch - b'A' + b'a',
                false => ch,
            });
        }
        state.write_u8(0xff);
    }
}

impl PartialEq for RefStr {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        let (slice1, slice2) = (self.to_bytes(), other.to_bytes());
        for (&a, &b) in slice1.iter().zip(slice2) {
            let is_match = match (a >= b'A' && a <= b'Z', b >= b'A' && b <= b'Z') {
                (true, true) => a == b,
                (false, false) => a == b,
                (true, false) => a - b'A' + b'a' == b,
                (false, true) => a == b - b'A' + b'a',
            };
            if !is_match {
                return false;
            }
        }
        true
    }
}

pub trait ToRefStrExt {
    fn to_ref_str(&self) -> RefStr;
    fn to_ref_string(&self) -> RefOrString;
    fn to_header_ref_string(&self) -> HeaderRefOrString;
}

impl ToRefStrExt for str {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(self)
    }
    fn to_ref_string(&self) -> RefOrString {
        RefOrString::String(self.to_string())
    }
    fn to_header_ref_string(&self) -> HeaderRefOrString {
        self.to_ref_string().into()
    }
}

impl ToRefStrExt for [u8] {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(unsafe { std::str::from_utf8_unchecked(self) })
    }
    fn to_ref_string(&self) -> RefOrString {
        RefOrString::String(unsafe { std::str::from_utf8_unchecked(self) }.to_string())
    }
    fn to_header_ref_string(&self) -> HeaderRefOrString {
        self.to_ref_string().into()
    }
}

#[derive(Clone, Debug, Eq)]
pub enum RefOrString {
    RefStr(RefStr),
    String(String),
}

impl RefOrString {
    pub fn from_str(val: &str) -> Self {
        RefOrString::RefStr(RefStr::from_str(val))
    }

    pub fn to_bytes(&self) -> &[u8] {
        match self {
            RefOrString::RefStr(ref_str) => ref_str.to_bytes(),
            RefOrString::String(str) => str.as_bytes(),
        }
    }

    pub fn to_str(&self) -> &str {
        match self {
            RefOrString::RefStr(ref_str) => ref_str.to_str(),
            RefOrString::String(str) => str.as_str(),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            RefOrString::RefStr(ref_str) => ref_str.to_str().to_string(),
            RefOrString::String(str) => str.clone(),
        }
    }
}

unsafe impl Send for RefOrString {}

impl std::fmt::Display for RefOrString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_str().fmt(f)
    }
}

impl Into<RefOrString> for String {
    fn into(self) -> RefOrString {
        RefOrString::String(self)
    }
}

impl Into<RefOrString> for RefStr {
    fn into(self) -> RefOrString {
        RefOrString::RefStr(self)
    }
}

impl Into<RefOrString> for &str {
    fn into(self) -> RefOrString {
        self.to_ref_string()
    }
}

impl std::hash::Hash for RefOrString {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            RefOrString::RefStr(ref_str) => ref_str.hash(state),
            RefOrString::String(str) => {
                for &ch in str.as_bytes() {
                    state.write_u8(match ch >= b'A' && ch <= b'Z' {
                        true => ch - b'A' + b'a',
                        false => ch,
                    });
                }
                state.write_u8(0xff);
            }
        }
    }
}

impl PartialEq for RefOrString {
    fn eq(&self, other: &Self) -> bool {
        let (slice1, slice2) = (self.to_bytes(), other.to_bytes());
        if slice1.len() != slice2.len() {
            return false;
        }
        for (&a, &b) in slice1.iter().zip(slice2) {
            let is_match = match (a >= b'A' && a <= b'Z', b >= b'A' && b <= b'Z') {
                (true, true) => a == b,
                (false, false) => a == b,
                (true, false) => a - b'A' + b'a' == b,
                (false, true) => a == b - b'A' + b'a',
            };
            if !is_match {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum HeaderRefOrString {
    HeaderItem(HeaderItem),
    RefOrString(RefOrString),
}

impl HeaderRefOrString {
    pub fn from_str(val: &str) -> Self {
        val.to_ref_string().into()
    }

    pub fn to_str(&self) -> &str {
        match self {
            HeaderRefOrString::HeaderItem(header_item) => header_item.to_str(),
            HeaderRefOrString::RefOrString(ref_or_string) => ref_or_string.to_str(),
        }
    }
}

impl Into<HeaderRefOrString> for HeaderItem {
    fn into(self) -> HeaderRefOrString {
        HeaderRefOrString::HeaderItem(self)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum HeaderItem {
    Date,
    Host,
    Server,
    Upgrade,
    Connection,
    Content_Type,
    Content_Length,
    Accept_Encoding,
    Transfer_Encoding,
}

impl HeaderItem {
    pub fn to_str(&self) -> &str {
        match self {
            HeaderItem::Date => "Date",
            HeaderItem::Host => "Host",
            HeaderItem::Server => "Server",
            HeaderItem::Upgrade => "Upgrade",
            HeaderItem::Connection => "Connection",
            HeaderItem::Content_Type => "Content-Type",
            HeaderItem::Content_Length => "Content-Length",
            HeaderItem::Accept_Encoding => "Accept-Encoding",
            HeaderItem::Transfer_Encoding => "Transfer-Encoding",
            _ => "Unknown",
        }
    }
}

impl Into<HeaderRefOrString> for RefOrString {
    fn into(self) -> HeaderRefOrString {
        let val = self.to_str();
        HeaderRefOrString::HeaderItem(match val.len() {
            4 if val.eq_ignore_ascii_case("Date") => HeaderItem::Date,
            4 if val.eq_ignore_ascii_case("Host") => HeaderItem::Host,
            6 if val.eq_ignore_ascii_case("Server") => HeaderItem::Server,
            7 if val.eq_ignore_ascii_case("Upgrade") => HeaderItem::Upgrade,
            10 if val.eq_ignore_ascii_case("Connection") => HeaderItem::Connection,
            12 if val.eq_ignore_ascii_case("Content-Type") => HeaderItem::Content_Type,
            14 if val.eq_ignore_ascii_case("Content-Length") => HeaderItem::Content_Length,
            15 if val.eq_ignore_ascii_case("Accept-Encoding") => HeaderItem::Accept_Encoding,
            17 if val.eq_ignore_ascii_case("Transfer-Encoding") => HeaderItem::Transfer_Encoding,
            _ => return HeaderRefOrString::RefOrString(self),
        })
    }
}

impl Into<HeaderRefOrString> for &str {
    fn into(self) -> HeaderRefOrString {
        self.to_ref_string().into()
    }
}
