use crate::HttpResponse;
use alloc::string::String;
use alloc::vec::Vec;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpAddress, IpEndpoint, Ipv4Address, Stack};

/// 发起 HTTP/1.1 GET 请求
///
/// # 参数
/// * `stack` - embassy-net 网络栈（`Copy`，按值传入）
/// * `url`   - 完整 URL，格式: `http://host[:port]/path`
///
/// # 示例
/// ```ignore
/// let res = potato_lite::client::get(stack, "http://192.168.1.1/api/data").await?;
/// // res.http_code, res.body
/// ```
pub async fn get(stack: Stack<'_>, url: &str) -> Result<HttpResponse, &'static str> {
    // 1. 解析 URL
    let (host, port, path) = parse_url(url)?;

    // 2. 解析目标地址
    let addr = resolve_addr(stack, &host, port).await?;

    // 3. 创建 TCP socket 并连接
    let mut rx_buf = [0u8; 2048];
    let mut tx_buf = [0u8; 2048];
    let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
    socket.connect(addr).await.map_err(|_| "connect failed")?;

    // 4. 发送 HTTP GET 请求
    let host_header = if port != 80 {
        alloc::format!("{}:{}", host, port)
    } else {
        host.clone()
    };
    let request = alloc::format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path,
        host_header,
    );
    let req_bytes = request.as_bytes();
    let mut written = 0;
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

    // 5. 读取响应
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 1024];
    loop {
        match socket.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }

    // 6. 解析响应
    parse_response(&buf)
}

// ---------------------------------------------------------------------------
// URL parsing
// ---------------------------------------------------------------------------

/// 解析 URL，返回 (host, port, path)
fn parse_url(url: &str) -> Result<(String, u16, String), &'static str> {
    // 去除 http:// 前缀
    let rest = if let Some(stripped) = url.strip_prefix("http://") {
        stripped
    } else {
        return Err("URL must start with http://");
    };

    // 分离 host:port 和 path
    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };

    // 分离 host 和 port
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
    // 尝试直接解析为 IPv4 地址
    if let Some(addr) = parse_ipv4(host) {
        return Ok(IpEndpoint::new(IpAddress::Ipv4(addr), port));
    }

    // DNS 查询
    let addrs = stack
        .dns_query(host, DnsQueryType::A)
        .await
        .map_err(|_| "DNS query failed")?;

    for addr in addrs {
        return Ok(IpEndpoint::new(addr, port));
    }

    Err("no A record found")
}

/// 解析 IPv4 地址字符串，如 "192.168.1.1"
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

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// 解析 HTTP 响应字节流
fn parse_response(data: &[u8]) -> Result<HttpResponse, &'static str> {
    if data.is_empty() {
        return Err("empty response");
    }

    // 查找头部结束位置
    let header_end = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header end")?;

    let header_bytes = &data[..header_end];
    let body_bytes = &data[header_end + 4..];

    // 解析状态行
    let header_str = core::str::from_utf8(header_bytes).map_err(|_| "invalid UTF-8 in headers")?;
    let mut lines = header_str.split("\r\n");

    let status_line = lines.next().ok_or("no status line")?;
    // "HTTP/1.1 200 OK"
    let mut parts = status_line.splitn(3, ' ');
    let _version = parts.next().ok_or("no version")?;
    let code_str = parts.next().ok_or("no status code")?;
    let http_code: u16 = code_str.parse().map_err(|_| "invalid status code")?;

    // 解析响应头
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((String::from(key.trim()), String::from(value.trim())));
        }
    }

    // 处理 Transfer-Encoding: chunked
    let is_chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("Transfer-Encoding") && v.contains("chunked"));

    let body = if is_chunked {
        decode_chunked(body_bytes)?
    } else {
        body_bytes.to_vec()
    };

    Ok(HttpResponse {
        http_code,
        headers,
        body,
    })
}

/// 解码 chunked transfer encoding
fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut result = Vec::new();
    let mut pos = 0;

    loop {
        // 查找 chunk size 行的结束
        let line_end = data[pos..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("incomplete chunk")?;

        let size_str = core::str::from_utf8(&data[pos..pos + line_end])
            .map_err(|_| "invalid chunk size")?
            .trim();

        let chunk_size = usize::from_str_radix(size_str, 16).map_err(|_| "invalid chunk size")?;

        pos += line_end + 2; // skip past \r\n

        if chunk_size == 0 {
            break;
        }

        if pos + chunk_size > data.len() {
            return Err("chunk data truncated");
        }

        result.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2; // skip chunk data + \r\n
    }

    Ok(result)
}
