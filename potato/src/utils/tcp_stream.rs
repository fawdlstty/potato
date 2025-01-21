#![allow(async_fn_in_trait)]
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;
//use tokio::io::AsyncWriteExt;

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
