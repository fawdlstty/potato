use flate2::{write::GzEncoder, Compression};
use std::io::Write;

pub trait CompressExt {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error>;
}

impl CompressExt for &[u8] {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(self)?;
        Ok(encoder.finish()?)
    }
}

impl CompressExt for Vec<u8> {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        (&self[..]).compress()
    }
}
