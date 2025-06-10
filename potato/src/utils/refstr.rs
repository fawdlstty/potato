#![allow(dead_code)]
#![allow(non_camel_case_types)]
use crate::HttpRequest;
use potato_macro::StandardHeader;

#[derive(Clone, Eq)]
pub struct RefStr {
    ptr: *const u8,
    len: usize,
}

impl RefStr {
    pub(crate) fn from_slice(data: &[u8], start: usize, len: usize) -> Self {
        let ptr = unsafe { data.get_unchecked(start) };
        Self { ptr, len }
    }

    pub(crate) fn from_str(data: &str) -> Self {
        let (ptr, len) = (data.as_ptr(), data.len());
        Self { ptr, len }
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
    pub(crate) fn from_str(val: &str) -> Self {
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
    pub(crate) fn from_str(val: &str) -> Self {
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

#[derive(Clone, Debug, Eq, Hash, PartialEq, StandardHeader)]
pub enum HeaderItem {
    Accept,
    Accept_CH,
    Accept_Encoding,
    Accept_Language,
    Accept_Patch,
    Accept_Post,
    Accept_Ranges,
    Access_Control_Allow_Credentials,
    Access_Control_Allow_Headers,
    Access_Control_Allow_Methods,
    Access_Control_Allow_Origin,
    Access_Control_Expose_Headers,
    Access_Control_Max_Age,
    Access_Control_Request_Headers,
    Access_Control_Request_Method,
    Age,
    Allow,
    Alt_Svc,
    Alt_Used,
    Authorization,
    Cache_Control,
    Clear_Site_Data,
    Connection,
    Content_Digest,
    Content_Disposition,
    Content_Encoding,
    Content_Language,
    Content_Length,
    Content_Location,
    Content_Range,
    Content_Security_Policy,
    Content_Security_Policy_Report_Only,
    Content_Type,
    Cookie,
    Cross_Origin_Embedder_Policy,
    Cross_Origin_Opener_Policy,
    Cross_Origin_Resource_Policy,
    Date,
    Device_Memory,
    ETag,
    Expect,
    Expires,
    Forwarded,
    From,
    Host,
    If_Match,
    If_Modified_Since,
    If_None_Match,
    If_Range,
    If_Unmodified_Since,
    Keep_Alive,
    Last_Modified,
    Link,
    Location,
    Max_Forwards,
    Origin,
    Priority,
    Proxy_Authenticate,
    Proxy_Authorization,
    Range,
    Referer,
    Referrer_Policy,
    Refresh,
    Repr_Digest,
    Retry_After,
    Sec_Fetch_Dest,
    Sec_Fetch_Mode,
    Sec_Fetch_Site,
    Sec_Fetch_User,
    Sec_Purpose,
    Sec_WebSocket_Accept,
    Sec_WebSocket_Extensions,
    Sec_WebSocket_Key,
    Sec_WebSocket_Protocol,
    Sec_WebSocket_Version,
    Server,
    Server_Timing,
    Service_Worker,
    Service_Worker_Allowed,
    Service_Worker_Navigation_Preload,
    Set_Cookie,
    SourceMap,
    Strict_Transport_Security,
    TE,
    Timing_Allow_Origin,
    Trailer,
    Transfer_Encoding,
    Upgrade,
    Upgrade_Insecure_Requests,
    User_Agent,
    Vary,
    Via,
    Want_Content_Digest,
    Want_Repr_Digest,
    WWW_Authenticate,
    X_Content_Type_Options,
    X_Frame_Options,
}

impl Into<HeaderRefOrString> for RefOrString {
    fn into(self) -> HeaderRefOrString {
        match HeaderItem::try_from_str(self.to_str()) {
            Some(header_item) => HeaderRefOrString::HeaderItem(header_item),
            None => HeaderRefOrString::RefOrString(self),
        }
    }
}

impl Into<HeaderRefOrString> for &str {
    fn into(self) -> HeaderRefOrString {
        self.to_ref_string().into()
    }
}
