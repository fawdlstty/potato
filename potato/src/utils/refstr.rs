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
    fn to_ref_string(&self) -> RefStrOrString;
}

impl ToRefStrExt for str {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(self)
    }
    fn to_ref_string(&self) -> RefStrOrString {
        self.to_ref_str().into()
    }
}

impl ToRefStrExt for [u8] {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(unsafe { std::str::from_utf8_unchecked(self) })
    }
    fn to_ref_string(&self) -> RefStrOrString {
        self.to_ref_str().into()
    }
}

#[derive(Debug, Eq)]
pub enum RefStrOrString {
    RefStr(RefStr),
    String(String),
}

impl RefStrOrString {
    pub fn from_str(val: &str) -> Self {
        RefStrOrString::RefStr(RefStr::from_str(val))
    }

    pub fn to_bytes(&self) -> &[u8] {
        match self {
            RefStrOrString::RefStr(ref_str) => ref_str.to_bytes(),
            RefStrOrString::String(str) => str.as_bytes(),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            RefStrOrString::RefStr(ref_str) => ref_str.to_str().to_string(),
            RefStrOrString::String(str) => str.clone(),
        }
    }
}

impl Into<RefStrOrString> for String {
    fn into(self) -> RefStrOrString {
        RefStrOrString::String(self)
    }
}

impl Into<RefStrOrString> for RefStr {
    fn into(self) -> RefStrOrString {
        RefStrOrString::RefStr(self)
    }
}

impl std::hash::Hash for RefStrOrString {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            RefStrOrString::RefStr(ref_str) => ref_str.hash(state),
            RefStrOrString::String(str) => {
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

impl PartialEq for RefStrOrString {
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
