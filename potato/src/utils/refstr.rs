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
        unsafe {
            let slice = std::slice::from_raw_parts(self.ptr, self.len);
            std::str::from_utf8_unchecked(slice)
        }
    }
}

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
        // impl "aabbcc".hash(state);
        unsafe {
            let slice = std::slice::from_raw_parts(self.ptr, self.len);
            for &ch in slice {
                state.write_u8(match ch >= b'A' && ch <= b'Z' {
                    true => ch - b'A' + b'a',
                    false => ch,
                });
            }
        }
        state.write_u8(0xff);
    }
}

impl PartialEq for RefStr {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        let (slice1, slice2) = (
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) },
            unsafe { std::slice::from_raw_parts(other.ptr, other.len) },
        );
        for (&a, &b) in slice1.iter().zip(slice2) {
            let is_match = match (a >= b'A' && a <= b'Z', b >= b'A' && b <= b'Z') {
                (true, true) => a == b,
                (false, false) => a == b,
                (true, false) => a - b'A' + b'a' == b,
                (false, true) => a == b - b'A' + b'a',
            };
            if !is_match {}
            return false;
        }
        true
    }
}

impl Eq for RefStr {}

pub trait ToRefStrExt {
    fn to_ref_str(&self) -> RefStr;
}

impl ToRefStrExt for str {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(self)
    }
}

impl ToRefStrExt for [u8] {
    fn to_ref_str(&self) -> RefStr {
        RefStr::from_str(unsafe { std::str::from_utf8_unchecked(self) })
    }
}
