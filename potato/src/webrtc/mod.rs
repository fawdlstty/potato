//! WebRTC模块
//!
//! 提供完整的WebRTC支持,包括:
//! - SFU媒体服务器
//! - 客户端推流/拉流
//! - WebSocket和REST双信令协议
//! - DataChannel支持
//! - 自动化媒体流处理

#[cfg(feature = "webrtc")]
pub mod client;
#[cfg(feature = "webrtc")]
pub mod datachannel;
#[cfg(feature = "webrtc")]
pub mod media;
#[cfg(feature = "webrtc")]
pub mod peer;
#[cfg(feature = "webrtc")]
pub mod room;
#[cfg(feature = "webrtc")]
pub mod server;
#[cfg(feature = "webrtc")]
pub mod signaling;
#[cfg(feature = "webrtc")]
pub mod signaling_handler;

#[cfg(feature = "webrtc")]
pub use client::*;
#[cfg(feature = "webrtc")]
pub use datachannel::*;
#[cfg(feature = "webrtc")]
pub use media::*;
#[cfg(feature = "webrtc")]
pub use peer::*;
#[cfg(feature = "webrtc")]
pub use room::*;
#[cfg(feature = "webrtc")]
pub use server::*;
#[cfg(feature = "webrtc")]
pub use signaling::*;
#[cfg(feature = "webrtc")]
pub use signaling_handler::*;

#[cfg(feature = "webrtc")]
use crate::PipeContext;
use std::sync::Arc;

/// WebRTC配置
#[cfg(feature = "webrtc")]
#[derive(Clone)]
pub struct WebRTCConfig {
    pub max_peers: u32,
    pub ice_servers: Vec<String>,
    pub udp_port_start: u16,
    pub udp_port_end: u16,
    pub enable_datachannel: bool,
    pub ws_path: String,
    pub rest_prefix: String,
    pub auto_reconnect: bool,
    pub log_level: String,
    // 事件回调（需要在创建SFU时传递）
    pub events: std::sync::Arc<WebRTCEvents>,
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            max_peers: 100,
            ice_servers: vec!["stun:stun.l.google.com:19302".to_string()],
            udp_port_start: 50000,
            udp_port_end: 60000,
            enable_datachannel: true,
            ws_path: "/ws".to_string(),
            rest_prefix: "/api/webrtc".to_string(),
            auto_reconnect: true,
            log_level: "info".to_string(),
            events: std::sync::Arc::new(WebRTCEvents::default()),
        }
    }
}

/// WebRTC事件回调
#[cfg(feature = "webrtc")]
pub struct WebRTCEvents {
    pub on_room_created: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub on_peer_joined: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    pub on_peer_left: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    pub on_publish_started: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    pub on_subscribe_started: Option<Arc<dyn Fn(&str, &str, &str) + Send + Sync>>,
    pub on_datachannel_message: Option<Arc<dyn Fn(&str, &str, &str, &[u8]) + Send + Sync>>,
}

impl Default for WebRTCEvents {
    fn default() -> Self {
        Self {
            on_room_created: None,
            on_peer_joined: None,
            on_peer_left: None,
            on_publish_started: None,
            on_subscribe_started: None,
            on_datachannel_message: None,
        }
    }
}

impl Clone for WebRTCEvents {
    fn clone(&self) -> Self {
        Self {
            on_room_created: self.on_room_created.clone(),
            on_peer_joined: self.on_peer_joined.clone(),
            on_peer_left: self.on_peer_left.clone(),
            on_publish_started: self.on_publish_started.clone(),
            on_subscribe_started: self.on_subscribe_started.clone(),
            on_datachannel_message: self.on_datachannel_message.clone(),
        }
    }
}

/// WebRTC Builder(链式配置)
#[cfg(feature = "webrtc")]
pub struct WebRTCBuilder<'a> {
    ctx: &'a mut PipeContext,
    config: WebRTCConfig,
    events: WebRTCEvents,
}

#[cfg(feature = "webrtc")]
impl<'a> WebRTCBuilder<'a> {
    /// 创建WebRTC Builder
    pub fn new(ctx: &'a mut PipeContext) -> Self {
        Self {
            ctx,
            config: WebRTCConfig::default(),
            events: WebRTCEvents::default(),
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

    /// 设置UDP端口范围
    pub fn udp_port_range(mut self, start: u16, end: u16) -> Self {
        self.config.udp_port_start = start;
        self.config.udp_port_end = end;
        self
    }

    /// 启用DataChannel
    pub fn enable_datachannel(mut self, enabled: bool) -> Self {
        self.config.enable_datachannel = enabled;
        self
    }

    /// 设置WebSocket路径
    pub fn ws_path(mut self, path: &str) -> Self {
        self.config.ws_path = path.to_string();
        self
    }

    /// 设置REST API前缀
    pub fn rest_prefix(mut self, prefix: &str) -> Self {
        self.config.rest_prefix = prefix.to_string();
        self
    }

    /// 设置自动重连
    pub fn auto_reconnect(mut self, enabled: bool) -> Self {
        self.config.auto_reconnect = enabled;
        self
    }

    /// 设置日志级别
    pub fn log_level(mut self, level: &str) -> Self {
        self.config.log_level = level.to_string();
        self
    }

    /// 房间创建事件
    pub fn on_room_created<F>(mut self, f: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.events.on_room_created = Some(Arc::new(f));
        self
    }

    /// Peer加入事件
    pub fn on_peer_joined<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        self.events.on_peer_joined = Some(Arc::new(f));
        self
    }

    /// Peer离开事件
    pub fn on_peer_left<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        self.events.on_peer_left = Some(Arc::new(f));
        self
    }

    /// 推流开始事件
    pub fn on_publish_started<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        self.events.on_publish_started = Some(Arc::new(f));
        self
    }

    /// 订阅开始事件
    pub fn on_subscribe_started<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str, &str) + Send + Sync + 'static,
    {
        self.events.on_subscribe_started = Some(Arc::new(f));
        self
    }

    /// DataChannel消息事件
    pub fn on_datachannel_message<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str, &str, &[u8]) + Send + Sync + 'static,
    {
        self.events.on_datachannel_message = Some(Arc::new(f));
        self
    }

    /// 完成配置并注册到PipeContext
    pub fn finish(self) {
        // 将WebRTC配置添加到PipeContext
        self.ctx.add_webrtc(self.config, self.events);
    }
}
