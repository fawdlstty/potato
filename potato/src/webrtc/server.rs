//! SFU服务器实现

use crate::webrtc::peer::Peer;
use crate::webrtc::room::Room;
use crate::webrtc::WebRTCConfig;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceServer, RTCPeerConnectionIceEvent,
    RTCSessionDescription,
};

pub type PeerSenderMap = HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>;
pub type PeerSenders = Arc<Mutex<PeerSenderMap>>;
pub type SharedPeerSenders = Arc<tokio::sync::RwLock<Option<PeerSenders>>>;

#[derive(Clone)]
struct ServerPeerContext {
    room_id: String,
    peer_id: String,
    peer_senders: SharedPeerSenders,
}

struct ServerPeerHandler {
    room: Arc<Room>,
    context: ServerPeerContext,
    forward_media: bool,
}

impl ServerPeerHandler {
    async fn send_ice_candidate(&self, candidate: RTCIceCandidateInit) {
        let senders_guard = self.context.peer_senders.read().await;
        let Some(senders) = senders_guard.as_ref() else {
            return;
        };
        let senders_lock = senders.lock().await;
        if let Some(tx) = senders_lock.get(&self.context.peer_id) {
            let ice_msg = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "ice_candidate",
                "params": {
                    "room_id": self.context.room_id,
                    "peer_id": self.context.peer_id,
                    "candidate": candidate.candidate,
                    "sdp_mid": candidate.sdp_mid.unwrap_or_default(),
                    "sdp_mline_index": candidate.sdp_mline_index.unwrap_or(0),
                },
                "id": null,
            });
            if let Err(e) = tx.send(ice_msg.to_string()) {
                eprintln!("发送ICE候选失败: {e}");
            }
        }
    }
}

#[async_trait]
impl PeerConnectionEventHandler for ServerPeerHandler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if let Ok(candidate) = event.candidate.to_json() {
            self.send_ice_candidate(candidate).await;
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        if !self.forward_media {
            return;
        }

        let room = self.room.clone();
        let peer_id = self.context.peer_id.clone();
        tokio::spawn(async move {
            let forwarder_key = format!("{peer_id}_rtp");
            // 最多等待 5 秒；forwarder 被删除（peer 离开）后退出而非永久自旋
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let sender = loop {
                {
                    let forwarders = room.rtp_forwarders.lock().await;
                    if let Some(sender) = forwarders.get(&forwarder_key) {
                        break sender.clone();
                    }
                }
                if std::time::Instant::now() >= deadline {
                    eprintln!("等待 {forwarder_key} 的 RTP 转发器超时，放弃转发");
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };

            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    let _ = sender.send(packet);
                } else if matches!(event, TrackRemoteEvent::OnEnded) {
                    break;
                }
            }
        });
    }
}

/// SFU核心结构
pub struct WebRTCSFU {
    rooms: Arc<Mutex<HashMap<String, Arc<Room>>>>,
    config: WebRTCConfig,
    peer_senders: SharedPeerSenders,
    /// 防止重复启动空房间清理任务
    cleanup_started: std::sync::atomic::AtomicBool,
}

/// 房间总数上限，防止房间洪水（每个房间都持有锁与结构）
const MAX_ROOMS: usize = 1024;

impl WebRTCSFU {
    /// 创建SFU实例
    pub fn new(config: WebRTCConfig) -> Self {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            config,
            peer_senders: Arc::new(tokio::sync::RwLock::new(None)),
            cleanup_started: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 启动后台空房间清理任务（幂等，只在首次调用时启动）
    pub fn start_room_cleanup_loop(&self) {
        use std::sync::atomic::Ordering;
        if self
            .cleanup_started
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let rooms = Arc::downgrade(&self.rooms);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let Some(rooms) = rooms.upgrade() else {
                    return;
                };
                if let Err(e) = Self::cleanup_empty_rooms_in(&rooms).await {
                    eprintln!("清理空房间失败: {e}");
                }
            }
        });
    }

    pub fn config(&self) -> &WebRTCConfig {
        &self.config
    }

    /// 设置peer发送器映射
    pub fn set_peer_senders(&self, peer_senders: PeerSenders) {
        if let Ok(mut guard) = self.peer_senders.try_write() {
            *guard = Some(peer_senders);
        }
    }

    /// 获取peer_senders的引用
    pub fn get_peer_senders(&self) -> SharedPeerSenders {
        self.peer_senders.clone()
    }

    /// 创建房间
    pub async fn create_room(&self, room_id: &str, max_peers: Option<u32>) -> Result<()> {
        let mut rooms = self.rooms.lock().await;
        if rooms.contains_key(room_id) {
            return Err(anyhow::anyhow!("Room already exists"));
        }
        if rooms.len() >= MAX_ROOMS {
            return Err(anyhow::anyhow!("too many rooms: limit is {MAX_ROOMS}"));
        }

        let max = max_peers.unwrap_or(self.config.max_peers);
        rooms.insert(room_id.to_string(), Arc::new(Room::new(room_id, max)));
        Ok(())
    }

    async fn build_peer_connection(
        &self,
        room: Arc<Room>,
        peer_id: &str,
        room_id: &str,
        forward_media: bool,
    ) -> Result<Arc<dyn PeerConnection>> {
        let config = RTCConfigurationBuilder::new()
            .with_ice_servers(vec![RTCIceServer {
                urls: self.config.ice_servers.clone(),
                ..Default::default()
            }])
            .build();
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        let handler = Arc::new(ServerPeerHandler {
            room,
            context: ServerPeerContext {
                room_id: room_id.to_string(),
                peer_id: peer_id.to_string(),
                peer_senders: self.peer_senders.clone(),
            },
            forward_media,
        });

        Ok(Arc::new(
            PeerConnectionBuilder::new()
                .with_configuration(config)
                .with_media_engine(media_engine)
                .with_handler(handler)
                .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
                .build()
                .await?,
        ))
    }

    /// 处理Offer
    pub async fn handle_offer(&self, peer_id: &str, room_id: &str, sdp: &str) -> Result<String> {
        let rooms = self.rooms.lock().await;
        let room = rooms
            .get(room_id)
            .ok_or_else(|| anyhow::anyhow!("Room not found"))?
            .clone();
        drop(rooms);

        // 重复 publish 视为重协商：先清理旧的 PeerConnection 与 forwarder
        if room.get_peer(peer_id).await.is_some() {
            let _ = room.remove_peer(peer_id).await;
        }

        let pc = self
            .build_peer_connection(room.clone(), peer_id, room_id, true)
            .await?;
        let offer = RTCSessionDescription::offer(sdp.to_string())?;
        pc.set_remote_description(offer).await?;
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;

        let mut peer = Peer::new(peer_id, room_id, pc);
        peer.set_publishing(true);
        // add_peer 失败（房间满等）时回收刚建的 PeerConnection
        if let Err(e) = room.add_peer(peer_id, Arc::new(peer)).await {
            let _ = room.remove_peer(peer_id).await;
            return Err(e);
        }
        if let Err(e) = room.create_rtp_forwarder(peer_id).await {
            let _ = room.remove_peer(peer_id).await;
            return Err(e);
        }
        Ok(answer.sdp)
    }

    /// 获取房间
    pub async fn get_room(&self, room_id: &str) -> Option<Arc<Room>> {
        self.rooms.lock().await.get(room_id).cloned()
    }

    /// 移除空房间
    pub async fn cleanup_empty_rooms(&self) -> Result<()> {
        Self::cleanup_empty_rooms_in(&self.rooms).await
    }

    async fn cleanup_empty_rooms_in(rooms: &Mutex<HashMap<String, Arc<Room>>>) -> Result<()> {
        let mut rooms = rooms.lock().await;
        let mut empty_rooms = Vec::new();
        for (room_id, room) in rooms.iter() {
            match room.peers.try_lock() {
                Ok(peers) if peers.is_empty() => empty_rooms.push(room_id.clone()),
                Ok(_) => {}
                Err(_) => eprintln!("无法获取房间 {room_id} 的锁，跳过清理"),
            }
        }
        for room_id in empty_rooms {
            rooms.remove(&room_id);
        }
        Ok(())
    }

    /// 处理订阅请求
    pub async fn handle_subscribe(
        &self,
        subscriber_id: &str,
        room_id: &str,
        publisher_id: &str,
        offer_sdp: &str,
    ) -> Result<String> {
        let rooms = self.rooms.lock().await;
        let room = rooms
            .get(room_id)
            .ok_or_else(|| anyhow::anyhow!("Room not found"))?
            .clone();
        drop(rooms);

        let pc = self
            .build_peer_connection(room.clone(), subscriber_id, room_id, false)
            .await?;
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
        pc.set_remote_description(offer).await?;
        let mut peer = Peer::new(subscriber_id, room_id, pc.clone());
        peer.add_subscription(publisher_id.to_string());
        room.add_peer(subscriber_id, Arc::new(peer)).await?;

        let result = async {
            room.forward_track(publisher_id, subscriber_id).await?;
            let answer = pc.create_answer(None).await?;
            pc.set_local_description(answer.clone()).await?;
            Ok(answer.sdp)
        }
        .await;

        if result.is_err() {
            if let Err(e) = room.remove_peer(subscriber_id).await {
                eprintln!("清理订阅失败 Peer {subscriber_id}: {e}");
            }
        }
        result
    }
}
