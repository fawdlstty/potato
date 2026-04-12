#![allow(async_fn_in_trait)]
use async_trait::async_trait;
use std::io::IoSlice;
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
    DuplexStream(tokio::io::DuplexStream),
    /// 带预读取缓冲区的流，用于ACME挑战检测后继续处理HTTP请求
    WithPreRead {
        stream: Box<HttpStream>,
        pre_read_data: Vec<u8>,
    },
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

    pub fn from_duplex_stream(stream: tokio::io::DuplexStream) -> Self {
        HttpStream::DuplexStream(stream)
    }

    /// 创建一个带预读取缓冲区的HttpStream
    pub fn with_pre_read(stream: HttpStream, pre_read_data: Vec<u8>) -> Self {
        HttpStream::WithPreRead {
            stream: Box::new(stream),
            pre_read_data,
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.read(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.read(buf).await?,
            HttpStream::DuplexStream(s) => s.read(buf).await?,
            HttpStream::WithPreRead {
                stream,
                pre_read_data,
            } => {
                if !pre_read_data.is_empty() {
                    let len = std::cmp::min(pre_read_data.len(), buf.len());
                    buf[..len].copy_from_slice(&pre_read_data[..len]);
                    pre_read_data.drain(..len);
                    len
                } else {
                    // 使用内部match避免递归调用
                    match stream.as_mut() {
                        HttpStream::Tcp(s) => s.read(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ServerTls(s) => s.read(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ClientTls(s) => s.read(buf).await?,
                        HttpStream::DuplexStream(s) => s.read(buf).await?,
                        HttpStream::WithPreRead { .. } => {
                            // 不应该发生，WithPreRead不应该嵌套
                            0
                        }
                    }
                }
            }
        })
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(match self {
            HttpStream::Tcp(s) => s.read_exact(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.read_exact(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.read_exact(buf).await?,
            HttpStream::DuplexStream(s) => s.read_exact(buf).await?,
            HttpStream::WithPreRead {
                stream,
                pre_read_data,
            } => {
                if !pre_read_data.is_empty() {
                    let len = std::cmp::min(pre_read_data.len(), buf.len());
                    buf[..len].copy_from_slice(&pre_read_data[..len]);
                    pre_read_data.drain(..len);
                    len
                } else {
                    match stream.as_mut() {
                        HttpStream::Tcp(s) => s.read_exact(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ServerTls(s) => s.read_exact(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ClientTls(s) => s.read_exact(buf).await?,
                        HttpStream::DuplexStream(s) => s.read_exact(buf).await?,
                        HttpStream::WithPreRead { .. } => 0,
                    }
                }
            }
        })
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        match self {
            HttpStream::Tcp(s) => s.write_all(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.write_all(buf).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.write_all(buf).await?,
            HttpStream::DuplexStream(s) => s.write_all(buf).await?,
            HttpStream::WithPreRead { stream, .. } => match stream.as_mut() {
                HttpStream::Tcp(s) => s.write_all(buf).await?,
                #[cfg(feature = "tls")]
                HttpStream::ServerTls(s) => s.write_all(buf).await?,
                #[cfg(feature = "tls")]
                HttpStream::ClientTls(s) => s.write_all(buf).await?,
                HttpStream::DuplexStream(s) => s.write_all(buf).await?,
                HttpStream::WithPreRead { .. } => {}
            },
        }
        Ok(())
    }

    pub async fn write_all_vectored(&mut self, bufs: &[IoSlice<'_>]) -> anyhow::Result<()> {
        if bufs.is_empty() {
            return Ok(());
        }
        match self {
            HttpStream::Tcp(s) => write_all_vectored_inner(s, bufs).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => write_all_vectored_inner(s, bufs).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => write_all_vectored_inner(s, bufs).await?,
            HttpStream::DuplexStream(s) => write_all_vectored_inner(s, bufs).await?,
            HttpStream::WithPreRead { stream, .. } => match stream.as_mut() {
                HttpStream::Tcp(s) => write_all_vectored_inner(s, bufs).await?,
                #[cfg(feature = "tls")]
                HttpStream::ServerTls(s) => write_all_vectored_inner(s, bufs).await?,
                #[cfg(feature = "tls")]
                HttpStream::ClientTls(s) => write_all_vectored_inner(s, bufs).await?,
                HttpStream::DuplexStream(s) => write_all_vectored_inner(s, bufs).await?,
                HttpStream::WithPreRead { .. } => {}
            },
        }
        Ok(())
    }

    pub async fn write_all_vectored2(&mut self, a: &[u8], b: &[u8]) -> anyhow::Result<()> {
        match self {
            HttpStream::Tcp(s) => write_all_vectored2_inner(s, a, b).await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => write_all_vectored2_inner(s, a, b).await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => write_all_vectored2_inner(s, a, b).await?,
            HttpStream::DuplexStream(s) => write_all_vectored2_inner(s, a, b).await?,
            HttpStream::WithPreRead { stream, .. } => match stream.as_mut() {
                HttpStream::Tcp(s) => write_all_vectored2_inner(s, a, b).await?,
                #[cfg(feature = "tls")]
                HttpStream::ServerTls(s) => write_all_vectored2_inner(s, a, b).await?,
                #[cfg(feature = "tls")]
                HttpStream::ClientTls(s) => write_all_vectored2_inner(s, a, b).await?,
                HttpStream::DuplexStream(s) => write_all_vectored2_inner(s, a, b).await?,
                HttpStream::WithPreRead { .. } => {}
            },
        }
        Ok(())
    }
}

async fn write_all_vectored_inner<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bufs: &[IoSlice<'_>],
) -> anyhow::Result<()> {
    let mut idx = 0usize;
    let mut offset = 0usize;
    while idx < bufs.len() {
        let mut slices = Vec::with_capacity(bufs.len() - idx);
        if offset > 0 {
            slices.push(IoSlice::new(&bufs[idx][offset..]));
            for b in &bufs[idx + 1..] {
                slices.push(IoSlice::new(b));
            }
        } else {
            for b in &bufs[idx..] {
                slices.push(IoSlice::new(b));
            }
        }

        let n = writer.write_vectored(&slices).await?;
        if n == 0 {
            return Err(anyhow::Error::msg("connection closed while writing"));
        }

        let mut written = n;
        if offset > 0 {
            let rem = bufs[idx].len() - offset;
            if written < rem {
                offset += written;
                continue;
            }
            written -= rem;
            idx += 1;
            offset = 0;
        }
        while idx < bufs.len() && written >= bufs[idx].len() {
            written -= bufs[idx].len();
            idx += 1;
        }
        if idx < bufs.len() && written > 0 {
            offset = written;
        }
    }
    Ok(())
}

async fn write_all_vectored2_inner<W: AsyncWrite + Unpin>(
    writer: &mut W,
    a: &[u8],
    b: &[u8],
) -> anyhow::Result<()> {
    let mut a_off = 0usize;
    let mut b_off = 0usize;
    loop {
        if a_off >= a.len() && b_off >= b.len() {
            return Ok(());
        }
        let n = if a_off < a.len() && b_off < b.len() {
            let bufs = [IoSlice::new(&a[a_off..]), IoSlice::new(&b[b_off..])];
            writer.write_vectored(&bufs).await?
        } else if a_off < a.len() {
            writer.write(&a[a_off..]).await?
        } else {
            writer.write(&b[b_off..]).await?
        };
        if n == 0 {
            return Err(anyhow::Error::msg("connection closed while writing"));
        }

        if a_off < a.len() {
            let a_rem = a.len() - a_off;
            if n < a_rem {
                a_off += n;
                continue;
            }
            a_off = a.len();
            b_off += n - a_rem;
        } else {
            b_off += n;
        }
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
