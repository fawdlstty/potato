//! REST信令实现
//!
//! 使用potato的HTTP路由系统提供请求响应模式的信令

use crate::webrtc::signaling::{SignalingMessage, SignalingMethod, SignalingParams};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

/// REST信令客户端
#[allow(dead_code)]
pub struct RestSignaling {
    base_url: String,
    request_id: Arc<Mutex<u64>>,
}

impl RestSignaling {
    /// 创建REST信令客户端
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            request_id: Arc::new(Mutex::new(0)),
        }
    }

    /// 发送信令请求并等待响应
    pub async fn request(
        &self,
        method: SignalingMethod,
        params: SignalingParams,
    ) -> Result<SignalingMessage> {
        let id = {
            let mut id_guard = self.request_id.lock().await;
            *id_guard += 1;
            *id_guard
        };

        let _request = SignalingMessage::request(method, params, id);

        // 注意:REST信令在实际使用中需要通过HTTP客户端发送请求
        // 这里简化处理,返回错误提示使用WebSocket信令
        Err(anyhow::anyhow!(
            "REST signaling requires HTTP client implementation. Use WebSocket signaling instead."
        ))
    }
}
