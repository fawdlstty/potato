use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use http::uri::Scheme;
use http::Uri;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

pub struct Session {
    unique_host: (String, bool, u16),
    stream: Box<dyn TcpStreamExt>,
}

impl Session {
    // fn is_same_host(&self, uri: &Uri) -> bool {
    //     self.uri.scheme() == uri.scheme()
    //         && self.uri.host() == uri.host()
    //         && self.uri.port() == uri.port()
    // }

    pub async fn new(host: String, use_ssl: bool, port: u16) -> anyhow::Result<Self> {
        // let uri = url.parse::<Uri>()?;
        // let host = uri.host().unwrap_or("localhost");
        // let use_ssl = uri.scheme() == Some(&Scheme::HTTPS);
        // let port = uri.port_u16().unwrap_or(match use_ssl {
        //     true => 443,
        //     false => 80,
        // });
        let stream: Box<dyn TcpStreamExt> = match use_ssl {
            true => {
                let mut root_cert_store = RootCertStore::empty();
                root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = ClientConfig::builder()
                    .with_root_certificates(root_cert_store)
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
        Ok(Self {
            unique_host: (host, use_ssl, port),
            stream,
        })
    }

    async fn begin_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<HttpResponse> {
        let (req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        let host = req.get_header_host().to_string();
        if self.unique_host != (host, use_ssl, port) {
            *self = Self::new(host, use_ssl, port).await?;
        }
        let req = format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\n\r\n",
            uri.path(),
            self.uri.host().unwrap_or("localhost")
        );
        self.stream.write_all(req.as_bytes()).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let (res, _) = HttpResponse::from_stream(&mut buf, &mut self.stream).await?;
        Ok(res)
    }

    pub async fn get(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
        self.http_request("GET", url).await
    }
}

pub async fn get(url: &str) -> anyhow::Result<HttpResponse> {
    Session::new(url).await?.get(url).await
}
