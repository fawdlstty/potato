use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};

pub trait CompressExt {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error>;
    fn decompress(&self) -> Result<Vec<u8>, std::io::Error>;
}

impl CompressExt for &[u8] {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(self)?;
        Ok(encoder.finish()?)
    }

    fn decompress(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut decoder = GzDecoder::new(*self);
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

impl CompressExt for Vec<u8> {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        (&self[..]).compress()
    }

    fn decompress(&self) -> Result<Vec<u8>, std::io::Error> {
        (&self[..]).decompress()
    }
}
