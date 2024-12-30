#![allow(async_fn_in_trait)]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;

pub trait TcpStreamExt: AsyncRead + AsyncWrite + Unpin {
    async fn read_until(&mut self, uc: u8) -> Vec<u8> {
        let mut buf = vec![];
        while let Ok(c) = self.read_u8().await {
            match c == uc {
                true => break,
                false => buf.push(c),
            }
        }
        buf
    }

    async fn read_line(&mut self) -> String {
        let mut line = String::from_utf8(self.read_until(b'\n').await).unwrap_or("".to_string());
        if line.ends_with('\r') {
            line.pop();
        }
        line
    }
}

impl TcpStreamExt for TcpStream {}
impl TcpStreamExt for TlsStream<TcpStream> {}
