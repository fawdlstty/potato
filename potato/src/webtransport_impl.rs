//! WebTransport 实现

use anyhow::Result;
use quinn::{Connection, RecvStream, SendStream, VarInt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// WebTransport 会话
pub struct WebTransportSession {
    inner: Arc<Mutex<Connection>>,
    remote_addr: SocketAddr,
}

impl WebTransportSession {
    pub(crate) fn new(connection: Connection) -> Self {
        let remote_addr = connection.remote_address();
        Self {
            inner: Arc::new(Mutex::new(connection)),
            remote_addr,
        }
    }

    /// 接受一个新的双向流
    pub async fn accept_bi(&self) -> Result<Option<WebTransportStream>> {
        let inner = self.inner.lock().await;
        match inner.accept_bi().await {
            Ok((send, recv)) => Ok(Some(WebTransportStream::new(send, recv))),
            Err(quinn::ConnectionError::ApplicationClosed(_)) => Ok(None),
            Err(quinn::ConnectionError::ConnectionClosed(_)) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to accept bidirectional stream: {}",
                e
            )),
        }
    }

    /// 接收数据报
    pub async fn recv_datagram(&self) -> Result<Vec<u8>> {
        let inner = self.inner.lock().await;
        let data = inner.read_datagram().await?;
        Ok(data.to_vec())
    }

    /// 发送数据报
    pub async fn send_datagram(&self, data: &[u8]) -> Result<()> {
        let inner = self.inner.lock().await;
        inner.send_datagram(data.to_vec().into())?;
        Ok(())
    }

    /// 获取远程地址
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// 关闭会话
    pub async fn close(&self, error_code: u32, reason: &str) -> Result<()> {
        let inner = self.inner.lock().await;
        inner.close(VarInt::from_u32(error_code), reason.as_bytes());
        Ok(())
    }
}

/// WebTransport 双向流
pub struct WebTransportStream {
    send: SendStream,
    recv: RecvStream,
}

impl WebTransportStream {
    pub(crate) fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }

    /// 发送数据
    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.send.write_all(data).await?;
        Ok(())
    }

    /// 接收数据
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        while let Some(chunk) = self.recv.read_chunk(usize::MAX, false).await? {
            buf.extend_from_slice(&chunk.bytes);
        }
        Ok(buf)
    }
}
