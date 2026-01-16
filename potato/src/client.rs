#![allow(non_camel_case_types)]
use crate::utils::bytes::CompressExt;
use crate::utils::refstr::{HeaderItem, Headers};
use crate::utils::tcp_stream::HttpStream;
use crate::{HttpMethod, HttpRequest, HttpResponse, SERVER_STR};
use anyhow::anyhow;
use std::collections::HashMap;
use tokio::net::TcpStream;

macro_rules! define_session_method {
    ($fn_name:ident, $method:ident) => {
        pub async fn $fn_name(
            &mut self,
            url: &str,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let mut req = self.new_request(HttpMethod::$method, url).await?;
            for arg in args.into_iter() {
                req.apply_header(arg);
            }
            self.do_request(req).await
        }
    };

    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident, $method:ident) => {
        pub async fn $fn_name(
            &mut self,
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            let mut req = self.new_request(HttpMethod::$method, url).await?;
            req.body = body.into();
            for arg in args.into_iter() {
                req.apply_header(arg);
            }
            self.do_request(req).await
        }

        pub async fn $fn_name2(
            &mut self,
            url: &str,
            body: serde_json::Value,
            mut args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            args.push(Headers::Content_Type("application/json".into()));
            self.$fn_name(url, serde_json::to_vec(&body)?, args).await
        }

        pub async fn $fn_name3(
            &mut self,
            url: &str,
            body: String,
            mut args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            args.push(Headers::Content_Type("application/json".into()));
            self.$fn_name(url, body.into_bytes(), args).await
        }
    };
}

pub struct SessionImpl {
    pub unique_host: (String, bool, u16),
    pub stream: HttpStream,
}

impl SessionImpl {
    pub async fn new(host: String, use_ssl: bool, port: u16) -> anyhow::Result<Self> {
        let stream: HttpStream = match use_ssl {
            #[cfg(feature = "tls")]
            true => {
                use rustls_pki_types::ServerName;
                use std::sync::Arc;
                use tokio_rustls::rustls::{ClientConfig, RootCertStore};
                use tokio_rustls::TlsConnector;
                let mut root_cert = RootCertStore::empty();
                root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = ClientConfig::builder()
                    .with_root_certificates(root_cert)
                    .with_no_client_auth();
                let connector = TlsConnector::from(Arc::new(config));
                let dnsname = ServerName::try_from(host.clone())?;
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                let stream = connector.connect(dnsname, stream).await?;
                HttpStream::from_client_tls(stream)
            }
            #[cfg(not(feature = "tls"))]
            true => Err(anyhow!("unsupported tls during non-tls build"))?,
            false => {
                let stream = TcpStream::connect(format!("{host}:{port}")).await?;
                HttpStream::from_tcp(stream)
            }
        };
        Ok(SessionImpl {
            unique_host: (host, use_ssl, port),
            stream,
        })
    }
}

pub struct Session {
    pub sess_impl: Option<SessionImpl>,
}

impl Session {
    pub fn new() -> Self {
        Self { sess_impl: None }
    }

    pub async fn new_request(
        &mut self,
        method: HttpMethod,
        url: &str,
    ) -> anyhow::Result<HttpRequest> {
        let (mut req, use_ssl, port) = HttpRequest::from_url(url, method)?;
        let host = req.get_header_host().unwrap_or("127.0.0.1").to_string();
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
        req.apply_header(Headers::User_Agent(SERVER_STR.clone()));
        Ok(req)
    }

    pub async fn do_request(&mut self, mut req: HttpRequest) -> anyhow::Result<HttpResponse> {
        if let Some(sess_impl) = &mut self.sess_impl {
            req.apply_header(Headers::Host(sess_impl.unique_host.0.clone()));
        }
        let sess_impl = self.session_impl()?;
        sess_impl.stream.write_all(&req.as_bytes()).await?;
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let (res, _) = HttpResponse::from_stream(&mut buf, &mut sess_impl.stream).await?;
        Ok(res)
    }

    fn session_impl(&mut self) -> anyhow::Result<&mut SessionImpl> {
        self.sess_impl
            .as_mut()
            .ok_or_else(|| anyhow!("session impl is null"))
    }

    define_session_method!(get, GET);
    define_session_method!(post, post_json, post_json_str, POST);
    define_session_method!(put, put_json, put_json_str, PUT);
    define_session_method!(delete, DELETE);
    define_session_method!(head, HEAD);
    define_session_method!(options, OPTIONS);
    define_session_method!(connect, CONNECT);
    define_session_method!(patch, PATCH);
    define_session_method!(trace, TRACE);
}

macro_rules! define_client_method {
    ($fn_name:ident) => {
        pub async fn $fn_name(url: &str, args: Vec<Headers>) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name(url, args).await
        }
    };
    ($fn_name:ident, $fn_name2:ident, $fn_name3:ident) => {
        pub async fn $fn_name(
            url: &str,
            body: Vec<u8>,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name(url, body, args).await
        }

        pub async fn $fn_name2(
            url: &str,
            body: serde_json::Value,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name2(url, body, args).await
        }

        pub async fn $fn_name3(
            url: &str,
            body: String,
            args: Vec<Headers>,
        ) -> anyhow::Result<HttpResponse> {
            Session::new().$fn_name3(url, body, args).await
        }
    };
}
define_client_method!(get);
define_client_method!(post, post_json, post_json_str);
define_client_method!(put, put_json, put_json_str);
define_client_method!(delete);
define_client_method!(head);
define_client_method!(options);
define_client_method!(connect);
define_client_method!(patch);
define_client_method!(trace);

//

// #[macro_export]
// macro_rules! get {
//     ($url:expr) => {{
//         let mut sess = Session::new();
//         match sess.new_request(HttpMethod::GET, url).await {
//             Ok(req) => sess.do_request(req).await
//             Err(err) => Err(err),
//         }
//     }};
//     ($url:expr $(, $args:expr)*) => {{
//         let mut sess = Session::new();
//         match sess.new_request(HttpMethod::GET, url).await {
//             Ok(req) => {
//                 $( req.apply_header(args); )*
//                 sess.do_request(req).await
//             }
//             Err(err) => Err(err),
//         }
//     }};
// }

pub struct TransferSession {
    pub req_path_prefix: String,
    pub dest_url: Option<String>,
    pub conns: HashMap<(String, bool, u16), HttpStream>,
}

impl TransferSession {
    pub fn from_forward_proxy() -> Self {
        TransferSession {
            req_path_prefix: "/".to_string(),
            dest_url: None,
            conns: HashMap::new(),
        }
    }

    pub fn from_reverse_proxy(req_path_prefix: String, dest_url: String) -> Self {
        TransferSession {
            req_path_prefix,
            dest_url: Some(dest_url),
            conns: HashMap::new(),
        }
    }

    pub async fn transfer(
        &mut self,
        req: &mut HttpRequest,
        modify_content: bool,
    ) -> anyhow::Result<HttpResponse> {
        // Check if this is a WebSocket upgrade request
        if req.is_websocket() {
            return self.transfer_websocket(req).await;
        }

        // Determine destination based on proxy type
        let (dest_host, dest_use_ssl, dest_port) = if let Some(ref dest_url) = self.dest_url {
            // Reverse proxy: use the configured destination URL
            let uri = dest_url.parse::<http::Uri>()?;
            let host = uri.host().unwrap_or("localhost");
            let use_ssl = uri.scheme() == Some(&http::uri::Scheme::HTTPS);
            let port = uri.port_u16().unwrap_or(if use_ssl { 443 } else { 80 });

            // Modify the request to remove the path prefix
            if self.req_path_prefix != "/" {
                let orig_path = req.url_path.to_string();
                if orig_path.starts_with(&self.req_path_prefix) {
                    let new_path = orig_path
                        .strip_prefix(&self.req_path_prefix)
                        .unwrap_or(&orig_path);
                    req.url_path = new_path.to_string().into();
                }
            }

            (host.to_string(), use_ssl, port)
        } else {
            // Forward proxy: extract host and port from the request URL
            let host = req.get_header_host().unwrap_or("localhost").to_string();

            // Check if the request URL contains the full URL (for forward proxy CONNECT method)
            let (use_ssl, port) = if req.method == HttpMethod::CONNECT {
                // CONNECT method typically used for HTTPS tunneling
                (true, 443)
            } else {
                // Extract scheme and port from the original request if it has a full URL
                let host_header = req.get_header("Host").unwrap_or(&host);
                let port_from_header = host_header
                    .split_once(':')
                    .map(|(_, p)| p.parse::<u16>().unwrap_or(80));

                // For forward proxy, we need to determine if SSL is used based on the Host header or request characteristics
                let use_ssl = req
                    .get_header("X-Forwarded-Proto")
                    .map_or(false, |proto| proto == "https")
                    || req
                        .get_header("X-Forwarded-Proto-Https")
                        .map_or(false, |_| true)
                    || port_from_header.map_or(false, |p| p == 443);
                let port = port_from_header.unwrap_or(if use_ssl { 443 } else { 80 });

                (use_ssl, port)
            };

            (host, use_ssl, port)
        };

        // Get or create connection to destination server
        let conn_key = (dest_host.clone(), dest_use_ssl, dest_port);
        let stream = match self.conns.get_mut(&conn_key) {
            Some(stream) => stream,
            None => {
                // Create new connection
                let new_stream: HttpStream = match dest_use_ssl {
                    #[cfg(feature = "tls")]
                    true => {
                        use rustls_pki_types::ServerName;
                        use std::sync::Arc;
                        use tokio_rustls::rustls::{ClientConfig, RootCertStore};
                        use tokio_rustls::TlsConnector;

                        let mut root_cert = RootCertStore::empty();
                        root_cert.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                        let config = ClientConfig::builder()
                            .with_root_certificates(root_cert)
                            .with_no_client_auth();
                        let connector = TlsConnector::from(Arc::new(config));
                        let dnsname = ServerName::try_from(dest_host.clone())?;
                        let tcp_stream =
                            TcpStream::connect(format!("{}:{}", dest_host, dest_port)).await?;
                        let tls_stream = connector.connect(dnsname, tcp_stream).await?;
                        HttpStream::from_client_tls(tls_stream)
                    }
                    #[cfg(not(feature = "tls"))]
                    true => Err(anyhow!("unsupported tls during non-tls build"))?,
                    false => {
                        let tcp_stream =
                            TcpStream::connect(format!("{}:{}", dest_host, dest_port)).await?;
                        HttpStream::from_tcp(tcp_stream)
                    }
                };

                self.conns.insert(conn_key.clone(), new_stream);
                self.conns.get_mut(&conn_key).unwrap()
            }
        };

        // Update Host header to match destination
        req.set_header(HeaderItem::Host, dest_host.clone());

        // Forward the request to destination
        stream.write_all(&req.as_bytes()).await?;

        // Read response from destination
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let (mut res, _) = HttpResponse::from_stream(&mut buf, stream).await?;

        // Modify content if needed (for reverse proxy)
        if modify_content {
            match res.get_header("Content-Encoding") {
                Some(s) if s.to_lowercase() == "gzip" => {
                    if let Ok(data) = res.body.decompress() {
                        if let Ok(s) = str::from_utf8(&data) {
                            // Determine proxy URL and path for replacement
                            if let Some(ref dest_url) = self.dest_url {
                                let proxy_url = if dest_url.ends_with('/') {
                                    &dest_url[..dest_url.len() - 1]
                                } else {
                                    dest_url.as_str()
                                };
                                let path = if self.req_path_prefix.ends_with('/') {
                                    &self.req_path_prefix[..self.req_path_prefix.len() - 1]
                                } else {
                                    self.req_path_prefix.as_str()
                                };
                                let data = s.replace(proxy_url, path).into_bytes();
                                if let Ok(data) = data.compress() {
                                    res.body = data;
                                }
                            }
                        }
                    }
                }
                Some(_) => {} // Skip modifying content if encoding is not gzip
                None => {
                    if let Ok(s) = str::from_utf8(&res.body) {
                        // Determine proxy URL and path for replacement
                        if let Some(ref dest_url) = self.dest_url {
                            let proxy_url = if dest_url.ends_with('/') {
                                &dest_url[..dest_url.len() - 1]
                            } else {
                                dest_url.as_str()
                            };
                            let path = if self.req_path_prefix.ends_with('/') {
                                &self.req_path_prefix[..self.req_path_prefix.len() - 1]
                            } else {
                                self.req_path_prefix.as_str()
                            };
                            res.body = s.replace(proxy_url, path).into_bytes();
                        }
                    }
                }
            }
            res.headers.remove("Transfer-Encoding");
            res.headers
                .insert("Content-Length".to_string(), res.body.len().to_string());
        }

        Ok(res)
    }

    async fn transfer_websocket(&mut self, req: &mut HttpRequest) -> anyhow::Result<HttpResponse> {
        // Helper function to build WebSocket URL
        fn build_websocket_url(
            scheme_opt: Option<&str>,
            host: &str,
            port: u16,
            path: &str,
            query_str: String,
        ) -> String {
            let scheme = match scheme_opt {
                Some("https") | Some("wss") => "wss",
                _ => "ws",
            };
            let port_str = match (scheme, port) {
                ("wss", 443) | ("ws", 80) => "".to_string(),
                _ => format!(":{port}"),
            };
            format!("{scheme}://{host}{port_str}{path}{query_str}")
        }

        // Determine destination URL for WebSocket connection
        let dest_url = if let Some(ref dest_url_str) = self.dest_url {
            // Reverse proxy: use the configured destination URL
            let uri = dest_url_str.parse::<http::Uri>()?;
            let path = if self.req_path_prefix != "/" {
                let orig_path = req.url_path.to_string();
                if orig_path.starts_with(&self.req_path_prefix) {
                    orig_path
                        .strip_prefix(&self.req_path_prefix)
                        .unwrap_or(&orig_path)
                        .to_string()
                } else {
                    orig_path
                }
            } else {
                req.url_path.to_str().to_string()
            };

            // Extract host and port from the parsed URI
            let host = uri.host().unwrap_or("localhost");
            let port =
                uri.port_u16()
                    .unwrap_or(if uri.scheme() == Some(&http::uri::Scheme::HTTPS) {
                        443
                    } else {
                        80
                    });
            build_websocket_url(uri.scheme_str(), host, port, &path, req.query_string())
        } else {
            // Forward proxy case - extract destination from the original request
            let host = req.get_header_host().unwrap_or("localhost");

            // Determine if we should use wss or ws based on the Host header or other indicators
            let use_ssl = req
                .get_header("X-Forwarded-Proto")
                .map_or(false, |proto| proto == "https" || proto == "wss")
                || req
                    .get_header("X-Forwarded-Proto-Https")
                    .map_or(false, |_| true)
                || req.url_path.to_str().starts_with("https")
                || host.contains(".com") && !host.contains("127.") && !host.starts_with("192.")
                || host.contains("localhost"); // Assume localhost might be secure in dev

            // Extract port from host header, default to 443 for wss and 80 for ws if not specified
            let (host_part, port_part) = host.split_once(':').unwrap_or((host, ""));

            let port = port_part
                .parse::<u16>()
                .unwrap_or(if use_ssl { 443 } else { 80 });

            // Get the original path and query string from the request
            let path = req.url_path.to_str();
            let query_str = req.query_string();

            build_websocket_url(
                if use_ssl { Some("https") } else { None },
                host_part,
                port,
                path,
                query_str,
            )
        };

        // Prepare headers for WebSocket connection
        let mut headers = Vec::new();
        for (key, value) in req.headers.iter() {
            if key.to_str() == "Host" {
                continue;
            }
            headers.push(crate::Headers::Custom((
                key.to_str().to_string(),
                value.to_str().to_string(),
            )));
        }

        let mut target_ws = crate::Websocket::connect(&dest_url, headers)
            .await
            .map_err(|err| anyhow::anyhow!("Failed to connect to {dest_url}: {err}"))?;

        let mut client_ws = req
            .upgrade_websocket()
            .await
            .map_err(|err| anyhow::anyhow!("Failed to upgrade to websocket: {err}"))?;

        loop {
            tokio::select! {
                frame = target_ws.recv() => {
                    match frame {
                        Ok(frame) => if client_ws.send(frame).await.is_err() {
                            break;
                        },
                        Err(_) => break,
                    }
                },
                frame = client_ws.recv() => {
                    match frame {
                        Ok(frame) => if target_ws.send(frame).await.is_err() {
                            break;
                        },
                        Err(_) => break,
                    }
                },
            };
        }

        Ok(HttpResponse::empty())
    }
}
