#![allow(dead_code)]
#![allow(non_camel_case_types)]
use crate::HttpRequest;
use hipstr::LocalHipStr;
use potato_macro::StandardHeader;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum HeaderOrHipStr {
    HeaderItem(HeaderItem),
    HipStr(LocalHipStr<'static>),
}

impl HeaderOrHipStr {
    pub fn from_str(val: &str) -> Self {
        Self::HipStr(LocalHipStr::from(val))
    }

    pub fn to_str(&self) -> &str {
        match self {
            HeaderOrHipStr::HeaderItem(header_item) => header_item.to_str(),
            HeaderOrHipStr::HipStr(hip_str) => hip_str.as_str(),
        }
    }
}

impl Into<HeaderOrHipStr> for HeaderItem {
    fn into(self) -> HeaderOrHipStr {
        HeaderOrHipStr::HeaderItem(self)
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

impl Into<HeaderOrHipStr> for &str {
    fn into(self) -> HeaderOrHipStr {
        HeaderOrHipStr::HipStr(LocalHipStr::from(self))
    }
}
