#![allow(async_fn_in_trait)]
use crate::{HttpMethod, HttpRequest, PostFile};
use core::str;
use tokio::{io::AsyncReadExt, net::TcpStream};

use super::string::StrExt;

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
        let url = items[1];
        match url.find('?') {
            Some(p) => {
                req.url_path = url[..p].to_string();
                req.url_query = url[p + 1..]
                    .split('&')
                    .into_iter()
                    .map(|s| s.split_once('=').unwrap_or((s, "")))
                    .map(|(a, b)| (a.url_decode(), b.url_decode()))
                    .collect();
            }
            None => req.url_path = url.to_string(),
        }
        req.version = items[2].to_string();
        loop {
            let line = self.read_line().await;
            if let Some((key, value)) = line.split_once(':') {
                req.set_header(key.trim(), value.trim());
            } else {
                break;
            }
        }
        if let Some(cnt_type) = req.get_header("Content-Type") {
            if cnt_type == "application/x-www-form-urlencoded" {
                let body_str = String::from_utf8(req.body).unwrap_or("".to_string());
                req.body = vec![];
                body_str.split('&').for_each(|s| {
                    if let Some((a, b)) = s.split_once('=') {
                        req.body_pairs.insert(a.url_decode(), b.url_decode());
                    }
                });
            } else if cnt_type.starts_with("multipart/form-data") {
                let boundary = cnt_type.split_once("boundary=").unwrap_or(("", "")).1;
                let body_str = unsafe { String::from_utf8_unchecked(req.body.clone()) };
                for mut s in body_str.split(format!("--{boundary}\r\n").as_str()) {
                    if s.starts_with("\r\n") {
                        s = &s[2..];
                    }
                    if s.ends_with("\r\n") {
                        s = &s[..s.len() - 2];
                    }
                    if let Some((key_str, content)) = s.split_once("\r\n\r\n") {
                        let keys: Vec<&str> = key_str
                            .split_inclusive(|p| [';', '\n'].contains(&p))
                            .map(|p| p.trim())
                            .filter(|p| !p.is_empty())
                            .collect();
                        for key in keys.into_iter() {
                            let mut name = None;
                            let mut filename = None;
                            if let Some((k, v)) = key.split_once('=') {
                                if k == "name" {
                                    name = Some(v.to_string());
                                } else if k == "filename" {
                                    filename = Some(v.to_string());
                                }
                            }
                            if let Some(name) = name {
                                if let Some(filename) = filename {
                                    req.body_files.insert(
                                        name,
                                        PostFile {
                                            filename,
                                            data: content.as_bytes().to_vec(),
                                        },
                                    );
                                } else {
                                    req.body_pairs.insert(name, content.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(req)
    }
}
