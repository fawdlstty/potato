//! WebSocket信令实现
//!
//! 复用potato现有的WebSocket实现,提供实时双向通信

use crate::webrtc::signaling::{SignalingMessage, SignalingParams};
use crate::Websocket;
use crate::WsFrame;
use anyhow::Result;
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// WebSocket信令传输
pub struct WebSocketSignaling {
    ws: Arc<Mutex<Websocket>>,
    request_id: Arc<Mutex<u64>>,
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<SignalingMessage>>>>,
    incoming_messages: Arc<Mutex<mpsc::Receiver<SignalingMessage>>>,
}

impl WebSocketSignaling {
    /// 连接到WebSocket信令服务器
    pub async fn connect(url: &str) -> Result<Self> {
        let ws = Websocket::connect(url, vec![]).await?;
        let (incoming_tx, incoming_rx) = mpsc::channel(64);
        let pending_requests = Arc::new(Mutex::new(HashMap::new()));
        let ws = Arc::new(Mutex::new(ws));
        tokio::spawn(dispatch(
            ws.clone(),
            pending_requests.clone(),
            Arc::new(Mutex::new(incoming_tx)),
        ));
        Ok(Self {
            ws,
            request_id: Arc::new(Mutex::new(0)),
            pending_requests,
            incoming_messages: Arc::new(Mutex::new(incoming_rx)),
        })
    }

    /// 发送信令消息
    pub async fn send(&self, msg: SignalingMessage) -> Result<()> {
        let json = serde_json::to_string(&msg)?;
        let mut ws = self.ws.lock().await;
        ws.send_text(&json).await?;
        Ok(())
    }

    /// 接收信令消息
    pub async fn receive(&self) -> Result<SignalingMessage> {
        let mut incoming = self.incoming_messages.lock().await;
        incoming
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("signaling connection closed"))
    }

    /// 发送请求并等待响应
    pub async fn request(
        &self,
        method: crate::webrtc::signaling::SignalingMethod,
        params: SignalingParams,
    ) -> Result<SignalingMessage> {
        let id = {
            let mut id_guard = self.request_id.lock().await;
            *id_guard += 1;
            *id_guard
        };

        let (response_tx, response_rx) = oneshot::channel();
        self.pending_requests.lock().await.insert(id, response_tx);

        let request = SignalingMessage::request(method, params, id);
        self.send(request).await?;
        let response = response_rx.await?;
        Ok(response)
    }
}

async fn dispatch(
    ws: Arc<Mutex<Websocket>>,
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<SignalingMessage>>>>,
    incoming_messages: Arc<Mutex<mpsc::Sender<SignalingMessage>>>,
) -> Result<()> {
    loop {
        let frame = {
            let mut ws = ws.lock().await;
            ws.recv().await
        };
        match frame? {
            WsFrame::Binary(_) => continue,
            WsFrame::Text(text) => {
                let msg: SignalingMessage = serde_json::from_str(&text)?;
                if let Some(id) = msg.id {
                    if let Some(tx) = pending_requests.lock().await.remove(&id) {
                        let _ = tx.send(msg);
                        continue;
                    }
                }
                let incoming = incoming_messages.lock().await;
                let _ = incoming.try_send(msg);
            }
        }
    }
}

/// 信令传输trait(WebSocket实现)
#[async_trait::async_trait]
pub trait SignalingTransport: Send + Sync {
    async fn send(&self, msg: SignalingMessage) -> Result<()>;
    async fn receive(&self) -> Result<SignalingMessage>;
    async fn request(
        &self,
        method: crate::webrtc::signaling::SignalingMethod,
        params: SignalingParams,
    ) -> Result<SignalingMessage>;
}

#[async_trait::async_trait]
impl SignalingTransport for WebSocketSignaling {
    async fn send(&self, msg: SignalingMessage) -> Result<()> {
        self.send(msg).await
    }

    async fn receive(&self) -> Result<SignalingMessage> {
        self.receive().await
    }

    async fn request(
        &self,
        method: crate::webrtc::signaling::SignalingMethod,
        params: SignalingParams,
    ) -> Result<SignalingMessage> {
        self.request(method, params).await
    }
}
