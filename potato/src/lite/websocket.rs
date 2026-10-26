//! WebSocket frame and handshake helpers (RFC 6455).

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;

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

enum WebsocketRole {
    Client,
    Server,
}
// ---------------------------------------------------------------------------
// Websocket
// ---------------------------------------------------------------------------

/// 当前帧的已解析帧头（帧体未到齐期间挂在连接上，跨取消恢复）。
struct FrameHeader {
    opcode: u8,
    payload_len: usize,
    /// 帧头总字节数（2 + 扩展长度域 + 掩码键）。
    header_len: usize,
    mask_key: Option<[u8; 4]>,
}

/// WebSocket connection backed by a user-provided async socket.
pub struct Websocket<S, R> {
    socket: S,
    rng: R,
    /// 内部读缓冲（半帧安全垫）：调用方以 `timeout`/`select!` 掐掉 `recv`
    /// future 时（尽力排干 / 空闲轮询 / 桥接转发形态），已从 socket 读出的
    /// 字节留存于此，下次 `recv` 从断点继续——无缓冲直读形态下该竞态会把
    /// 部分字节永久丢出字节流，后续解析整体错位（表现为好端端的连接被误
    /// 判帧错误/EOF）。
    rbuf: Vec<u8>,
    /// 进行中帧的已解析帧头（见 [`Websocket::recv`] 两阶段解析）：帧头一旦
    /// 解析落定即挂在此处，帧体补齐前被取消也能从原断点续读。帧完整即复
    /// 位为 None。仅字节安全垫（rbuf）+ 本字段共同构成完整的取消安全：字节
    /// 不丢、帧解析进度不丢。
    frame: Option<FrameHeader>,
}

impl<S, R> Websocket<S, R> {
    pub const fn from_socket(socket: S, rng: R) -> Self {
        Self { socket, rng, rbuf: Vec::new(), frame: None }
    }

    pub fn into_inner(self) -> S {
        self.socket
    }

    pub fn socket_mut(&mut self) -> &mut S {
        &mut self.socket
    }
}

impl<S: WebsocketIo, R: FnMut(&mut [u8])> Websocket<S, R> {
    /// Receives a complete WebSocket message.
    ///
    /// 两阶段解析（取消安全）：调用方普遍以 `timeout`/`select!` 包裹本方法
    /// （尽力排干的 1ms 轮询、桥接转发的 tick 分支等），取消点可能落在帧
    /// 解析中途。因此帧读取不走「顺序解析遇缺即读」的单遍形态，而是：
    /// 帧头（2+扩展+掩码键）从 rbuf 纯函数解析并缓存于 `self.frame`；帧体
    /// 未到齐只做「向 rbuf 补字节」一种 await（字节永不消费）。被取消后
    /// 重入：帧头缓存仍在、rbuf 字节仍在，从断点继续——绝不从半帧中间重
    /// 新按帧头解析（那会把帧体字节当帧头读，字节流整体错位）。
    pub async fn recv(&mut self) -> anyhow::Result<WsFrame> {
        let mut accum: Vec<u8> = Vec::new();
        loop {
            let (opcode, payload_len, header_len, mask_key) = match self.frame.as_ref() {
                Some(done) => (
                    done.opcode,
                    done.payload_len,
                    done.header_len,
                    done.mask_key,
                ),
                None => match Self::parse_frame_header(&self.rbuf)? {
                    Some(done) => {
                        let tuple = (
                            done.opcode,
                            done.payload_len,
                            done.header_len,
                            done.mask_key,
                        );
                        self.frame = Some(done);
                        tuple
                    }
                    None => {
                        Self::fill_rbuf(&mut self.socket, &mut self.rbuf).await?;
                        continue;
                    }
                },
            };
            if self.rbuf.len() < header_len + payload_len {
                Self::fill_rbuf(&mut self.socket, &mut self.rbuf).await?;
                continue;
            }
            let mut payload: Vec<u8> =
                self.rbuf[header_len..header_len + payload_len].to_vec();
            self.rbuf.drain(..header_len + payload_len);
            self.frame = None;
            if let Some(mk) = mask_key {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mk[i % 4];
                }
            }
            match opcode {
                0x8 => return Err(anyhow::anyhow!("WebSocket close frame received")),
                0x9 => send_client_frame(&mut self.socket, WsFrameImpl::Pong, &mut self.rng).await?,
                0xA => {}
                0x2 => {
                    accum.extend(payload);
                    return Ok(WsFrame::Binary(accum));
                }
                0x1 => {
                    accum.extend(payload);
                    let text = String::from_utf8(accum)
                        .map_err(|_| anyhow::anyhow!("invalid UTF-8 in WebSocket text frame"))?;
                    return Ok(WsFrame::Text(text));
                }
                0x0 => {
                    accum.extend(payload);
                    if accum.len() > LITE_WS_MAX_MESSAGE_SIZE {
                        return Err(anyhow::anyhow!(
                            "WebSocket message too large: exceeds {LITE_WS_MAX_MESSAGE_SIZE}"
                        ));
                    }
                }
                _ => return Err(anyhow::anyhow!("unsupported WebSocket opcode: {opcode:#04x}")),
            }
        }
    }

    /// 从 rbuf 纯函数解析帧头（客户端角色：服务器帧不得带掩码位）。
    /// `Ok(None)` = 字节不足，需补读；帧头不完整绝不消费 rbuf。
    fn parse_frame_header(rbuf: &[u8]) -> anyhow::Result<Option<FrameHeader>> {
        if rbuf.len() < 2 {
            return Ok(None);
        }
        let opcode = rbuf[0] & 0b0000_1111;
        let masked = rbuf[1] & 0b1000_0000 != 0;
        if masked {
            // 客户端角色只收服务器帧（不掩码）；方向违例 = 字节流已错位。
            return Err(anyhow::anyhow!(
                "WebSocket frame masking violates RFC 6455 direction rules"
            ));
        }
        let len7 = (rbuf[1] & 0b0111_1111) as usize;
        let ext_len = match len7 {
            126 => 2usize,
            127 => 8usize,
            _ => 0usize,
        };
        if rbuf.len() < 2 + ext_len {
            return Ok(None);
        }
        let payload_len = match len7 {
            126 => u16::from_be_bytes([rbuf[2], rbuf[3]]) as usize,
            127 => {
                let declared = u64::from_be_bytes([
                    rbuf[2], rbuf[3], rbuf[4], rbuf[5], rbuf[6], rbuf[7], rbuf[8], rbuf[9],
                ]);
                usize::try_from(declared)
                    .map_err(|_| anyhow::anyhow!("WebSocket frame length exceeds address space"))?
            }
            _ => len7,
        };
        if payload_len > LITE_WS_MAX_FRAME_SIZE {
            return Err(anyhow::anyhow!(
                "WebSocket frame too large: {payload_len} bytes exceeds {LITE_WS_MAX_FRAME_SIZE}"
            ));
        }
        Ok(Some(FrameHeader {
            opcode,
            payload_len,
            header_len: 2 + ext_len,
            mask_key: None,
        }))
    }

    /// 向 rbuf 补充字节（尽力一块；调用方循环判断够不够）。
    async fn fill_rbuf(socket: &mut S, rbuf: &mut Vec<u8>) -> anyhow::Result<()> {
        let mut tmp = [0u8; 8192];
        let n = socket_read(socket, &mut tmp).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed during read"));
        }
        rbuf.extend_from_slice(&tmp[..n]);
        Ok(())
    }

    /// Sends a WebSocket frame.
    pub async fn send(&mut self, frame: WsFrame) -> anyhow::Result<()> {
        match frame {
            WsFrame::Binary(data) => {
                send_client_frame(&mut self.socket, WsFrameImpl::Binary(data), &mut self.rng).await
            }
            WsFrame::Text(text) => {
                send_client_frame(
                    &mut self.socket,
                    WsFrameImpl::Text(text.into_bytes()),
                    &mut self.rng,
                )
                .await
            }
        }
    }

    /// Sends a text message.
    pub async fn send_text(&mut self, data: &str) -> anyhow::Result<()> {
        send_client_frame(
            &mut self.socket,
            WsFrameImpl::Text(data.as_bytes().to_vec()),
            &mut self.rng,
        )
        .await
    }

    /// Sends binary data.
    pub async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Binary(data), &mut self.rng).await
    }

    /// Sends a ping frame.
    pub async fn send_ping(&mut self) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Ping, &mut self.rng).await
    }

    /// Sends a close frame.
    pub async fn send_close(&mut self) -> anyhow::Result<()> {
        send_client_frame(&mut self.socket, WsFrameImpl::Close, &mut self.rng).await
    }
}

// ---------------------------------------------------------------------------
// Client handshake
// ---------------------------------------------------------------------------

/// Writes a client handshake using a caller-provided random generator and
/// validates the RFC 6455 `Sec-WebSocket-Accept` response.
pub async fn client_handshake_with_rng<S, R>(
    socket: &mut S,
    host: &str,
    port: u16,
    path: &str,
    rng: &mut R,
) -> anyhow::Result<()>
where
    S: WebsocketIo,
    R: FnMut(&mut [u8]),
{
    let mut key_bytes = [0u8; 16];
    rng(&mut key_bytes);
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
            let response = core::str::from_utf8(&buf[..hdr_end])
                .map_err(|_| anyhow::anyhow!("invalid UTF-8 in handshake response"))?;
            let expected_accept = compute_ws_accept(&key);
            if find_header_value(response, "Sec-WebSocket-Accept") != Some(&expected_accept) {
                send_client_frame(socket, WsFrameImpl::Close, rng).await?;
                return Err(anyhow::anyhow!("invalid Sec-WebSocket-Accept in handshake"));
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

fn find_header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case(name) {
            Some(value.trim())
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Base64 helpers
// ---------------------------------------------------------------------------

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
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

/// Hard caps for no_std targets where `ServerConfig` is unavailable:
/// a single frame may claim up to 16 MiB and one assembled message at most
/// 64 MiB before the connection is failed.
const LITE_WS_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const LITE_WS_MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

async fn recv_impl<S: WebsocketIo>(
    socket: &mut S,
    role: &WebsocketRole,
    rbuf: &mut Vec<u8>,
) -> anyhow::Result<WsFrameImpl> {
    // 可恢复两阶段（与 Websocket::recv 同语义）：帧头从 rbuf 纯解析，
    // 不足/未定一律不消费任何字节，只做「向 rbuf 补字节」一种 await；帧体
    // 整帧进 rbuf 后才一次性消费解掩码。中途被取消（select 桥接方的竞争
    // 分支先完成）重入时，从帧头重解析同一批字节从断点续齐——绝不把帧体
    // 中段当帧头读（那会让字节流整体错位；配持久 rbuf 的调用方依赖此
    // 语义，单次调用形态 rbuf 即弃、行为不变）。
    let (opcode, payload_len, header_len, mask_key) = loop {
        if rbuf.len() < 2 {
            rbuf_fill(socket, rbuf).await?;
            continue;
        }
        let opcode = rbuf[0] & 0b0000_1111;
        let masked = rbuf[1] & 0b1000_0000 != 0;
        // RFC 6455 §5.1: client frames MUST be masked, server frames MUST NOT be
        if masked == matches!(role, WebsocketRole::Client) {
            return Err(anyhow::anyhow!(
                "WebSocket frame masking violates RFC 6455 direction rules"
            ));
        }
        let len7 = (rbuf[1] & 0b0111_1111) as usize;
        let ext_len = match len7 {
            126 => 2usize,
            127 => 8usize,
            _ => 0usize,
        };
        if rbuf.len() < 2 + ext_len {
            rbuf_fill(socket, rbuf).await?;
            continue;
        }
        let payload_len = match len7 {
            126 => u16::from_be_bytes([rbuf[2], rbuf[3]]) as usize,
            127 => {
                let declared = u64::from_be_bytes([
                    rbuf[2], rbuf[3], rbuf[4], rbuf[5], rbuf[6], rbuf[7], rbuf[8], rbuf[9],
                ]);
                usize::try_from(declared)
                    .map_err(|_| anyhow::anyhow!("WebSocket frame length exceeds address space"))?
            }
            _ => len7,
        };
        if payload_len > LITE_WS_MAX_FRAME_SIZE {
            return Err(anyhow::anyhow!(
                "WebSocket frame too large: {payload_len} bytes exceeds {LITE_WS_MAX_FRAME_SIZE}"
            ));
        }
        let mask_key = if masked {
            if rbuf.len() < 2 + ext_len + 4 {
                rbuf_fill(socket, rbuf).await?;
                continue;
            }
            Some([
                rbuf[2 + ext_len],
                rbuf[3 + ext_len],
                rbuf[4 + ext_len],
                rbuf[5 + ext_len],
            ])
        } else {
            None
        };
        break (
            opcode,
            payload_len,
            2 + ext_len + if masked { 4 } else { 0 },
            mask_key,
        );
    };
    while rbuf.len() < header_len + payload_len {
        rbuf_fill(socket, rbuf).await?;
    }
    let mut payload: Vec<u8> = rbuf[header_len..header_len + payload_len].to_vec();
    rbuf.drain(..header_len + payload_len);
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

/// 向 rbuf 补一块字节（尽力一块；调用方循环判断够不够）。连接关闭 = 错。
async fn rbuf_fill<S: WebsocketIo>(socket: &mut S, rbuf: &mut Vec<u8>) -> anyhow::Result<()> {
    let mut tmp = [0u8; 8192];
    let n = socket_read(socket, &mut tmp).await?;
    if n == 0 {
        return Err(anyhow::anyhow!("connection closed during read"));
    }
    rbuf.extend_from_slice(&tmp[..n]);
    Ok(())
}

pub async fn ws_recv<S: WebsocketIo>(socket: &mut S) -> anyhow::Result<WsFrame> {
    ws_recv_with_role(socket, WebsocketRole::Server, &mut no_rng).await
}

async fn ws_recv_with_role<S: WebsocketIo, R: FnMut(&mut [u8])>(
    socket: &mut S,
    role: WebsocketRole,
    rng: &mut R,
) -> anyhow::Result<WsFrame> {
    // 自由函数形态无跨调用状态：单次调用内缓冲（行为与直读等价）。
    let mut rbuf = Vec::new();
    ws_recv_with_role_buffered(socket, role, rng, &mut rbuf).await
}

/// 带持久读缓冲的帧接收（服务端角色，对端帧 masked，ping 自动回 pong）。
/// TCP 分段与 WS 帧边界不对齐时，单次读可能带回后续帧的字节——桥接方
/// 必须跨调用复用同一 rbuf，否则超前读取的字节在调用返回后被丢弃。
pub async fn ws_recv_buffered<S: WebsocketIo>(
    socket: &mut S,
    rbuf: &mut Vec<u8>,
) -> anyhow::Result<WsFrame> {
    ws_recv_with_role_buffered(socket, WebsocketRole::Server, &mut no_rng, rbuf).await
}

async fn ws_recv_with_role_buffered<S: WebsocketIo, R: FnMut(&mut [u8])>(
    socket: &mut S,
    role: WebsocketRole,
    rng: &mut R,
    rbuf: &mut Vec<u8>,
) -> anyhow::Result<WsFrame> {
    let mut accum: Vec<u8> = Vec::new();
    loop {
        match recv_impl(socket, &role, rbuf).await? {
            WsFrameImpl::Close => return Err(anyhow::anyhow!("WebSocket close frame received")),
            WsFrameImpl::Ping => match role {
                WebsocketRole::Client => send_client_frame(socket, WsFrameImpl::Pong, rng).await?,
                WebsocketRole::Server => send_server_frame(socket, WsFrameImpl::Pong).await?,
            },
            WsFrameImpl::Pong => {}
            WsFrameImpl::Binary(data) => {
                accum.extend(data);
                return Ok(WsFrame::Binary(accum));
            }
            WsFrameImpl::Text(data) => {
                accum.extend(data);
                let text = String::from_utf8(accum)
                    .map_err(|_| anyhow::anyhow!("invalid UTF-8 in WebSocket text frame"))?;
                return Ok(WsFrame::Text(text));
            }
            WsFrameImpl::PartData(data) => {
                accum.extend(data);
                if accum.len() > LITE_WS_MAX_MESSAGE_SIZE {
                    return Err(anyhow::anyhow!(
                        "WebSocket message too large: exceeds {LITE_WS_MAX_MESSAGE_SIZE} bytes"
                    ));
                }
            }
        }
    }
}

async fn send_client_frame<S: WebsocketIo, R: FnMut(&mut [u8])>(
    socket: &mut S,
    frame: WsFrameImpl,
    rng: &mut R,
) -> anyhow::Result<()> {
    send_frame(socket, frame, true, rng).await
}

async fn send_server_frame<S: WebsocketIo>(
    socket: &mut S,
    frame: WsFrameImpl,
) -> anyhow::Result<()> {
    send_frame(socket, frame, false, &mut no_rng).await
}

async fn send_frame<S: WebsocketIo, R: FnMut(&mut [u8])>(
    socket: &mut S,
    frame: WsFrameImpl,
    masked: bool,
    rng: &mut R,
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
        rng(&mut mask_key);
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

fn no_rng(_bytes: &mut [u8]) {}

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
