#![allow(async_fn_in_trait)]
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
#[cfg(feature = "tls")]
use tokio_rustls::client::TlsStream as ClientTlsStream;
#[cfg(feature = "tls")]
use tokio_rustls::server::TlsStream as ServerTlsStream;

pub enum HttpStream {
    Tcp(TcpStream),
    #[cfg(feature = "tls")]
    ServerTls(ServerTlsStream<TcpStream>),
    #[cfg(feature = "tls")]
    ClientTls(ClientTlsStream<TcpStream>),
}
unsafe impl Send for HttpStream {}

impl HttpStream {
    pub fn from_tcp(s: TcpStream) -> Self {
        Self::Tcp(s)
    }

    #[cfg(feature = "tls")]
    pub fn from_server_tls(s: ServerTlsStream<TcpStream>) -> Self {
        Self::ServerTls(s)
    }

    #[cfg(feature = "tls")]
    pub fn from_client_tls(s: ClientTlsStream<TcpStream>) -> Self {
        Self::ClientTls(s)
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.read(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.read(buf).await?,
        })
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read_exact(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.read_exact(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.read_exact(buf).await?,
        })
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        match self {
            HttpStream::Tcp(s) => s.write_all(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.write_all(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.write_all(buf).await?,
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
#[cfg(feature = "tls")]
impl TcpStreamExt for ClientTlsStream<TcpStream> {}
#[cfg(feature = "tls")]
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
    async fn extend_by_streams(&mut self, stream: &mut HttpStream) -> anyhow::Result<usize>;
}

#[async_trait]
impl VecU8Ext for Vec<u8> {
    async fn extend_by_streams(&mut self, stream: &mut HttpStream) -> anyhow::Result<usize> {
        let mut tmp_buf = [0u8; 1024];
        let n = stream.read(&mut tmp_buf).await?;
        if n == 0 {
            return Err(anyhow::Error::msg("connection closed"));
        }
        self.extend(&tmp_buf[0..n]);
        Ok(n)
    }
}
