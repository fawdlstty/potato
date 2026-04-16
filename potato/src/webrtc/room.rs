//! 房间管理模块
//!
//! 负责WebRTC房间的创建、管理和Peer协调

use crate::webrtc::peer::Peer;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::TrackLocalWriter;

/// 房间信息
pub struct Room {
    pub room_id: String,
    pub max_peers: u32,
    pub peers: Arc<Mutex<HashMap<String, Arc<Peer>>>>,
    pub tracks: Arc<Mutex<HashMap<String, Vec<Arc<dyn TrackLocal + Send + Sync>>>>>,
    // 用于RTP转发: 从发布者到订阅者的转发器
    pub rtp_forwarders: Arc<
        Mutex<HashMap<String, Arc<tokio::sync::broadcast::Sender<webrtc::rtp::packet::Packet>>>>,
    >,
}

impl Room {
    /// 创建新房间
    pub fn new(room_id: &str, max_peers: u32) -> Self {
        Self {
            room_id: room_id.to_string(),
            max_peers,
            peers: Arc::new(Mutex::new(HashMap::new())),
            tracks: Arc::new(Mutex::new(HashMap::new())),
            rtp_forwarders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 创建RTP转发器（在发布者推流时调用）
    pub async fn create_rtp_forwarder(&self, peer_id: &str) -> Result<()> {
        let forwarder_key = format!("{}_rtp", peer_id);
        let mut forwarders = self.rtp_forwarders.lock().await;

        // 如果已经存在，则不重复创建
        if forwarders.contains_key(&forwarder_key) {
            println!("RTP转发器已存在: {}", peer_id);
            return Ok(());
        }

        // 创建broadcast channel（容量1000，足够缓冲RTP包）
        let (tx, _) = tokio::sync::broadcast::channel::<webrtc::rtp::packet::Packet>(1000);
        forwarders.insert(forwarder_key, Arc::new(tx));

        println!("为发布者 {} 创建RTP转发器", peer_id);
        Ok(())
    }

    /// 添加Peer到房间
    pub async fn add_peer(&self, peer_id: &str, peer: Arc<Peer>) -> Result<()> {
        let mut peers = self.peers.lock().await;
        if peers.len() >= self.max_peers as usize {
            return Err(anyhow!("Room is full"));
        }
        if peers.contains_key(peer_id) {
            return Err(anyhow!("Peer already exists"));
        }
        peers.insert(peer_id.to_string(), peer);
        Ok(())
    }

    /// 从房间移除Peer
    pub async fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let mut peers = self.peers.lock().await;
        if let Some(peer) = peers.remove(peer_id) {
            // 关闭PeerConnection
            if let Err(e) = peer.peer_connection.close().await {
                eprintln!("关闭Peer {} 的PeerConnection失败: {}", peer_id, e);
            }
        } else {
            return Err(anyhow!("Peer not found"));
        }

        // 清理该Peer的轨道
        let mut tracks = self.tracks.lock().await;
        tracks.remove(peer_id);

        // 清理该Peer的RTP转发器
        let mut forwarders = self.rtp_forwarders.lock().await;
        forwarders.remove(&format!("{}_rtp", peer_id));

        Ok(())
    }

    /// 转发轨道(SFU核心功能)
    pub async fn forward_track(&self, from_peer: &str, to_peer: &str) -> Result<()> {
        // 先获取必要的信息，然后释放锁
        let (to_peer_pc, broadcast_sender) = {
            let peers = self.peers.lock().await;

            // 获取发送方Peer（用于验证存在性）
            let _from_peer_obj = peers
                .get(from_peer)
                .ok_or_else(|| anyhow::anyhow!("Publisher peer not found: {}", from_peer))?;

            // 获取接收方Peer（验证存在性）
            let to_peer_obj = peers
                .get(to_peer)
                .ok_or_else(|| anyhow::anyhow!("Subscriber peer not found: {}", to_peer))?;

            println!("准备转发 {} 的轨道给 {}", from_peer, to_peer);

            // 获取broadcast sender用于接收RTP包
            let forwarders = self.rtp_forwarders.lock().await;
            let forwarder_key = format!("{}_rtp", from_peer);

            if !forwarders.contains_key(&forwarder_key) {
                drop(forwarders);
                drop(peers);
                return Err(anyhow::anyhow!(
                    "RTP forwarder not found for peer: {}",
                    from_peer
                ));
            }

            let broadcast_sender = forwarders
                .get(&forwarder_key)
                .ok_or_else(|| anyhow::anyhow!("RTP forwarder not found for peer: {}", from_peer))?
                .clone();
            let to_peer_pc = to_peer_obj.peer_connection.clone();

            (to_peer_pc, broadcast_sender)
        };

        // 修复问题2：为订阅者创建TrackLocal并添加到其PeerConnection
        // 需要从发布者的轨道信息创建对应的TrackLocal

        // 修复问题3：TrackLocal的stream_id需要与客户端期望的格式匹配
        // 客户端期望格式: "{publisher_id}_{media_type}"
        // 因此我们创建两个TrackLocal，一个用于video，一个用于audio
        // 但因为我们不知道具体媒体类型，先创建通用的，使用publisher_id_video

        let codec = webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
            mime_type: "video/VP8".to_string(),
            ..Default::default()
        };

        // 修复：stream_id格式改为 "{publisher_id}_video" 以匹配客户端解析逻辑
        let track_local =
            webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP::new(
                codec,
                format!("{}_video_track", from_peer), // track id
                format!("{}_video", from_peer),       // stream_id - 匹配客户端期望格式
            );

        // 使用Arc包装TrackLocal以便在多个地方使用
        let track_local_arc = Arc::new(track_local);

        // 将TrackLocal添加到订阅者的PeerConnection
        let _rtp_sender = to_peer_pc.add_track(track_local_arc.clone()).await?;
        println!("已为订阅者 {} 添加TrackLocal", to_peer);

        // 启动RTP转发任务：从broadcast channel接收RTP包并写入TrackLocal
        let track_local_for_task = track_local_arc.clone();
        let subscriber_id = to_peer.to_string();
        let from_peer_id = from_peer.to_string();
        tokio::spawn(async move {
            let mut receiver = broadcast_sender.subscribe();
            println!("启动RTP转发任务: {} -> {}", from_peer_id, subscriber_id);

            loop {
                match receiver.recv().await {
                    Ok(rtp_packet) => {
                        // 直接写入RTP包 - TrackLocal trait有write方法
                        if let Err(e) = track_local_for_task.write(&rtp_packet.payload).await {
                            eprintln!("写入RTP包失败: {}", e);
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("RTP转发滞后，丢失了 {} 个包", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        println!("RTP转发器关闭: {} -> {}", from_peer_id, subscriber_id);
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// 添加Peer的轨道
    pub async fn add_tracks(
        &self,
        peer_id: &str,
        tracks: Vec<Arc<dyn TrackLocal + Send + Sync>>,
    ) -> Result<()> {
        let mut track_map = self.tracks.lock().await;
        println!("Peer {} 添加了 {} 个轨道", peer_id, tracks.len());
        track_map.insert(peer_id.to_string(), tracks);
        Ok(())
    }

    /// 获取房间内所有Peer列表
    pub async fn get_peers_info(&self) -> Vec<PeerInfo> {
        let peers = self.peers.lock().await;
        peers
            .iter()
            .map(|(id, peer)| PeerInfo {
                id: id.clone(),
                is_publishing: peer.publishing,
            })
            .collect()
    }

    /// 广播Peer加入事件
    pub async fn broadcast_peer_joined(&self, new_peer_id: &str) -> Result<()> {
        let peers = self.peers.lock().await;
        for (peer_id, peer) in peers.iter() {
            if peer_id != new_peer_id {
                // 通知已有Peer有新成员加入
                peer.notify_peer_joined(new_peer_id).await?;
            }
        }
        Ok(())
    }

    /// 广播Peer离开事件
    pub async fn broadcast_peer_left(&self, left_peer_id: &str) -> Result<()> {
        let peers = self.peers.lock().await;
        for (peer_id, peer) in peers.iter() {
            if peer_id != left_peer_id {
                peer.notify_peer_left(left_peer_id).await?;
            }
        }
        Ok(())
    }

    /// 获取Peer
    pub async fn get_peer(&self, peer_id: &str) -> Option<Arc<Peer>> {
        let peers = self.peers.lock().await;
        peers.get(peer_id).cloned()
    }

    /// 检查房间是否为空
    pub async fn is_empty(&self) -> bool {
        let peers = self.peers.lock().await;
        peers.is_empty()
    }
}

/// Peer信息(简化版,用于客户端查询)
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerInfo {
    pub id: String,
    pub is_publishing: bool,
}
