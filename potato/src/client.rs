use http::Uri;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub async fn get(url: &str) -> anyhow::Result<String> {
    let uri = url.parse::<Uri>()?;
    let host = uri.host().unwrap_or("localhost");
    let port = uri.port_u16().unwrap_or(80);
    let mut stream = TcpStream::connect(format!("{host}:{port}")).await?;
    let req = format!("GET {} HTTP/1.1\r\nHost: {host}\r\n\r\n", uri.path());
    stream.write_all(req.as_bytes()).await?;
    let mut buf = vec![];
    stream.read_to_end(&mut buf).await?;
    Ok(String::from_utf8(buf)?)
}
