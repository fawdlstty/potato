//! SFU服务器实现
//!
//! Selective Forwarding Unit - 选择性转发单元

use crate::webrtc::peer::Peer;
use crate::webrtc::room::Room;
use crate::webrtc::WebRTCConfig;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::api::APIBuilder;

/// SFU核心结构
pub struct WebRTCSFU {
    rooms: Arc<Mutex<HashMap<String, Arc<Room>>>>,
    api: webrtc::api::API,
    config: WebRTCConfig,
    // 修复问题1：用于发送ICE候选给客户端的通道
    // 使用RwLock允许多个reader同时访问，writer独占访问
    peer_senders: Arc<
        tokio::sync::RwLock<
            Option<
                Arc<
                    tokio::sync::Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>,
                >,
            >,
        >,
    >,
}

impl WebRTCSFU {
    /// 创建SFU实例
    pub fn new(config: WebRTCConfig) -> Self {
        let api = APIBuilder::new().build();
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            api,
            config,
            peer_senders: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// 设置peer发送器映射（用于ICE候选转发）
    /// 修复问题1：这个方法应该在WebSocket连接建立时调用
    pub fn set_peer_senders(
        &self,
        peer_senders: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>,
    ) {
        // 使用RwLock的write锁来设置
        if let Ok(mut guard) = self.peer_senders.try_write() {
            *guard = Some(peer_senders);
        }
    }

    /// 获取peer_senders的引用
    pub fn get_peer_senders(
        &self,
    ) -> Arc<
        tokio::sync::RwLock<
            Option<
                Arc<
                    tokio::sync::Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>,
                >,
            >,
        >,
    > {
        self.peer_senders.clone()
    }

    /// 创建房间
    pub async fn create_room(&self, room_id: &str, max_peers: Option<u32>) -> Result<()> {
        let mut rooms = self.rooms.lock().await;
        if rooms.contains_key(room_id) {
            return Err(anyhow::anyhow!("Room already exists"));
        }

        let max = max_peers.unwrap_or(self.config.max_peers);
        let room = Arc::new(Room::new(room_id, max));
        rooms.insert(room_id.to_string(), room);

        println!("创建房间: {}", room_id);
        Ok(())
    }

    /// 处理Offer
    pub async fn handle_offer(&self, peer_id: &str, room_id: &str, sdp: &str) -> Result<String> {
        let rooms = self.rooms.lock().await;
        let room = rooms
            .get(room_id)
            .ok_or_else(|| anyhow::anyhow!("Room not found"))?;

        // 创建PeerConnection
        let config = webrtc::peer_connection::configuration::RTCConfiguration {
            ice_servers: vec![webrtc::ice_transport::ice_server::RTCIceServer {
                urls: self.config.ice_servers.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let pc = Arc::new(self.api.new_peer_connection(config).await?);

        // 设置轨道接收回调，用于转发媒体流
        let room_clone = room.clone();
        let peer_id_clone = peer_id.to_string();
        pc.on_track(Box::new(move |track, _receiver, _streams| {
            let room = room_clone.clone();
            let peer_id = peer_id_clone.clone();

            Box::pin(async move {
                println!(
                    "Peer {} 收到轨道: {} (类型: {:?})",
                    peer_id,
                    track.id(),
                    track.kind()
                );

                // 读取RTP包并转发给所有订阅者
                let mut buf = [0u8; 1500];
                loop {
                    match track.read(&mut buf).await {
                        Ok((rtp_packet, _)) => {
                            // 将RTP包发送给所有订阅者
                            // 通过房间的rtp_forwarders broadcast channel
                            let forwarders = room.rtp_forwarders.lock().await;
                            let forwarder_key = format!("{peer_id}_rtp");

                            if let Some(sender) = forwarders.get(&forwarder_key) {
                                // 发送RTP包到broadcast channel
                                // 注意：忽略没有接收者的情况（send返回RecvError::Lagged或Closed时才报错）
                                let _ = sender.send(rtp_packet);
                            } else {
                                // 如果转发器不存在，说明还没有创建
                                // 这在正常情况下不应该发生，因为create_rtp_forwarder应该在添加peer时调用
                                eprintln!("RTP转发器不存在: {peer_id}，RTP包将被丢弃");
                            }
                        }
                        Err(_) => {
                            println!("Peer {peer_id} 的轨道 {} 结束", track.id());
                            break;
                        }
                    }
                }
            })
        }));

        // 修复问题1：设置ICE候选回调 - 通过信令发送给客户端
        let peer_senders_ref = self.peer_senders.clone();
        let _peer_id_for_ice = peer_id.to_string();
        let _room_id_for_ice = room_id.to_string(); // 保存room_id
        pc.on_ice_candidate(Box::new(move |candidate| {
            let peer_id = _peer_id_for_ice.clone();
            let room_id = _room_id_for_ice.clone();
            let peer_senders = peer_senders_ref.clone();

            Box::pin(async move {
                if let Some(candidate) = candidate {
                    if let Ok(json) = candidate.to_json() {
                        println!("Peer {peer_id} ICE候选: {}", json.candidate);

                        // 修复问题1：通过peer_senders发送ICE候选给客户端
                        let senders_guard = peer_senders.read().await;
                        if let Some(senders) = senders_guard.as_ref() {
                            let senders_lock = senders.lock().await;
                            if let Some(tx) = senders_lock.get(&peer_id) {
                                let ice_msg = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "method": "ice_candidate",
                                    "params": {
                                        "room_id": room_id,
                                        "peer_id": peer_id,
                                        "candidate": json.candidate,
                                        "sdp_mid": json.sdp_mid.unwrap_or_default(),
                                        "sdp_mline_index": json.sdp_mline_index.unwrap_or(0),
                                    },
                                    "id": null,
                                });

                                if let Err(e) = tx.send(ice_msg.to_string()) {
                                    eprintln!("发送ICE候选失败: {e}");
                                }
                            }
                        }
                    }
                }
            })
        }));

        // 设置远程描述
        let offer =
            webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(
                sdp.to_string(),
            )?;
        pc.set_remote_description(offer).await?;

        // 创建Answer
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;

        // 创建Peer对象并添加到房间
        let mut peer = Peer::new(peer_id, room_id, pc.clone());
        peer.set_publishing(true); // 设置为推流状态
        let peer = Arc::new(peer);
        room.add_peer(peer_id, peer).await?;

        // 修复问题1：在发布者添加后，立即创建RTP转发器
        // 这样订阅者才能接收到RTP流
        room.create_rtp_forwarder(peer_id).await?;

        Ok(answer.sdp)
    }

    /// 获取房间
    pub async fn get_room(&self, room_id: &str) -> Option<Arc<Room>> {
        let rooms = self.rooms.lock().await;
        rooms.get(room_id).cloned()
    }

    /// 移除空房间
    pub async fn cleanup_empty_rooms(&self) -> Result<()> {
        // 修复问题5：使用更安全的房间清理逻辑
        let mut rooms = self.rooms.lock().await;

        // 收集需要删除的房间ID
        let mut empty_rooms = Vec::new();

        for (room_id, room) in rooms.iter() {
            // 使用try_lock避免死锁
            match room.peers.try_lock() {
                Ok(peers) => {
                    if peers.is_empty() {
                        empty_rooms.push(room_id.clone());
                    }
                }
                Err(_) => {
                    // 如果无法获取锁，保守保留房间
                    println!("无法获取房间 {} 的锁，跳过清理", room_id);
                }
            }
        }

        // 删除空房间
        for room_id in empty_rooms {
            rooms.remove(&room_id);
            println!("清理空房间: {room_id}");
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
            .ok_or_else(|| anyhow::anyhow!("Room not found"))?;

        // 创建PeerConnection
        let config = webrtc::peer_connection::configuration::RTCConfiguration {
            ice_servers: vec![webrtc::ice_transport::ice_server::RTCIceServer {
                urls: self.config.ice_servers.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let pc = Arc::new(self.api.new_peer_connection(config).await?);

        // 设置ICE候选回调
        let subscriber_id_clone = subscriber_id.to_string();
        let peer_senders_ref = self.peer_senders.clone();
        let _room_id_for_ice = room_id.to_string(); // 保存room_id
        pc.on_ice_candidate(Box::new(move |candidate| {
            let subscriber_id = subscriber_id_clone.clone();
            let room_id = _room_id_for_ice.clone();
            let peer_senders = peer_senders_ref.clone();

            Box::pin(async move {
                if let Some(candidate) = candidate {
                    if let Ok(json) = candidate.to_json() {
                        println!("订阅者 {subscriber_id} ICE候选: {}", json.candidate);

                        // 修复问题1：通过peer_senders发送ICE候选给客户端
                        let senders_guard = peer_senders.read().await;
                        if let Some(senders) = senders_guard.as_ref() {
                            let senders_lock = senders.lock().await;
                            if let Some(tx) = senders_lock.get(&subscriber_id) {
                                let ice_msg = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "method": "ice_candidate",
                                    "params": {
                                        "room_id": room_id,
                                        "peer_id": subscriber_id,
                                        "candidate": json.candidate,
                                        "sdp_mid": json.sdp_mid.unwrap_or_default(),
                                        "sdp_mline_index": json.sdp_mline_index.unwrap_or(0),
                                    },
                                    "id": null,
                                });

                                if let Err(e) = tx.send(ice_msg.to_string()) {
                                    eprintln!("发送ICE候选失败: {e}");
                                }
                            }
                        }
                    }
                }
            })
        }));

        // 设置远程描述（订阅者的Offer）
        let offer =
            webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(
                offer_sdp.to_string(),
            )?;
        pc.set_remote_description(offer).await?;

        // 修复问题2：转发发布者的轨道给订阅者
        // forward_track现在只设置RTP转发器，不创建TrackLocal
        // TrackLocal将在收到实际RTP流后动态创建
        room.forward_track(publisher_id, subscriber_id).await?;

        // 注意：由于forward_track不再创建TrackLocal，
        // 订阅者需要在收到SDP Answer后等待服务器推送媒体流
        // 实际的RTP转发将在server.rs的handle_offer中的on_track回调中处理

        // 创建Answer
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;

        // 创建Peer对象并添加到房间
        let mut peer = Peer::new(subscriber_id, room_id, pc.clone());
        peer.add_subscription(publisher_id.to_string());
        let peer = Arc::new(peer);
        room.add_peer(subscriber_id, peer).await?;

        Ok(answer.sdp)
    }
}
