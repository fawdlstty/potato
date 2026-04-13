//! Peer连接管理模块
//!
//! 管理单个Peer的WebRTC连接状态

use anyhow::Result;
use std::sync::Arc;
use webrtc::peer_connection::RTCPeerConnection;

/// Peer连接
pub struct Peer {
    pub peer_id: String,
    pub room_id: String,
    pub peer_connection: Arc<RTCPeerConnection>,
    pub publishing: bool,
    pub subscribing: Vec<String>, // 订阅的peer_id列表
}

impl Peer {
    /// 创建新Peer
    pub fn new(peer_id: &str, room_id: &str, pc: Arc<RTCPeerConnection>) -> Self {
        Self {
            peer_id: peer_id.to_string(),
            room_id: room_id.to_string(),
            peer_connection: pc,
            publishing: false,
            subscribing: Vec::new(),
        }
    }

    /// 通知有新Peer加入
    pub async fn notify_peer_joined(&self, new_peer_id: &str) -> Result<()> {
        // 通过信令通道通知
        // 具体实现需要在SFU层处理
        println!("Peer {} 通知: {} 加入房间", self.peer_id, new_peer_id);
        Ok(())
    }

    /// 通知有Peer离开
    pub async fn notify_peer_left(&self, left_peer_id: &str) -> Result<()> {
        println!("Peer {} 通知: {} 离开房间", self.peer_id, left_peer_id);
        Ok(())
    }

    /// 设置为推流状态
    pub fn set_publishing(&mut self, publishing: bool) {
        self.publishing = publishing;
    }

    /// 添加订阅
    pub fn add_subscription(&mut self, publisher_id: String) {
        if !self.subscribing.contains(&publisher_id) {
            self.subscribing.push(publisher_id);
        }
    }

    /// 移除订阅
    pub fn remove_subscription(&mut self, publisher_id: &str) {
        self.subscribing.retain(|id| id != publisher_id);
    }

    /// 关闭Peer连接
    pub async fn close(&self) -> Result<()> {
        self.peer_connection.close().await?;
        Ok(())
    }
}
