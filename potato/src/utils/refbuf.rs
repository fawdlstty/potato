#[derive(Clone, Debug)]
pub struct RefBuf {
    ptr: *const u8,
    len: usize,
}

impl RefBuf {
    pub fn from_buf(buf: &[u8]) -> Self {
        let (ptr, len) = (buf.as_ptr(), buf.len());
        Self { ptr, len }
    }

    pub fn to_buf(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

pub trait ToRefBufExt {
    fn to_ref_buf(&self) -> RefBuf;
    fn to_ref_buffer(&self) -> RefOrBuffer;
}

impl ToRefBufExt for [u8] {
    fn to_ref_buf(&self) -> RefBuf {
        RefBuf::from_buf(self)
    }

    fn to_ref_buffer(&self) -> RefOrBuffer {
        RefOrBuffer::RefBuf(RefBuf::from_buf(self))
    }
}

#[derive(Clone, Debug)]
pub enum RefOrBuffer {
    RefBuf(RefBuf),
    Buffer(Vec<u8>),
}

impl RefOrBuffer {
    pub fn from_ref_buf(buf: &[u8]) -> Self {
        let (ptr, len) = (buf.as_ptr(), buf.len());
        Self::RefBuf(RefBuf { ptr, len })
    }

    pub fn from_buffer(buf: Vec<u8>) -> Self {
        Self::Buffer(buf)
    }

    pub fn to_buf(&self) -> &[u8] {
        match self {
            Self::RefBuf(refbuf) => refbuf.to_buf(),
            Self::Buffer(buf) => buf.as_slice(),
        }
    }
}

impl Into<RefOrBuffer> for Vec<u8> {
    fn into(self) -> RefOrBuffer {
        RefOrBuffer::Buffer(self)
    }
}
