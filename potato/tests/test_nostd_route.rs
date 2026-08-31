#[cfg(not(feature = "std"))]
use potato::{http_get, HttpRequest, HttpResponse, RouteHandler, ROUTE_HANDLERS};

#[cfg(not(feature = "std"))]
mod pending_route {
    use super::*;
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll, Waker};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    #[derive(Debug)]
    struct GateState {
        polled: AtomicBool,
        woken: AtomicBool,
        waker: Mutex<Option<Waker>>,
    }

    static GATE: OnceLock<Arc<GateState>> = OnceLock::new();

    struct WakeOnce {
        state: Arc<GateState>,
    }

    impl Future for WakeOnce {
        type Output = HttpResponse;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.state.polled.swap(true, Ordering::AcqRel) {
                Poll::Ready(HttpResponse::text("woken"))
            } else {
                *self.state.waker.lock().unwrap() = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    #[http_get("/nostd-pending-route")]
    async fn pending_handler() -> HttpResponse {
        WakeOnce {
            state: GATE.get().unwrap().clone(),
        }
        .await
    }

    #[test]
    fn async_route_waits_for_explicit_wake() {
        GATE.set(Arc::new(GateState {
            polled: AtomicBool::new(false),
            woken: AtomicBool::new(false),
            waker: Mutex::new(None),
        }))
        .unwrap();
        let gate = GATE.get().unwrap().clone();
        let waker_thread = std::thread::spawn(move || loop {
            if gate.polled.load(Ordering::Acquire) {
                let waker = gate.waker.lock().unwrap().take();
                if let Some(waker) = waker {
                    waker.wake();
                    gate.woken.store(true, Ordering::Release);
                    break;
                }
            }
            std::thread::yield_now();
        });

        let mut req = HttpRequest::new();
        let response = __potato_lite_wrap_pending_handler(&mut req)
            .expect("woken route must return a response");
        waker_thread.join().unwrap();

        assert!(GATE.get().unwrap().woken.load(Ordering::Acquire));
        assert_eq!(response.http_code, 200);
        assert_eq!(response.body, b"woken");
    }
}

#[cfg(all(not(feature = "std"), feature = "websocket"))]
mod websocket_handshake {
    use potato::{
        block_on, client_handshake_with_rng, compute_ws_accept, ws_recv, Websocket, WebsocketIo,
    };
    use std::cell::Cell;

    struct TestSocket {
        input: Vec<u8>,
        read_pos: usize,
        output: Vec<u8>,
    }

    impl TestSocket {
        fn new(input: impl Into<Vec<u8>>) -> Self {
            let input = input.into();
            Self {
                input,
                read_pos: 0,
                output: Vec::new(),
            }
        }
    }

    impl WebsocketIo for TestSocket {
        type Error = core::convert::Infallible;

        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let remaining = self.input.len() - self.read_pos;
            let count = remaining.min(buf.len());
            buf[..count].copy_from_slice(&self.input[self.read_pos..self.read_pos + count]);
            self.read_pos += count;
            Ok(count)
        }

        async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
            self.output.extend_from_slice(data);
            Ok(data.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn counter_rng() -> impl FnMut(&mut [u8]) {
        let value = Cell::new(0);
        let rng = move |out: &mut [u8]| {
            let current = value.get();
            out.fill(current);
            value.set(current + 1);
        };
        rng
    }

    #[test]
    fn websocket_client_accepts_valid_sec_websocket_accept() {
        let mut rng = counter_rng();
        let key = String::from("AAAAAAAAAAAAAAAAAAAAAA==");
        let accept = compute_ws_accept(&key);
        let mut socket = TestSocket::new(format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        ));

        block_on(client_handshake_with_rng(
            &mut socket,
            "example.test",
            80,
            "/",
            &mut rng,
        ))
        .unwrap();

        let request = String::from_utf8(socket.output).unwrap();
        assert!(request.contains(&format!("Sec-WebSocket-Key: {key}\r\n")));
    }

    #[test]
    fn websocket_client_rejects_invalid_sec_websocket_accept() {
        let mut rng = |out: &mut [u8]| out.fill(0x42);
        let mut socket = TestSocket::new(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: invalid\r\n\r\n"
                .to_string(),
        );

        let result = block_on(client_handshake_with_rng(
            &mut socket,
            "example.test",
            80,
            "/",
            &mut rng,
        ));

        assert!(result.is_err());
        assert!(socket
            .output
            .ends_with(&[0x88, 0x80, 0x42, 0x42, 0x42, 0x42]));
    }

    #[test]
    fn websocket_reuses_injected_rng_for_frame_masking() {
        let mut rng = counter_rng();
        let key = String::from("AAAAAAAAAAAAAAAAAAAAAA==");
        let accept = compute_ws_accept(&key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        let mut socket = TestSocket::new(response);
        block_on(client_handshake_with_rng(
            &mut socket,
            "example.test",
            80,
            "/",
            &mut rng,
        ))
        .unwrap();

        let mut websocket = Websocket::from_socket(socket, rng);
        block_on(websocket.send_text("ab")).unwrap();
        let output = websocket.into_inner().output;

        // The handshake used zero bytes; the next RNG state masks this frame.
        assert!(output.ends_with(&[0x81, 0x82, 1, 1, 1, 1, ('a' as u8) ^ 1, ('b' as u8) ^ 1]));
    }

    #[test]
    fn websocket_client_pong_is_masked() {
        let rng = counter_rng();
        let mut websocket =
            Websocket::from_socket(TestSocket::new(vec![0x89, 0x00, 0x82, 0x00]), rng);

        block_on(websocket.recv()).unwrap();

        let output = websocket.into_inner().output;
        assert_eq!(&output[..2], &[0x8a, 0x80]);
    }

    #[test]
    fn websocket_server_pong_is_unmasked() {
        let mut socket = TestSocket::new(vec![
            0x89, 0x80, 0x11, 0x22, 0x33, 0x44, 0x82, 0x80, 0x55, 0x66, 0x77, 0x88,
        ]);

        block_on(ws_recv(&mut socket)).unwrap();

        assert_eq!(&socket.output[..2], &[0x8a, 0x00]);
    }
}

#[cfg(not(feature = "std"))]
#[http_get("/nostd-route-check")]
async fn handler(_req: &mut HttpRequest) -> HttpResponse {
    HttpResponse::text("ok")
}

#[cfg(not(feature = "std"))]
#[test]
fn nostd_macro_wrapper_runs_async_route() {
    let mut req = HttpRequest::new();
    let response = __potato_lite_wrap_handler(&mut req).unwrap();
    assert_eq!(response.http_code, 200);
    assert_eq!(response.body, b"ok");
}

#[cfg(not(feature = "std"))]
#[test]
fn nostd_macro_registers_route() {
    let route: Option<&RouteHandler> = ROUTE_HANDLERS
        .iter()
        .find(|route| route.path == "/nostd-route-check");
    assert_eq!(route.map(|route| route.method), Some("GET"));
}
