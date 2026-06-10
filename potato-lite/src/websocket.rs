//! WebSocket 客户端实现（RFC 6455）
//!
//! 用法与 potato 项目的 `Websocket` 一致，底层使用 embassy-net TcpSocket 传输。

use alloc::string::String;
use alloc::vec::Vec;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpAddress, IpEndpoint, Ipv4Address, Stack};

// ---------------------------------------------------------------------------
// 帧类型
// ---------------------------------------------------------------------------

/// WebSocket 数据帧（用户可见）
#[derive(Debug)]
pub enum WsFrame {
    /// 文本消息
    Text(String),
    /// 二进制消息
    Binary(Vec<u8>),
}

/// WebSocket 内部帧类型
pub(crate) enum WsFrameImpl {
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

/// WebSocket 连接，用法参考 potato 项目 `Websocket`
pub struct Websocket<'d> {
    socket: TcpSocket<'d>,
}

impl<'d> Websocket<'d> {
    // -----------------------------------------------------------------------
    // 客户端 connect
    // -----------------------------------------------------------------------

    /// 建立 WebSocket 连接
    ///
    /// # 参数
    /// * `stack`  - embassy-net 网络栈（`Copy`）
    /// * `url`    - WebSocket URL，格式: `ws://host[:port]/path`
    /// * `rx_buf` - 接收缓冲区（由调用方提供，生命周期需覆盖整个连接）
    /// * `tx_buf` - 发送缓冲区（由调用方提供，生命周期需覆盖整个连接）
    ///
    /// # 示例
    /// ```ignore
    /// let mut rx = [0u8; 4096];
    /// let mut tx = [0u8; 4096];
    /// let mut ws = potato_lite::websocket::Websocket::connect(stack, "ws://192.168.1.1/ws", &mut rx, &mut tx).await?;
    /// ws.send_text("hello").await?;
    /// ```
    pub async fn connect(
        stack: Stack<'d>,
        url: &str,
        rx_buf: &'d mut [u8],
        tx_buf: &'d mut [u8],
    ) -> Result<Self, &'static str> {
        // 1. 解析 URL
        let (host, port, path) = parse_ws_url(url)?;

        // 2. DNS 解析 + TCP 连接
        let addr = resolve_addr(stack, &host, port).await?;
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket
            .connect(addr)
            .await
            .map_err(|_| "TCP connect failed")?;

        // 3. 生成 Sec-WebSocket-Key（16 字节随机数据的 base64 编码）
        let mut key_bytes = [0u8; 16];
        simple_random_bytes(&mut key_bytes);
        let key = base64_encode(&key_bytes);

        // 4. 构造并发送 HTTP 升级请求
        let host_header = if port != 80 {
            alloc::format!("{}:{}", host, port)
        } else {
            host.clone()
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
        let req_bytes = request.as_bytes();
        let mut written = 0usize;
        while written < req_bytes.len() {
            let n = socket
                .write(&req_bytes[written..])
                .await
                .map_err(|_| "write failed")?;
            if n == 0 {
                return Err("write failed");
            }
            written += n;
        }
        let _ = socket.flush().await;

        // 5. 读取响应，检查 101 状态
        let mut buf = [0u8; 1024];
        let mut pos = 0usize;
        loop {
            if pos >= buf.len() {
                return Err("response header too large");
            }
            let n = socket
                .read(&mut buf[pos..])
                .await
                .map_err(|_| "read failed")?;
            if n == 0 {
                return Err("connection closed during handshake");
            }
            pos += n;
            if let Some(hdr_end) = find_header_end(&buf[..pos]) {
                // 解析状态行
                let status_line = core::str::from_utf8(&buf[..hdr_end])
                    .map_err(|_| "invalid UTF-8")?
                    .split("\r\n")
                    .next()
                    .ok_or("no status line")?;
                // "HTTP/1.1 101 Switching Protocols"
                let mut parts = status_line.splitn(3, ' ');
                let _ver = parts.next().ok_or("no version")?;
                let code: u16 = parts
                    .next()
                    .ok_or("no status code")?
                    .parse()
                    .map_err(|_| "invalid status code")?;
                if code != 101 {
                    return Err("server did not accept WebSocket upgrade");
                }
                // hdr_end 即为头部结束位置（\r\n\r\n 之后），握手完成
                break;
            }
        }

        Ok(Websocket { socket })
    }

    // -----------------------------------------------------------------------
    // 服务端升级（由 server.rs 调用）
    // -----------------------------------------------------------------------

    /// 从已完成的 HTTP 握手中构造 Websocket 实例（服务端使用）
    #[allow(dead_code)]
    pub(crate) fn from_socket(socket: TcpSocket<'d>) -> Self {
        Self { socket }
    }

    // -----------------------------------------------------------------------
    // 帧读取
    // -----------------------------------------------------------------------

    /// 读取单个 WebSocket 帧（内部实现）
    async fn recv_impl(&mut self) -> Result<WsFrameImpl, &'static str> {
        // 读取 2 字节头部
        let mut hdr = [0u8; 2];
        self.read_exact(&mut hdr).await?;

        let _fin = hdr[0] & 0b1000_0000 != 0;
        let opcode = hdr[0] & 0b0000_1111;
        let masked = hdr[1] & 0b1000_0000 != 0;
        let mut payload_len: usize = (hdr[1] & 0b0111_1111) as usize;

        // 扩展长度
        match payload_len {
            126 => {
                let mut ext = [0u8; 2];
                self.read_exact(&mut ext).await?;
                payload_len = u16::from_be_bytes(ext) as usize;
            }
            127 => {
                let mut ext = [0u8; 8];
                self.read_exact(&mut ext).await?;
                payload_len = u64::from_be_bytes(ext) as usize;
            }
            _ => {}
        }

        // 读取 mask key（如果有）
        let mask_key = if masked {
            let mut mk = [0u8; 4];
            self.read_exact(&mut mk).await?;
            Some(mk)
        } else {
            None
        };

        // 读取 payload
        let mut payload = Vec::with_capacity(payload_len);
        if payload_len > 0 {
            let mut remaining = payload_len;
            let mut tmp = [0u8; 256];
            while remaining > 0 {
                let chunk = core::cmp::min(remaining, tmp.len());
                self.read_exact(&mut tmp[..chunk]).await?;
                payload.extend_from_slice(&tmp[..chunk]);
                remaining -= chunk;
            }
        }

        // 反掩码
        if let Some(mk) = mask_key {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mk[i % 4];
            }
        }

        // 返回帧
        match opcode {
            0x0 => Ok(WsFrameImpl::PartData(payload)),
            0x1 => Ok(WsFrameImpl::Text(payload)),
            0x2 => Ok(WsFrameImpl::Binary(payload)),
            0x8 => Ok(WsFrameImpl::Close),
            0x9 => Ok(WsFrameImpl::Ping),
            0xA => Ok(WsFrameImpl::Pong),
            _ => Err("unsupported WebSocket opcode"),
        }
    }

    /// 接收一条完整的 WebSocket 消息（自动处理 Ping/Pong 和分段数据）
    ///
    /// # 示例
    /// ```ignore
    /// match ws.recv().await? {
    ///     WsFrame::Text(text) => { /* 处理文本 */ }
    ///     WsFrame::Binary(data) => { /* 处理二进制数据 */ }
    /// }
    /// ```
    pub async fn recv(&mut self) -> Result<WsFrame, &'static str> {
        let mut accum: Vec<u8> = Vec::new();
        loop {
            match self.recv_impl().await? {
                WsFrameImpl::Close => return Err("WebSocket close frame"),
                WsFrameImpl::Ping => {
                    self.send_impl_frame(WsFrameImpl::Pong).await?;
                }
                WsFrameImpl::Pong => {
                    // 忽略 Pong，继续等待数据
                }
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
                    // 分段数据，继续读取
                    accum.extend(data);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 帧发送
    // -----------------------------------------------------------------------

    /// 构造并发送一个 WebSocket 帧（内部实现）
    ///
    /// 客户端发送的帧必须设置 MASK bit（RFC 6455 §5.1）
    async fn send_impl_frame(&mut self, frame: WsFrameImpl) -> Result<(), &'static str> {
        let (fin, opcode, payload) = match frame {
            WsFrameImpl::Close => (true, 0x8u8, Vec::new()),
            WsFrameImpl::Ping => (true, 0x9, Vec::new()),
            WsFrameImpl::Pong => (true, 0xA, Vec::new()),
            WsFrameImpl::Binary(data) => (true, 0x2, data),
            WsFrameImpl::Text(data) => (true, 0x1, data),
            WsFrameImpl::PartData(data) => (false, 0x0, data),
        };

        let payload_len = payload.len();

        // 构造帧头部
        let mut hdr = Vec::with_capacity(14); // 最多 2 + 8 + 4
        hdr.push(if fin { 0x80 | opcode } else { opcode });

        // 客户端帧必须设置 mask bit（0x80）
        if payload_len < 126 {
            hdr.push((payload_len as u8) | 0x80);
        } else if payload_len < 65536 {
            hdr.push(126 | 0x80);
            hdr.extend((payload_len as u16).to_be_bytes());
        } else {
            hdr.push(127 | 0x80);
            hdr.extend((payload_len as u64).to_be_bytes());
        }

        // 生成 mask key
        let mut mask_key = [0u8; 4];
        simple_random_bytes(&mut mask_key);
        hdr.extend_from_slice(&mask_key);

        // 写入头部
        self.write_all(&hdr).await?;

        // 掩码 payload 并写入
        if payload_len > 0 {
            let mut masked = Vec::with_capacity(payload_len);
            for (i, &b) in payload.iter().enumerate() {
                masked.push(b ^ mask_key[i % 4]);
            }
            self.write_all(&masked).await?;
        }

        let _ = self.socket.flush().await;
        Ok(())
    }

    /// 发送 WebSocket 帧
    ///
    /// # 示例
    /// ```ignore
    /// ws.send(WsFrame::Text("hello".into())).await?;
    /// ```
    pub async fn send(&mut self, frame: WsFrame) -> Result<(), &'static str> {
        match frame {
            WsFrame::Binary(data) => self.send_impl_frame(WsFrameImpl::Binary(data)).await,
            WsFrame::Text(text) => {
                self.send_impl_frame(WsFrameImpl::Text(text.into_bytes()))
                    .await
            }
        }
    }

    /// 发送文本消息
    ///
    /// # 示例
    /// ```ignore
    /// ws.send_text("hello world").await?;
    /// ```
    pub async fn send_text(&mut self, data: &str) -> Result<(), &'static str> {
        self.send_impl_frame(WsFrameImpl::Text(data.as_bytes().to_vec()))
            .await
    }

    /// 发送二进制数据
    ///
    /// # 示例
    /// ```ignore
    /// ws.send_binary(vec![1, 2, 3]).await?;
    /// ```
    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<(), &'static str> {
        self.send_impl_frame(WsFrameImpl::Binary(data)).await
    }

    /// 发送 Ping 帧
    ///
    /// # 示例
    /// ```ignore
    /// ws.send_ping().await?;
    /// ```
    pub async fn send_ping(&mut self) -> Result<(), &'static str> {
        self.send_impl_frame(WsFrameImpl::Ping).await
    }

    /// 发送 Close 帧（主动关闭连接）
    pub async fn send_close(&mut self) -> Result<(), &'static str> {
        self.send_impl_frame(WsFrameImpl::Close).await
    }

    // -----------------------------------------------------------------------
    // 底层 IO 辅助
    // -----------------------------------------------------------------------

    /// 精确读取 n 字节
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), &'static str> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let n = self
                .socket
                .read(&mut buf[offset..])
                .await
                .map_err(|_| "read error")?;
            if n == 0 {
                return Err("connection closed");
            }
            offset += n;
        }
        Ok(())
    }

    /// 写入所有字节
    async fn write_all(&mut self, data: &[u8]) -> Result<(), &'static str> {
        let mut written = 0usize;
        while written < data.len() {
            let n = self
                .socket
                .write(&data[written..])
                .await
                .map_err(|_| "write error")?;
            if n == 0 {
                return Err("write failed");
            }
            written += n;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// URL 解析
// ---------------------------------------------------------------------------

/// 解析 WebSocket URL，格式: `ws://host[:port]/path`
fn parse_ws_url(url: &str) -> Result<(String, u16, String), &'static str> {
    let rest = if let Some(stripped) = url.strip_prefix("ws://") {
        stripped
    } else {
        return Err("WebSocket URL must start with ws://");
    };

    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };

    let (host, port) = match authority.rfind(':') {
        Some(pos) => {
            let port_str = &authority[pos + 1..];
            let port = port_str.parse::<u16>().map_err(|_| "invalid port")?;
            (&authority[..pos], port)
        }
        None => (authority, 80u16),
    };

    if host.is_empty() {
        return Err("empty host");
    }

    Ok((String::from(host), port, String::from(path)))
}

/// 将主机名解析为 IpEndpoint
async fn resolve_addr(stack: Stack<'_>, host: &str, port: u16) -> Result<IpEndpoint, &'static str> {
    if let Some(addr) = parse_ipv4(host) {
        return Ok(IpEndpoint::new(IpAddress::Ipv4(addr), port));
    }

    let addrs = stack
        .dns_query(host, DnsQueryType::A)
        .await
        .map_err(|_| "DNS query failed")?;

    for addr in addrs {
        return Ok(IpEndpoint::new(addr, port));
    }

    Err("no A record found")
}

/// 解析 IPv4 地址字符串
fn parse_ipv4(s: &str) -> Option<Ipv4Address> {
    let mut octets = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return None;
        }
        octets[idx] = part.parse().ok()?;
        idx += 1;
    }
    if idx == 4 {
        Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
    } else {
        None
    }
}

/// 在缓冲区中查找 `\r\n\r\n` 的位置（返回结束偏移）
fn find_header_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

// ---------------------------------------------------------------------------
// Base64 编码（RFC 4648）
// ---------------------------------------------------------------------------

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 编码（标准 Base64，无填充）
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

// ---------------------------------------------------------------------------
// 简单 PRNG（基于 embassy-time ticks 的 xorshift64）
// ---------------------------------------------------------------------------

/// 使用 embassy-time ticks 作为种子，填充随机字节
fn simple_random_bytes(out: &mut [u8]) {
    // 使用 embassy-time 当前 ticks 作为种子
    let mut state: u64 = embassy_time::Instant::now().as_ticks();
    // 避免零种子
    if state == 0 {
        state = 0x1234_5678_9ABC_DEF0;
    }
    for byte in out.iter_mut() {
        // xorshift64 步进
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
}

// ---------------------------------------------------------------------------
// 服务端 WebSocket 握手辅助函数
// ---------------------------------------------------------------------------

/// 计算 Sec-WebSocket-Accept 值（SHA-1 + Base64）
///
/// `ws_key` 是客户端发送的 `Sec-WebSocket-Key` 头部值
pub(crate) fn compute_ws_accept(ws_key: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(ws_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let digest = hasher.finalize();
    base64_encode(&digest)
}

/// 构造服务端 WebSocket 升级响应字节流
pub(crate) fn build_ws_upgrade_response(ws_key: &str) -> Vec<u8> {
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
// 基于 &mut TcpSocket 的自由函数（供服务端 handler 使用）
// ---------------------------------------------------------------------------

/// 从 TcpSocket 读取一个 WebSocket 帧（内部实现）
async fn ws_recv_impl(socket: &mut TcpSocket<'_>) -> Result<WsFrameImpl, &'static str> {
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
        _ => Err("unsupported WebSocket opcode"),
    }
}

/// 接收一条完整的 WebSocket 消息（服务端使用，自动处理 Ping/Pong 和分段数据）
///
/// # 示例
/// ```ignore
/// match ws_recv(&mut socket).await? {
///     WsFrame::Text(text) => { /* 处理文本 */ }
///     WsFrame::Binary(data) => { /* 处理二进制数据 */ }
/// }
/// ```
pub async fn ws_recv(socket: &mut TcpSocket<'_>) -> Result<WsFrame, &'static str> {
    let mut accum: Vec<u8> = Vec::new();
    loop {
        match ws_recv_impl(socket).await? {
            WsFrameImpl::Close => return Err("WebSocket close frame"),
            WsFrameImpl::Ping => {
                ws_send_impl(socket, WsFrameImpl::Pong).await?;
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

/// 发送 WebSocket 帧（服务端内部实现，不带 mask）
async fn ws_send_impl(socket: &mut TcpSocket<'_>, frame: WsFrameImpl) -> Result<(), &'static str> {
    let (fin, opcode, payload) = match frame {
        WsFrameImpl::Close => (true, 0x8u8, Vec::new()),
        WsFrameImpl::Ping => (true, 0x9, Vec::new()),
        WsFrameImpl::Pong => (true, 0xA, Vec::new()),
        WsFrameImpl::Binary(data) => (true, 0x2, data),
        WsFrameImpl::Text(data) => (true, 0x1, data),
        WsFrameImpl::PartData(data) => (false, 0x0, data),
    };

    let payload_len = payload.len();
    let mut hdr = Vec::with_capacity(10);
    hdr.push(if fin { 0x80 | opcode } else { opcode });

    // 服务端发送的帧不设置 mask bit
    if payload_len < 126 {
        hdr.push(payload_len as u8);
    } else if payload_len < 65536 {
        hdr.push(126);
        hdr.extend((payload_len as u16).to_be_bytes());
    } else {
        hdr.push(127);
        hdr.extend((payload_len as u64).to_be_bytes());
    }

    ws_write_all(socket, &hdr).await?;
    if payload_len > 0 {
        ws_write_all(socket, &payload).await?;
    }
    let _ = socket.flush().await;
    Ok(())
}

/// 发送文本消息（服务端使用）
pub async fn ws_send_text(socket: &mut TcpSocket<'_>, data: &str) -> Result<(), &'static str> {
    ws_send_impl(socket, WsFrameImpl::Text(data.as_bytes().to_vec())).await
}

/// 发送二进制数据（服务端使用）
pub async fn ws_send_binary(socket: &mut TcpSocket<'_>, data: Vec<u8>) -> Result<(), &'static str> {
    ws_send_impl(socket, WsFrameImpl::Binary(data)).await
}

/// 发送 Ping 帧（服务端使用）
pub async fn ws_send_ping(socket: &mut TcpSocket<'_>) -> Result<(), &'static str> {
    ws_send_impl(socket, WsFrameImpl::Ping).await
}

/// 发送 Close 帧（服务端使用）
pub async fn ws_send_close(socket: &mut TcpSocket<'_>) -> Result<(), &'static str> {
    ws_send_impl(socket, WsFrameImpl::Close).await
}

/// 精确读取 n 字节
async fn ws_read_exact(socket: &mut TcpSocket<'_>, buf: &mut [u8]) -> Result<(), &'static str> {
    let mut offset = 0usize;
    while offset < buf.len() {
        let n = socket
            .read(&mut buf[offset..])
            .await
            .map_err(|_| "read error")?;
        if n == 0 {
            return Err("connection closed");
        }
        offset += n;
    }
    Ok(())
}

/// 写入所有字节
async fn ws_write_all(socket: &mut TcpSocket<'_>, data: &[u8]) -> Result<(), &'static str> {
    let mut written = 0usize;
    while written < data.len() {
        let n = socket
            .write(&data[written..])
            .await
            .map_err(|_| "write error")?;
        if n == 0 {
            return Err("write failed");
        }
        written += n;
    }
    Ok(())
}
