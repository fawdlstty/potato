#[derive(Clone, Debug)]
pub struct RefBuf {
    ptr: *const u8,
    len: usize,
}

impl RefBuf {
    pub fn from_buf(buf: &[u8]) -> Self {
        Self {
            ptr: buf.as_ptr(),
            len: buf.len(),
        }
    }

    pub fn to_buf(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

pub trait ToRefBufExt {
    fn to_ref_buf(&self) -> RefBuf;
}

impl ToRefBufExt for [u8] {
    fn to_ref_buf(&self) -> RefBuf {
        RefBuf::from_buf(self)
    }
}
