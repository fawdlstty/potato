use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};

/// Upper bound for decompression output (512 MiB). Decompressing untrusted
/// data without a limit turns a tiny "gzip bomb" into an OOM kill.
pub const MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

pub trait CompressExt {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error>;
    fn decompress(&self) -> Result<Vec<u8>, std::io::Error>;
}

impl CompressExt for &[u8] {
    fn compress(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(self)?;
        encoder.finish()
    }

    fn decompress(&self) -> Result<Vec<u8>, std::io::Error> {
        // 上限不静默截断：take 读满后再尝试读 1 字节，仍有输出即视为炸弹
        let mut decoder = GzDecoder::new(*self);
        {
            let mut limited = decoder.by_ref().take(MAX_DECOMPRESSED_BYTES);
            let mut buf = Vec::new();
            limited.read_to_end(&mut buf)?;
            let mut probe = [0u8; 1];
            let extra = decoder.by_ref().read(&mut probe)?;
            if extra > 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "decompressed data exceeds limit (possible decompression bomb)",
                ));
            }
            Ok(buf)
        }
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
