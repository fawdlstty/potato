//! 房间管理模块

use crate::webrtc::peer::Peer;
use anyhow::{anyhow, Result};
use rtc::rtp::Packet;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;

pub type TrackLocalHandle = Arc<dyn TrackLocal>;
pub type RoomTracks = HashMap<String, Vec<TrackLocalHandle>>;
pub type RtpForwarderSender = tokio::sync::broadcast::Sender<Packet>;
pub type RtpForwarders = HashMap<String, Arc<RtpForwarderSender>>;

/// 房间信息
pub struct Room {
    pub room_id: String,
    pub max_peers: u32,
    pub peers: Arc<Mutex<HashMap<String, Arc<Peer>>>>,
    pub tracks: Arc<Mutex<RoomTracks>>,
    pub rtp_forwarders: Arc<Mutex<RtpForwarders>>,
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

    /// 创建RTP转发器
    pub async fn create_rtp_forwarder(&self, peer_id: &str) -> Result<()> {
        let forwarder_key = format!("{peer_id}_rtp");
        let mut forwarders = self.rtp_forwarders.lock().await;
        if forwarders.contains_key(&forwarder_key) {
            return Ok(());
        }

        let (tx, _) = tokio::sync::broadcast::channel::<Packet>(1000);
        forwarders.insert(forwarder_key, Arc::new(tx));
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
        let peer = {
            let mut peers = self.peers.lock().await;
            peers.remove(peer_id)
        };

        if let Some(peer) = peer {
            if let Err(e) = peer.peer_connection.close().await {
                eprintln!("关闭 Peer {peer_id} 连接失败: {e}");
            }
        } else {
            return Err(anyhow!("Peer not found"));
        }

        self.tracks.lock().await.remove(peer_id);
        self.rtp_forwarders
            .lock()
            .await
            .remove(&format!("{peer_id}_rtp"));
        Ok(())
    }

    /// 转发轨道(SFU核心功能)
    pub async fn forward_track(&self, from_peer: &str, to_peer: &str) -> Result<()> {
        let (to_peer_pc, broadcast_sender) = {
            let peers = self.peers.lock().await;
            let from_peer_obj = peers
                .get(from_peer)
                .ok_or_else(|| anyhow!("Publisher peer not found: {from_peer}"))?;
            // 只转发真正处于推流状态的 peer，未推流的 peer 没有可转发的媒体
            if !from_peer_obj.publishing {
                return Err(anyhow!("Publisher peer is not publishing: {from_peer}"));
            }
            let to_peer_obj = peers
                .get(to_peer)
                .ok_or_else(|| anyhow!("Subscriber peer not found: {to_peer}"))?;

            let forwarders = self.rtp_forwarders.lock().await;
            let forwarder_key = format!("{from_peer}_rtp");
            let broadcast_sender = forwarders
                .get(&forwarder_key)
                .ok_or_else(|| anyhow!("RTP forwarder not found for peer: {from_peer}"))?
                .clone();
            (to_peer_obj.peer_connection.clone(), broadcast_sender)
        };

        let ssrc = rand::random::<u32>();
        let codec = RTCRtpCodec {
            mime_type: "video/VP8".to_string(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: vec![],
        };
        let media_track = rtc::media_stream::MediaStreamTrack::new(
            format!("{from_peer}_video"),
            uuid::Uuid::new_v4().to_string(),
            format!("{from_peer}_video_track"),
            RtpCodecKind::Video,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec,
                ..Default::default()
            }],
        );
        let track_local: TrackLocalHandle = Arc::new(TrackLocalStaticRTP::new(media_track));
        to_peer_pc.add_track(track_local.clone()).await?;

        tokio::spawn(async move {
            let mut receiver = broadcast_sender.subscribe();
            while let Ok(packet) = receiver.recv().await {
                if let Err(e) = track_local.write_rtp(packet).await {
                    eprintln!("写入RTP包失败: {e}");
                    break;
                }
            }
        });

        Ok(())
    }

    /// 添加Peer的轨道
    pub async fn add_tracks(&self, peer_id: &str, tracks: Vec<TrackLocalHandle>) -> Result<()> {
        self.tracks.lock().await.insert(peer_id.to_string(), tracks);
        Ok(())
    }

    /// 获取房间内所有Peer列表
    pub async fn get_peers_info(&self) -> Vec<PeerInfo> {
        self.peers
            .lock()
            .await
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
        self.peers.lock().await.get(peer_id).cloned()
    }

    /// 检查房间是否为空
    pub async fn is_empty(&self) -> bool {
        self.peers.lock().await.is_empty()
    }
}

/// Peer信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerInfo {
    pub id: String,
    pub is_publishing: bool,
}
