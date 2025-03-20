use crate::utils::tcp_stream::TcpStreamExt;
use crate::HttpResponse;
use http::uri::Scheme;
use http::Uri;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

pub struct Session {
    uri: Uri,
    stream: Box<dyn TcpStreamExt>,
}

impl Session {
    fn is_same_host(&self, uri: &Uri) -> bool {
        self.uri.scheme() == uri.scheme()
            && self.uri.host() == uri.host()
            && self.uri.port() == uri.port()
    }

    pub async fn new(url: &str) -> anyhow::Result<Self> {
        let uri = url.parse::<Uri>()?;
        let host = uri.host().unwrap_or("localhost");
        let use_ssl = uri.scheme() == Some(&Scheme::HTTPS);
        let port = uri.port_u16().unwrap_or(match use_ssl {
            true => 443,
            false => 80,
        });
        let stream: Box<dyn TcpStreamExt> = match use_ssl {
            true => {
                let mut root_cert_store = RootCertStore::empty();
                root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = ClientConfig::builder()
                    .with_root_certificates(root_cert_store)
                    .with_no_client_auth();
                let connector = TlsConnector::from(Arc::new(config));
                let dnsname = ServerName::try_from(host.to_string()).unwrap();
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                let stream = connector.connect(dnsname, stream).await?;
                Box::new(stream)
            }
            false => {
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                Box::new(stream)
            }
        };
        Ok(Self { uri, stream })
    }

    pub async fn get(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
        let uri = url.parse::<Uri>()?;
        if !self.is_same_host(&uri) {
            *self = Self::new(url).await?;
        }
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\n\r\n",
            uri.path(),
            self.uri.host().unwrap_or("localhost")
        );
        self.stream.write_all(req.as_bytes()).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        self.stream.read_buf(buf)
        //self.stream.read_to_end(&mut buf).await?;
        panic!()
    }
}

pub async fn get(url: &str) -> anyhow::Result<HttpResponse> {
    Session::new(url).await?.get(url).await
}
