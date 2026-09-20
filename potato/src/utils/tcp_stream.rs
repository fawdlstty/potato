#![allow(async_fn_in_trait)]
use async_trait::async_trait;
use std::io::IoSlice;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
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
    /// 速率限制的流
    RateLimited(RateLimitedStream),
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
                            // 不应该发生，WithPreRead不应该嵌套；
                            // 返回错误避免被当作 EOF 静默丢数据
                            return Err(anyhow::Error::msg(
                                "nested WithPreRead stream is not supported",
                            ));
                        }
                        HttpStream::RateLimited(s) => Box::pin(s.read(buf)).await?,
                    }
                }
            }
            HttpStream::RateLimited(s) => Box::pin(s.read(buf)).await?,
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
                    // read_exact semantics: keep draining the pre-read buffer
                    // (then the underlying stream) until buf is full.
                    let mut filled = std::cmp::min(pre_read_data.len(), buf.len());
                    buf[..filled].copy_from_slice(&pre_read_data[..filled]);
                    pre_read_data.drain(..filled);
                    while filled < buf.len() {
                        let n = match stream.as_mut() {
                            HttpStream::Tcp(s) => s.read(&mut buf[filled..]).await?,
                            #[cfg(feature = "tls")]
                            HttpStream::ServerTls(s) => s.read(&mut buf[filled..]).await?,
                            #[cfg(feature = "tls")]
                            HttpStream::ClientTls(s) => s.read(&mut buf[filled..]).await?,
                            HttpStream::DuplexStream(s) => s.read(&mut buf[filled..]).await?,
                            HttpStream::WithPreRead { .. } => {
                                return Err(anyhow::Error::msg(
                                    "nested WithPreRead stream is not supported",
                                ));
                            }
                            HttpStream::RateLimited(s) => {
                                Box::pin(s.read(&mut buf[filled..])).await?
                            }
                        };
                        if n == 0 {
                            return Err(anyhow::Error::msg("connection closed"));
                        }
                        filled += n;
                    }
                    buf.len()
                } else {
                    match stream.as_mut() {
                        HttpStream::Tcp(s) => s.read_exact(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ServerTls(s) => s.read_exact(buf).await?,
                        #[cfg(feature = "tls")]
                        HttpStream::ClientTls(s) => s.read_exact(buf).await?,
                        HttpStream::DuplexStream(s) => s.read_exact(buf).await?,
                        HttpStream::WithPreRead { .. } => {
                            return Err(anyhow::Error::msg(
                                "nested WithPreRead stream is not supported",
                            ));
                        }
                        HttpStream::RateLimited(s) => {
                            // RateLimitedStream 没有 read_exact，需要自己实现
                            let mut read = 0;
                            while read < buf.len() {
                                let n = s.read(&mut buf[read..]).await?;
                                if n == 0 {
                                    return Err(anyhow::Error::msg("connection closed"));
                                }
                                read += n;
                            }
                            read
                        }
                    }
                }
            }
            HttpStream::RateLimited(s) => {
                let mut read = 0;
                while read < buf.len() {
                    let n = s.read(&mut buf[read..]).await?;
                    if n == 0 {
                        return Err(anyhow::Error::msg("connection closed"));
                    }
                    read += n;
                }
                read
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
                HttpStream::WithPreRead { .. } => return Err(anyhow::Error::msg("nested WithPreRead stream is not supported")),
                HttpStream::RateLimited(s) => Box::pin(s.write_all(buf)).await?,
            },
            HttpStream::RateLimited(s) => Box::pin(s.write_all(buf)).await?,
        }
        Ok(())
    }

    /// flush 必须在 `write_all` 之后由调用方显式执行：tokio-rustls 的
    /// AsyncWrite 允许 `write_all` 返回时仍有密文滞留内部缓冲（socket 发送
    /// 缓冲写满后不再前进），尾部数据若不 flush 永不出网——服务端按
    /// Content-Length 等不到完整请求体，实测 open.bigmodel.cn 对 >~64KiB 的
    /// 请求长挂 120s 后回 502（WorkRecorder BYOK 分析超时事故根因，2026-09-16）。
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt as _;
        match self {
            HttpStream::Tcp(s) => s.flush().await?,
            #[cfg(feature = "tls")]
            HttpStream::ServerTls(s) => s.flush().await?,
            #[cfg(feature = "tls")]
            HttpStream::ClientTls(s) => s.flush().await?,
            HttpStream::DuplexStream(s) => s.flush().await?,
            // WithPreRead / RateLimited 只出现在服务端 / ACME 路径，客户端
            // 请求发送不经过它们
            HttpStream::WithPreRead { .. } | HttpStream::RateLimited(_) => {}
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
                HttpStream::WithPreRead { .. } => return Err(anyhow::Error::msg("nested WithPreRead stream is not supported")),
                HttpStream::RateLimited(s) => {
                    for buf in bufs {
                        s.write_all(buf).await?;
                    }
                }
            },
            HttpStream::RateLimited(s) => {
                for buf in bufs {
                    s.write_all(buf).await?;
                }
            }
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
                HttpStream::WithPreRead { .. } => return Err(anyhow::Error::msg("nested WithPreRead stream is not supported")),
                HttpStream::RateLimited(s) => {
                    s.write_all(a).await?;
                    s.write_all(b).await?;
                }
            },
            HttpStream::RateLimited(s) => {
                s.write_all(a).await?;
                s.write_all(b).await?;
            }
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

/// # Safety
/// 此trait用于将裸指针转换为可变引用，调用者必须保证指针的有效性
pub unsafe trait TcpStreamExt2 {
    /// # Safety
    /// 调用者必须保证指针指向有效的TcpStreamExt对象
    unsafe fn get_mut(self) -> &'static mut dyn TcpStreamExt;
}

unsafe impl TcpStreamExt2 for *mut dyn TcpStreamExt {
    unsafe fn get_mut(self) -> &'static mut dyn TcpStreamExt {
        &mut *self as &mut dyn TcpStreamExt
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

/// 速率限制器 - 使用令牌桶算法
pub struct RateLimiter {
    /// 最大速率 (bits/sec)
    max_rate_bits_per_sec: u64,
    /// 当前令牌数 (bits)
    tokens: f64,
    /// 上次更新时间
    last_update: Instant,
}

impl RateLimiter {
    pub fn new(max_rate_bits_per_sec: u64) -> Self {
        Self {
            max_rate_bits_per_sec,
            tokens: (max_rate_bits_per_sec as f64) / 10.0, // 初始令牌：1/10秒的量
            last_update: Instant::now(),
        }
    }

    /// 获取等待时间（如果需要限速）
    pub fn acquire(&mut self, bits: u64) -> Option<Duration> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        self.last_update = now;

        // 补充令牌
        let new_tokens = (self.max_rate_bits_per_sec as f64) * elapsed.as_secs_f64();
        self.tokens = (self.tokens + new_tokens).min(self.max_rate_bits_per_sec as f64);

        let bits_f64 = bits as f64;
        if self.tokens >= bits_f64 {
            self.tokens -= bits_f64;
            None // 不需要等待
        } else {
            let deficit = bits_f64 - self.tokens;
            let wait_secs = deficit / (self.max_rate_bits_per_sec as f64);
            Some(Duration::from_secs_f64(wait_secs))
        }
    }

    /// 预先扣减可用令牌，返回实际允许传输的位数
    pub fn reserve(&mut self, max_bits: u64) -> u64 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        self.last_update = now;
        self.tokens = (self.tokens + self.max_rate_bits_per_sec as f64 * elapsed.as_secs_f64())
            .min(self.max_rate_bits_per_sec as f64);

        let available = self.tokens.floor().max(0.0) as u64;
        let bits = if max_bits >= 8 && available < 8 {
            0
        } else {
            max_bits.min(available)
        };
        self.tokens -= bits as f64;
        bits
    }

    /// 获取下一个可传输字节就绪的等待时间
    pub fn wait_time_for_token(&self) -> Duration {
        let deficit = if self.tokens >= 8.0 {
            0.0
        } else {
            8.0 - self.tokens
        };
        Duration::from_secs_f64(deficit / self.max_rate_bits_per_sec as f64)
    }
}

/// 速率限制的 HttpStream 包装器
pub struct RateLimitedStream {
    stream: Box<HttpStream>,
    inbound_limiter: Arc<Mutex<RateLimiter>>,
    outbound_limiter: Arc<Mutex<RateLimiter>>,
}

/// 统一的流类型，支持速率限制和普通流
pub enum UnifiedStream {
    Normal(Box<HttpStream>),
    RateLimited(RateLimitedStream),
}

impl UnifiedStream {
    pub async fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self {
            UnifiedStream::Normal(s) => s.read(buf).await,
            UnifiedStream::RateLimited(s) => s.read(buf).await,
        }
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        match self {
            UnifiedStream::Normal(s) => s.write_all(buf).await,
            UnifiedStream::RateLimited(s) => s.write_all(buf).await,
        }
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match self {
            UnifiedStream::Normal(s) => s.read_exact(buf).await,
            UnifiedStream::RateLimited(s) => {
                // RateLimitedStream 没有 read_exact，需要自己实现
                let mut read = 0;
                while read < buf.len() {
                    let n = s.read(&mut buf[read..]).await?;
                    if n == 0 {
                        return Err(anyhow::Error::msg("connection closed"));
                    }
                    read += n;
                }
                Ok(read)
            }
        }
    }

    pub async fn write_all_vectored(&mut self, bufs: &[IoSlice<'_>]) -> anyhow::Result<()> {
        match self {
            UnifiedStream::Normal(s) => s.write_all_vectored(bufs).await,
            UnifiedStream::RateLimited(s) => {
                // 对于速率限制，我们简单地顺序写入
                for buf in bufs {
                    s.write_all(buf).await?;
                }
                Ok(())
            }
        }
    }

    pub async fn write_all_vectored2(&mut self, a: &[u8], b: &[u8]) -> anyhow::Result<()> {
        match self {
            UnifiedStream::Normal(s) => s.write_all_vectored2(a, b).await,
            UnifiedStream::RateLimited(s) => {
                s.write_all(a).await?;
                s.write_all(b).await
            }
        }
    }

    /// 转换为 HttpStream（用于兼容旧接口）
    pub fn into_http_stream(self) -> HttpStream {
        match self {
            UnifiedStream::Normal(s) => *s,
            UnifiedStream::RateLimited(s) => s.into_inner(),
        }
    }

    /// 从 HttpStream 创建
    pub fn from_http_stream(stream: HttpStream) -> Self {
        UnifiedStream::Normal(Box::new(stream))
    }
}

impl RateLimitedStream {
    pub fn new(
        stream: HttpStream,
        inbound_rate_bits_per_sec: u64,
        outbound_rate_bits_per_sec: u64,
    ) -> Self {
        Self {
            stream: Box::new(stream),
            inbound_limiter: Arc::new(Mutex::new(RateLimiter::new(inbound_rate_bits_per_sec))),
            outbound_limiter: Arc::new(Mutex::new(RateLimiter::new(outbound_rate_bits_per_sec))),
        }
    }

    pub fn into_inner(self) -> HttpStream {
        *self.stream
    }

    /// 创建共享的速率限制器（用于双向限速）
    pub fn new_shared(
        stream: HttpStream,
        inbound_rate_bits_per_sec: u64,
        outbound_rate_bits_per_sec: u64,
    ) -> (Self, Arc<Mutex<RateLimiter>>, Arc<Mutex<RateLimiter>>) {
        let inbound_limiter = Arc::new(Mutex::new(RateLimiter::new(inbound_rate_bits_per_sec)));
        let outbound_limiter = Arc::new(Mutex::new(RateLimiter::new(outbound_rate_bits_per_sec)));
        let limited_stream = Self {
            stream: Box::new(stream),
            inbound_limiter: inbound_limiter.clone(),
            outbound_limiter: outbound_limiter.clone(),
        };
        (limited_stream, inbound_limiter, outbound_limiter)
    }

    pub fn from_shared(
        stream: HttpStream,
        inbound_limiter: Arc<Mutex<RateLimiter>>,
        outbound_limiter: Arc<Mutex<RateLimiter>>,
    ) -> Self {
        Self {
            stream: Box::new(stream),
            inbound_limiter,
            outbound_limiter,
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let admitted_bits = loop {
            let mut limiter = self.inbound_limiter.lock().await;
            let bits = limiter.reserve((buf.len() * 8) as u64);
            if bits >= 8 {
                break bits;
            }
            let wait_time = limiter.wait_time_for_token();
            drop(limiter);
            tokio::time::sleep(wait_time).await;
        };

        let admitted_bytes = (admitted_bits / 8) as usize;

        self.stream.read(&mut buf[..admitted_bytes]).await
    }

    pub async fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let admitted_bits = loop {
                let mut limiter = self.outbound_limiter.lock().await;
                let bits = limiter.reserve(((buf.len() - offset) * 8) as u64);
                if bits >= 8 {
                    break bits;
                }
                let wait_time = limiter.wait_time_for_token();
                drop(limiter);
                tokio::time::sleep(wait_time).await;
            };

            let admitted_bytes = (admitted_bits / 8) as usize;
            self.stream
                .write_all(&buf[offset..offset + admitted_bytes])
                .await?;
            offset += admitted_bytes;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;
    use std::time::Duration;

    #[test]
    fn reserve_truncates_to_available_tokens() {
        let mut limiter = RateLimiter::new(8_000);
        assert_eq!(limiter.reserve(1_600), 800);
        // 8 kb/s 下相邻语句间的调度间隔就可能补入令牌（1 bit 仅需 0.125ms），
        // 断言"再次整笔请求只会拿到远小于请求量的截断值"并预留约 100ms 的
        // 调度容差，而不是要求恰好为 0。
        assert!(limiter.reserve(8_000) < 800);
    }

    #[tokio::test]
    async fn reserve_refills_without_bulk_admission() {
        let mut limiter = RateLimiter::new(8_000);
        assert_eq!(limiter.reserve(800), 800);

        // 同上：排空初始令牌后不允许立即再次整笔放行；按 8 kb/s 计算，仅在
        // 相邻语句间出现 100ms 以上的调度停顿才会误报，容差约为此前的 800 倍。
        assert!(limiter.reserve(8_000) < 800);

        tokio::time::sleep(Duration::from_millis(125)).await;
        assert_eq!(limiter.reserve(800), 800);
    }
}

