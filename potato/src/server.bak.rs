use crate::{HttpRequest, HttpResponse};
use std::{future::Future, marker::PhantomData, net::SocketAddr};
use tokio::{io::AsyncWriteExt, net::TcpListener};

// type NextFunc = dyn Fn(HttpRequest) -> dyn Future<Output = HttpResponse>;

// pub trait PipeResponse: Future<Output = HttpResponse> {}
// pub trait PipeNextFunc: Fn(HttpRequest) -> dyn PipeResponse {}
// pub trait PipeFunc: Fn(HttpRequest, &dyn PipeNextFunc) -> dyn PipeResponse {}

pub struct RequestContext {
    pub req: HttpRequest,
}

// pub struct HttpServer {
//     pub addr: String,
//     pub pipe_funcs: Vec<Box<dyn PipeFunc>>,
// }

pub struct HttpServer<F, F1, R> {
    pub addr: String,
    pub pipe_funcs: Vec<Box<F>>,
    _f1: PhantomData<F1>,
    _r: PhantomData<R>,
}

impl<F, F1, R> HttpServer<F, F1, R>
where
    F: Fn(RequestContext, &F1) -> R + 'static,
    F1: Fn(RequestContext) -> R + 'static,
    R: Future<Output = HttpResponse> + 'static,
{
    pub fn new(addr: impl Into<String>) -> Self {
        HttpServer::<F, F1, R> {
            addr: addr.into(),
            pipe_funcs: vec![],
            _f1: PhantomData,
            _r: PhantomData,
        }
    }

    pub fn add_pipe_func(&mut self, func: F) {
        self.pipe_funcs.push(Box::new(func));
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        let listener = TcpListener::bind(&addr).await?;

        // TODO combile pipe funcs
        let f = Box::new(|ctx: RequestContext| async { HttpResponse {} });
        // let mut f = Box::new(|req: HttpRequest| async { HttpResponse {} });
        // while let Some(func) = self.pipe_funcs.pop() {
        //     f = Box::new(move |req: HttpRequest| async { func(req, &f).await });
        // }

        loop {
            // accept connection
            let (mut socket, addr) = listener.accept().await?;

            let f1 = f.clone();
            _ = tokio::task::spawn(async move {
                loop {
                    // TODO parse http protocol
                    let req = HttpRequest {};
                    let ctx = RequestContext { req };

                    // call process pipes
                    let res = f1(ctx).await;
                    if let Err(_) = socket.write_all(&res.as_bytes()).await {
                        break;
                    }
                }
            });
        }
        Ok(())
    }
}
