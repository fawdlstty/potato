use std::io::Write;

use flate2::{write::GzEncoder, Compression};

pub trait VecU8Ext {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error>;
}

impl VecU8Ext for &[u8] {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(self)?;
        Ok(encoder.finish()?)
    }
}

impl VecU8Ext for Vec<u8> {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        (&self[..]).compress()
    }
}
