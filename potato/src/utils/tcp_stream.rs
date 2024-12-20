#![allow(async_fn_in_trait)]
use crate::{HttpMethod, HttpRequest};
use core::str;
use http::Uri;
use tokio::{io::AsyncReadExt, net::TcpStream};

pub trait TcpStreamExt {
    async fn read_until(&mut self, c: u8) -> Vec<u8>;
    async fn read_line(&mut self) -> String;
    async fn read_request(&mut self) -> anyhow::Result<HttpRequest>;
}

impl TcpStreamExt for TcpStream {
    async fn read_until(&mut self, uc: u8) -> Vec<u8> {
        let mut buf = vec![];
        while let Ok(c) = self.read_u8().await {
            match c == uc {
                true => break,
                false => buf.push(c),
            }
        }
        buf
    }

    async fn read_line(&mut self) -> String {
        let mut line = String::from_utf8(self.read_until(b'\n').await).unwrap_or("".to_string());
        if line.ends_with('\r') {
            line.pop();
        }
        line
    }

    async fn read_request(&mut self) -> anyhow::Result<HttpRequest> {
        let mut req = HttpRequest::new();
        let line = self.read_line().await;
        let items = line.split(' ').collect::<Vec<&str>>();
        if items.len() != 3 {
            return Err(anyhow::Error::msg("Unresolvable request"));
        }
        req.method = match items[0] {
            "GET" => HttpMethod::GET,
            "POST" => HttpMethod::POST,
            "PUT" => HttpMethod::PUT,
            "DELETE" => HttpMethod::DELETE,
            "OPTIONS" => HttpMethod::OPTIONS,
            "HEAD" => HttpMethod::HEAD,
            _ => return Err(anyhow::Error::msg("Unresolvable method")),
        };
        req.uri = items[1].parse::<Uri>()?;
        req.version = items[2].to_string();
        loop {
            let line = self.read_line().await;
            if let Some((key, value)) = line.split_once(':') {
                req.headers
                    .insert(key.trim().to_string(), value.trim().to_string());
            } else {
                break;
            }
        }
        Ok(req)
    }
}
