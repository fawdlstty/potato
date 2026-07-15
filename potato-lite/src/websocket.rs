//! WebSocket frame and handshake helpers (RFC 6455).

use crate::RuntimeInstant;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::hash::{Hash, Hasher};

// ---------------------------------------------------------------------------
// Socket abstraction
// ---------------------------------------------------------------------------

/// Minimal async I/O contract required by the WebSocket implementation.
pub trait WebsocketIo {
    type Error: core::fmt::Debug;

    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl Future<Output = Result<usize, Self::Error>> + 'a;

    fn write<'a>(
        &'a mut self,
        data: &'a [u8],
    ) -> impl Future<Output = Result<usize, Self::Error>> + 'a;

    fn flush(&mut self) -> impl Future<Output = Result<(), Self::Error>> + '_;
}

#[cfg(feature = "std")]
impl WebsocketIo for crate::TcpSocket {
    type Error = std::io::Error;

    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl Future<Output = Result<usize, Self::Error>> + 'a {
        async move { tokio::io::AsyncReadExt::read(self, buf).await }
    }

    fn write<'a>(
        &'a mut self,
        data: &'a [u8],
    ) -> impl Future<Output = Result<usize, Self::Error>> + 'a {
        async move { tokio::io::AsyncWriteExt::write(self, data).await }
    }

    fn flush(&mut self) -> impl Future<Output = Result<(), Self::Error>> + '_ {
        async move { tokio::io::AsyncWriteExt::flush(self).await }
    }
}

// ---------------------------------------------------------------------------
// Frame types
// ---------------------------------------------------------------------------

/// WebSocket data frame exposed to users.
#[derive(Debug)]
pub enum WsFrame {
    Text(String),
    Binary(Vec<u8>),
}

enum WsFrameImpl {
    Close,
    Ping,
    Pong,
    Binary(Vec<u8>),
    Text(Vec<u8>),
    PartData(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Websocket
// ---------------------------------------------------------------------------

/// WebSocket connection backed by a user-provided async socket.
pub struct Websocket<S> {
    socket: S,
}

impl<S> Websocket<S> {
    pub const fn from_socket(socket: S) -> Self {
        Self { socket }
    }

    pub fn into_inner(self) -> S {
        self.socket
    }

    pub fn socket_mut(&mut self) -> &mut S {
        &mut self.socket
    }
}

#[cfg(feature = "std")]
impl Websocket<crate::TcpSocket> {
    /// Establishes a client WebSocket connection over the std/tokio backend.
    pub async fn connect(
        _stack: crate::Stack,
        url: &str,
        _rx_buf: &mut [u8],
        _tx_buf: &mut [u8],
    ) -> anyhow::Result<Self> {
        let (host, port, path) = parse_ws_url(url)?;

        let mut socket =
            <crate::TcpSocket as agnostic_net::TcpStream>::connect((host.as_str(), port))
                .await
                .map_err(|e| anyhow::anyhow!("TCP connect failed: {:?}", e))?;

        client_handshake(&mut socket, &host, port, &path).await?;

        Ok(Self { socket })
    }
}

impl<S: WebsocketIo> Websocket<S> {
    /// Receives a complete WebSocket message.
    pub async fn recv(&mut self) -> anyhow::Result<WsFrame> {
        ws_recv(&mut self.socket).await
    }

    /// Sends a WebSocket frame.
    pub async fn send(&mut self, frame: WsFrame) -> anyhow::Result<()> {
        match frame {
            WsFrame::Binary(data) => {
                send_client_frame(&mut self.socket, WsFrameImpl::Binary(data)).await
            }
            WsFrame::Text(text) => {
                send_client_frame(&mut self.socket, WsFrameImpl::Text(text.into_bytes())).await
            }
        }
    }

    /// Sends a text message.
    pub async fn send_text(&mut self, data: &str) -> anyhow::Result<()> {
        send_client_frame(
            &mut self.socket,
            WsFrameImpl::Text(data.as_bytes().to_vec()),
        )
        .await
    }

    /// Sends binary data.
    pub async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Binary(data)).await
    }

    /// Sends a ping frame.
    pub async fn send_ping(&mut self) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Ping).await
    }

    /// Sends a close frame.
    pub async fn send_close(&mut self) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Close).await
    }
}

// ---------------------------------------------------------------------------
// Client handshake
// ---------------------------------------------------------------------------

/// Writes a client-side WebSocket opening handshake and validates a 101 response.
pub async fn client_handshake<S: WebsocketIo>(
    socket: &mut S,
    host: &str,
    port: u16,
    path: &str,
) -> anyhow::Result<()> {
    let mut key_bytes = [0u8; 16];
    simple_random_bytes(&mut key_bytes);
    let key = base64_encode(&key_bytes);

    let host_header = if port != 80 {
        alloc::format!("{}:{}", host, port)
    } else {
        String::from(host)
    };
    let request = alloc::format!(
        "GET {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: {}\r\n\
         \r\n",
        path,
        host_header,
        key,
    );

    ws_write_all(socket, request.as_bytes()).await?;
    socket_flush(socket).await?;

    let mut buf = [0u8; 1024];
    let mut pos = 0usize;
    loop {
        if pos >= buf.len() {
            return Err(anyhow::anyhow!("response header too large"));
        }
        let n = socket_read(socket, &mut buf[pos..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed during handshake"));
        }
        pos += n;

        if let Some(hdr_end) = find_header_end(&buf[..pos]) {
            let status_line = core::str::from_utf8(&buf[..hdr_end])
                .map_err(|_| anyhow::anyhow!("invalid UTF-8 in handshake response"))?
                .split("\r\n")
                .next()
                .ok_or_else(|| anyhow::anyhow!("no status line in handshake response"))?;
            let mut parts = status_line.splitn(3, ' ');
            let _version = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("no version in handshake status line"))?;
            let code: u16 = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("no status code in handshake status line"))?
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid status code in handshake"))?;
            if code != 101 {
                return Err(anyhow::anyhow!(
                    "server did not accept WebSocket upgrade, got status {}",
                    code
                ));
            }
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// URL parsing
// ---------------------------------------------------------------------------

/// Parses a WebSocket URL in the form `ws://host[:port]/path`.
pub fn parse_ws_url(url: &str) -> anyhow::Result<(String, u16, String)> {
    let rest = if let Some(stripped) = url.strip_prefix("ws://") {
        stripped
    } else {
        return Err(anyhow::anyhow!("WebSocket URL must start with ws://"));
    };

    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };

    let (host, port) = match authority.rfind(':') {
        Some(pos) => {
            let port_str = &authority[pos + 1..];
            let port = port_str
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("invalid port in WebSocket URL: {}", port_str))?;
            (&authority[..pos], port)
        }
        None => (authority, 80u16),
    };

    if host.is_empty() {
        return Err(anyhow::anyhow!("empty host in WebSocket URL"));
    }

    Ok((String::from(host), port, String::from(path)))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

// ---------------------------------------------------------------------------
// Base64 and random seed helpers
// ---------------------------------------------------------------------------

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 2 < data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let remaining = data.len() - i;
    if remaining == 1 {
        let n = (data[i] as u32) << 16;
        out.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if remaining == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        out.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

fn simple_random_bytes(out: &mut [u8]) {
    let mut state = seed_now();
    if state == 0 {
        state = 0x1234_5678_9ABC_DEF0;
    }
    for byte in out.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
}

fn seed_now() -> u64 {
    let mut hasher = SeedHasher(0x1234_5678_9ABC_DEF0);
    <RuntimeInstant as agnostic_lite::time::Instant>::now().hash(&mut hasher);
    hasher.finish()
}

struct SeedHasher(u64);

impl Hasher for SeedHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.rotate_left(13);
            self.0 = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
}

// ---------------------------------------------------------------------------
// Server handshake helpers
// ---------------------------------------------------------------------------

pub fn compute_ws_accept(ws_key: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(ws_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let digest = hasher.finalize();
    base64_encode(&digest)
}

pub fn build_ws_upgrade_response(ws_key: &str) -> Vec<u8> {
    let accept = compute_ws_accept(ws_key);
    let response = alloc::format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept,
    );
    response.into_bytes()
}

// ---------------------------------------------------------------------------
// Frame I/O
// ---------------------------------------------------------------------------

async fn recv_impl<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<WsFrameImpl> {
    let mut hdr = [0u8; 2];
    ws_read_exact(socket, &mut hdr).await?;

    let _fin = hdr[0] & 0b1000_0000 != 0;
    let opcode = hdr[0] & 0b0000_1111;
    let masked = hdr[1] & 0b1000_0000 != 0;
    let mut payload_len: usize = (hdr[1] & 0b0111_1111) as usize;

    match payload_len {
        126 => {
            let mut ext = [0u8; 2];
            ws_read_exact(socket, &mut ext).await?;
            payload_len = u16::from_be_bytes(ext) as usize;
        }
        127 => {
            let mut ext = [0u8; 8];
            ws_read_exact(socket, &mut ext).await?;
            payload_len = u64::from_be_bytes(ext) as usize;
        }
        _ => {}
    }

    let mask_key = if masked {
        let mut mk = [0u8; 4];
        ws_read_exact(socket, &mut mk).await?;
        Some(mk)
    } else {
        None
    };

    let mut payload = Vec::with_capacity(payload_len);
    if payload_len > 0 {
        let mut remaining = payload_len;
        let mut tmp = [0u8; 256];
        while remaining > 0 {
            let chunk = core::cmp::min(remaining, tmp.len());
            ws_read_exact(socket, &mut tmp[..chunk]).await?;
            payload.extend_from_slice(&tmp[..chunk]);
            remaining -= chunk;
        }
    }

    if let Some(mk) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mk[i % 4];
        }
    }

    match opcode {
        0x0 => Ok(WsFrameImpl::PartData(payload)),
        0x1 => Ok(WsFrameImpl::Text(payload)),
        0x2 => Ok(WsFrameImpl::Binary(payload)),
        0x8 => Ok(WsFrameImpl::Close),
        0x9 => Ok(WsFrameImpl::Ping),
        0xA => Ok(WsFrameImpl::Pong),
        _ => Err(anyhow::anyhow!("unsupported WebSocket opcode: {}", opcode)),
    }
}

pub async fn ws_recv<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<WsFrame> {
    let mut accum: Vec<u8> = Vec::new();
    loop {
        match recv_impl(socket).await? {
            WsFrameImpl::Close => return Err(anyhow::anyhow!("WebSocket close frame received")),
            WsFrameImpl::Ping => {
                send_server_frame(socket, WsFrameImpl::Pong).await?;
            }
            WsFrameImpl::Pong => {}
            WsFrameImpl::Binary(data) => {
                accum.extend(data);
                return Ok(WsFrame::Binary(accum));
            }
            WsFrameImpl::Text(data) => {
                accum.extend(data);
                let text = String::from_utf8(accum).unwrap_or_default();
                return Ok(WsFrame::Text(text));
            }
            WsFrameImpl::PartData(data) => {
                accum.extend(data);
            }
        }
    }
}

async fn send_client_frame<S: WebsocketIo>(
    socket: &mut S,
    frame: WsFrameImpl,
) -> anyhow::Result<()> {
    send_frame(socket, frame, true).await
}

async fn send_server_frame<S: WebsocketIo>(
    socket: &mut S,
    frame: WsFrameImpl,
) -> anyhow::Result<()> {
    send_frame(socket, frame, false).await
}

async fn send_frame<S: WebsocketIo>(
    socket: &mut S,
    frame: WsFrameImpl,
    masked: bool,
) -> anyhow::Result<()> {
    let (fin, opcode, payload) = match frame {
        WsFrameImpl::Close => (true, 0x8u8, Vec::new()),
        WsFrameImpl::Ping => (true, 0x9, Vec::new()),
        WsFrameImpl::Pong => (true, 0xA, Vec::new()),
        WsFrameImpl::Binary(data) => (true, 0x2, data),
        WsFrameImpl::Text(data) => (true, 0x1, data),
        WsFrameImpl::PartData(data) => (false, 0x0, data),
    };

    let payload_len = payload.len();
    let mut hdr = Vec::with_capacity(if masked { 14 } else { 10 });
    hdr.push(if fin { 0x80 | opcode } else { opcode });

    let mask_bit = if masked { 0x80 } else { 0 };
    if payload_len < 126 {
        hdr.push((payload_len as u8) | mask_bit);
    } else if payload_len < 65536 {
        hdr.push(126 | mask_bit);
        hdr.extend((payload_len as u16).to_be_bytes());
    } else {
        hdr.push(127 | mask_bit);
        hdr.extend((payload_len as u64).to_be_bytes());
    }

    if masked {
        let mut mask_key = [0u8; 4];
        simple_random_bytes(&mut mask_key);
        hdr.extend_from_slice(&mask_key);
        ws_write_all(socket, &hdr).await?;

        if payload_len > 0 {
            let mut masked_payload = Vec::with_capacity(payload_len);
            for (i, &b) in payload.iter().enumerate() {
                masked_payload.push(b ^ mask_key[i % 4]);
            }
            ws_write_all(socket, &masked_payload).await?;
        }
    } else {
        ws_write_all(socket, &hdr).await?;
        if payload_len > 0 {
            ws_write_all(socket, &payload).await?;
        }
    }

    socket_flush(socket).await
}

pub async fn ws_send_text<S: WebsocketIo>(socket: &mut S, data: &str) -> anyhow::Result<()> {
    send_server_frame(socket, WsFrameImpl::Text(data.as_bytes().to_vec())).await
}

pub async fn ws_send_binary<S: WebsocketIo>(socket: &mut S, data: Vec<u8>) -> anyhow::Result<()> {
    send_server_frame(socket, WsFrameImpl::Binary(data)).await
}

pub async fn ws_send_ping<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<()> {
    send_server_frame(socket, WsFrameImpl::Ping).await
}

pub async fn ws_send_close<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<()> {
    send_server_frame(socket, WsFrameImpl::Close).await
}

async fn ws_read_exact<S: WebsocketIo>(socket: &mut S, buf: &mut [u8]) -> anyhow::Result<()> {
    let mut offset = 0usize;
    while offset < buf.len() {
        let n = socket_read(socket, &mut buf[offset..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed during read"));
        }
        offset += n;
    }
    Ok(())
}

async fn ws_write_all<S: WebsocketIo>(socket: &mut S, data: &[u8]) -> anyhow::Result<()> {
    let mut written = 0usize;
    while written < data.len() {
        let n = socket_write(socket, &data[written..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("write failed: connection closed"));
        }
        written += n;
    }
    Ok(())
}

async fn socket_read<S: WebsocketIo>(socket: &mut S, buf: &mut [u8]) -> anyhow::Result<usize> {
    socket
        .read(buf)
        .await
        .map_err(|e| anyhow::anyhow!("read failed: {:?}", e))
}

async fn socket_write<S: WebsocketIo>(socket: &mut S, data: &[u8]) -> anyhow::Result<usize> {
    socket
        .write(data)
        .await
        .map_err(|e| anyhow::anyhow!("write failed: {:?}", e))
}

async fn socket_flush<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<()> {
    socket
        .flush()
        .await
        .map_err(|e| anyhow::anyhow!("flush failed: {:?}", e))
}
