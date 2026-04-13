//! WebSocket信令实现
//!
//! 复用potato现有的WebSocket实现,提供实时双向通信

use crate::webrtc::signaling::{SignalingMessage, SignalingParams};
use crate::Websocket;
use crate::WsFrame;
use anyhow::Result;
use serde_json;
use std::sync::Arc;
use tokio::sync::Mutex;

/// WebSocket信令传输
pub struct WebSocketSignaling {
    ws: Arc<Mutex<Websocket>>,
    request_id: Arc<Mutex<u64>>,
}

impl WebSocketSignaling {
    /// 连接到WebSocket信令服务器
    pub async fn connect(url: &str) -> Result<Self> {
        let ws = Websocket::connect(url, vec![]).await?;
        Ok(Self {
            ws: Arc::new(Mutex::new(ws)),
            request_id: Arc::new(Mutex::new(0)),
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
        let mut ws = self.ws.lock().await;
        loop {
            match ws.recv().await? {
                WsFrame::Text(text) => {
                    let msg: SignalingMessage = serde_json::from_str(&text)?;
                    return Ok(msg);
                }
                WsFrame::Binary(_) => continue, // 忽略二进制消息
            }
        }
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

        let request = SignalingMessage::request(method, params, id);
        self.send(request).await?;

        // 等待响应
        loop {
            let response = self.receive().await?;
            if response.id == Some(id) {
                return Ok(response);
            }
            // 如果不是我们的响应,继续等待
        }
    }
}

/// 信令传输trait(抽象WebSocket和REST)
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
