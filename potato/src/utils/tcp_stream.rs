#![allow(async_fn_in_trait)]
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;

pub enum HttpStream {
    Tcp(TcpStream),
    Tls(ServerTlsStream<TcpStream>),
}
unsafe impl Send for HttpStream {}

impl HttpStream {
    pub fn from_tcp(s: TcpStream) -> Self {
        Self::Tcp(s)
    }

    pub fn from_tls(s: ServerTlsStream<TcpStream>) -> Self {
        Self::Tls(s)
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read(buf).await?,
            HttpStream::Tls(s) => s.read(buf).await?,
        })
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read_exact(buf).await?,
            HttpStream::Tls(s) => s.read_exact(buf).await?,
        })
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        match self {
            HttpStream::Tcp(s) => s.write_all(buf).await?,
            HttpStream::Tls(s) => s.write_all(buf).await?,
        }
        Ok(())
    }
}

#[async_trait]
pub trait TcpStreamExt: AsyncRead + AsyncWrite + Unpin + Send {
    // async fn read_until(&mut self, uc: u8) -> Vec<u8> {
    //     let mut buf = vec![];
    //     while let Ok(c) = self.read_u8().await {
    //         match c == uc {
    //             true => break,
    //             false => buf.push(c),
    //         }
    //     }
    //     buf
    // }

    // async fn read_line(&mut self) -> String {
    //     let mut line = String::from_utf8(self.read_until(b'\n').await).unwrap_or("".to_string());
    //     if line.ends_with('\r') {
    //         line.pop();
    //     }
    //     line
    // }
}

impl TcpStreamExt for TcpStream {}
impl TcpStreamExt for ClientTlsStream<TcpStream> {}
impl TcpStreamExt for ServerTlsStream<TcpStream> {}

pub trait TcpStreamExt2 {
    fn get_mut(self) -> &'static mut dyn TcpStreamExt;
}

impl TcpStreamExt2 for *mut dyn TcpStreamExt {
    fn get_mut(self) -> &'static mut dyn TcpStreamExt {
        unsafe { &mut *self as &mut dyn TcpStreamExt }
    }
}

#[async_trait]
pub trait VecU8Ext {
    async fn extend_by_streams(
        &mut self,
        stream: &mut Box<dyn TcpStreamExt>,
    ) -> anyhow::Result<usize>;
}

#[async_trait]
impl VecU8Ext for Vec<u8> {
    async fn extend_by_streams(
        &mut self,
        stream: &mut Box<dyn TcpStreamExt>,
    ) -> anyhow::Result<usize> {
        let mut tmp_buf = [0u8; 1024];
        let n = stream.read(&mut tmp_buf).await?;
        if n == 0 {
            return Err(anyhow::Error::msg("connection closed"));
        }
        self.extend(&tmp_buf[0..n]);
        Ok(n)
    }
}
