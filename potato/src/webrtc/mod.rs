//! WebRTC模块
//!
//! 提供 WebSocket 信令、SFU、PeerConnection 与媒体/DataChannel 转发支持。

#![cfg(feature = "webrtc")]

pub mod client;
pub mod datachannel;
pub mod media;
pub mod peer;
pub mod room;
pub mod server;
pub mod signaling;
pub mod signaling_handler;

pub use client::*;
pub use datachannel::*;
pub use media::*;
pub use peer::*;
pub use room::*;
pub use server::*;
pub use signaling::*;
pub use signaling_handler::*;

use crate::PipeContext;
use std::sync::Arc;

/// WebRTC运行时配置
#[derive(Clone)]
pub struct WebRTCConfig {
    pub max_peers: u32,
    pub ice_servers: Vec<String>,
    pub ws_path: String,
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            max_peers: 100,
            ice_servers: vec!["stun:stun.l.google.com:19302".to_string()],
            ws_path: "/ws".to_string(),
        }
    }
}

/// WebRTC Builder(链式配置)
pub struct WebRTCBuilder<'a> {
    ctx: &'a mut PipeContext,
    config: WebRTCConfig,
}

impl<'a> WebRTCBuilder<'a> {
    /// 创建WebRTC Builder
    pub fn new(ctx: &'a mut PipeContext) -> Self {
        Self {
            ctx,
            config: WebRTCConfig::default(),
        }
    }

    /// 设置每房间最大人数
    pub fn max_peers(mut self, n: u32) -> Self {
        self.config.max_peers = n;
        self
    }

    /// 设置ICE服务器
    pub fn ice_servers(mut self, servers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.config.ice_servers = servers.into_iter().map(|s| s.into()).collect();
        self
    }

    /// 设置WebSocket路径
    pub fn ws_path(mut self, path: &str) -> Self {
        self.config.ws_path = path.to_string();
        self
    }

    /// 完成配置并注册到PipeContext
    pub fn finish(self) {
        // 将WebRTC配置添加到PipeContext
        let sfu = Arc::new(crate::webrtc::WebRTCSFU::new(self.config));
        let handler = Arc::new(crate::webrtc::WebRtcSignalingHandler::new(sfu));
        self.ctx.add_webrtc(handler);
    }
}
