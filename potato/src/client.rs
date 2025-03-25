use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use anyhow::anyhow;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

pub struct SessionImpl {
    unique_host: (String, bool, u16),
    stream: Box<dyn TcpStreamExt>,
}

impl SessionImpl {
    pub async fn new(host: String, use_ssl: bool, port: u16) -> anyhow::Result<Self> {
        let stream: Box<dyn TcpStreamExt> = match use_ssl {
            true => {
                let mut root_cert = RootCertStore::empty();
                root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = ClientConfig::builder()
                    .with_root_certificates(root_cert)
                    .with_no_client_auth();
                let connector = TlsConnector::from(Arc::new(config));
                let dnsname = ServerName::try_from(host.clone())?;
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                let stream = connector.connect(dnsname, stream).await?;
                Box::new(stream)
            }
            false => {
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                Box::new(stream)
            }
        };
        Ok(SessionImpl {
            unique_host: (host, use_ssl, port),
            stream,
        })
    }
}

pub struct Session {
    sess_impl: Option<SessionImpl>,
}

impl Session {
    pub fn new() -> Self {
        Self { sess_impl: None }
    }

    async fn start_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<HttpRequest> {
        let (req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        let host = req.get_header_host().to_string();
        let mut is_same_host = false;
        if let Some(sess_impl) = &mut self.sess_impl {
            let (host1, use_ssl1, port1) = &sess_impl.unique_host;
            if (host1, use_ssl1, port1) == (&host, &use_ssl, &port) {
                is_same_host = true;
            }
        }
        if !is_same_host {
            self.sess_impl = Some(SessionImpl::new(host, use_ssl, port).await?);
        }
        Ok(req)
    }

    async fn end_request(&mut self, req: HttpRequest) -> anyhow::Result<HttpResponse> {
        let sess_impl = self
            .sess_impl
            .as_mut()
            .ok_or_else(|| anyhow!("session impl is null"))?;
        sess_impl.stream.write_all(&req.as_bytes()).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let (res, _) = HttpResponse::from_stream(&mut buf, &mut sess_impl.stream).await?;
        Ok(res)
    }

    pub async fn get(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
        let req = self.start_request(HttpMethod::GET, url).await?;
        let res = self.end_request(req).await?;
        Ok(res)
    }

    pub async fn post(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
        let req = self.start_request(HttpMethod::POST, url).await?;
        let res = self.end_request(req).await?;
        Ok(res)
    }

    pub async fn put(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
        let req = self.start_request(HttpMethod::PUT, url).await?;
        let res = self.end_request(req).await?;
        Ok(res)
    }
}

pub async fn get(url: &str) -> anyhow::Result<HttpResponse> {
    let mut sess = Session::new();
    let res = sess.get(url).await?;
    Ok(res)
}

pub async fn post(url: &str) -> anyhow::Result<HttpResponse> {
    let mut sess = Session::new();
    let res = sess.post(url).await?;
    Ok(res)
}

pub async fn put(url: &str) -> anyhow::Result<HttpResponse> {
    let mut sess = Session::new();
    let res = sess.put(url).await?;
    Ok(res)
}
