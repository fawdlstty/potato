#![allow(non_camel_case_types)]
use crate::utils::refstr::Headers;
use crate::utils::tcp_stream::TcpStreamExt;
use crate::{HttpMethod, HttpRequest, HttpResponse};
use anyhow::anyhow;
use rustls_pki_types::ServerName;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

macro_rules! define_session_method {
    ($fn_name:ident, $fn_name2:ident, $method:ident) => {
        pub async fn $fn_name(&mut self, url: &str) -> anyhow::Result<HttpResponse> {
            self.get_args(url, vec![]).await
        }

        pub async fn $fn_name2(
            &mut self,
            url: &str,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let mut req = self.start_request(HttpMethod::$method, url).await?;
            for arg in args.into_iter() {
                req.apply_header(arg);
            }
            self.end_request(req).await
        }
    };
}

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

    define_session_method!(get, get_args, GET);
    define_session_method!(post, post_args, POST);
    define_session_method!(put, put_args, PUT);
    define_session_method!(delete, delete_args, DELETE);
    define_session_method!(head, head_args, HEAD);
    define_session_method!(options, options_args, OPTIONS);
    define_session_method!(connect, connect_args, CONNECT);
    define_session_method!(patch, patch_args, PATCH);
    define_session_method!(trace, trace_args, TRACE);
}

macro_rules! define_client_method {
    ($fn_name:ident, $fn_name2:ident) => {
        pub async fn $fn_name(url: &str) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name(url).await
        }

        pub async fn $fn_name2(url: &str, args: Vec<Headers>) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name2(url, args).await
        }
    };
}
define_client_method!(get, get_args);
define_client_method!(post, post_args);
define_client_method!(put, put_args);
define_client_method!(delete, delete_args);
define_client_method!(head, head_args);
define_client_method!(options, options_args);
define_client_method!(connect, connect_args);
define_client_method!(patch, patch_args);
define_client_method!(trace, trace_args);

//

// #[macro_export]
// macro_rules! get {
//     ($url:expr) => {{
//         let mut sess = Session::new();
//         match sess.start_request(HttpMethod::GET, url).await {
//             Ok(req) => sess.end_request(req).await
//             Err(err) => Err(err),
//         }
//     }};
//     ($url:expr $(, $args:expr)*) => {{
//         let mut sess = Session::new();
//         match sess.start_request(HttpMethod::GET, url).await {
//             Ok(req) => {
//                 $( req.apply_header(args); )*
//                 sess.end_request(req).await
//             }
//             Err(err) => Err(err),
//         }
//     }};
// }
