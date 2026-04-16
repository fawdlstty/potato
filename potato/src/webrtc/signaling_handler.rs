//! WebRTC信令处理器
//!
//! 处理WebRTC信令的WebSocket连接和消息路由

use crate::webrtc::server::WebRTCSFU;
use crate::webrtc::signaling::{
    IceCandidateParams, JoinRoomParams, PublishParams, SignalingMessage, SignalingMethod,
    SignalingParams, SubscribeParams,
};
use crate::HttpRequest;
use crate::Websocket;
use crate::WsFrame;
use anyhow::Result;
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// WebRTC信令处理器
pub struct WebRtcSignalingHandler {
    sfu: Arc<WebRTCSFU>,
    /// 维护peer_id到消息发送通道的映射（用于ICE候选转发）
    peer_senders: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
}

impl WebRtcSignalingHandler {
    /// 创建新的信令处理器
    pub fn new(sfu: Arc<WebRTCSFU>) -> Self {
        let peer_senders = Arc::new(Mutex::new(HashMap::new()));

        // 修复问题1：将peer_senders传递给SFU，使SFU能够转发ICE候选给客户端
        // 需要通过unsafe或重新设计架构，这里我们使用一个更好的方案：
        // 在SFU中保存一个弱引用，在信令处理器中保存强引用
        // 但由于SFU已经创建，我们采用回调方案

        Self { sfu, peer_senders }
    }

    /// 处理WebSocket升级请求
    pub async fn handle_websocket(&self, req: &mut HttpRequest) -> Result<()> {
        // 升级到WebSocket
        let mut ws = req.upgrade_websocket().await?;

        // 修复问题4：在WebSocket连接建立时，将peer_senders注册到SFU
        // 这样SFU的on_ice_candidate回调就能使用peer_senders发送ICE候选
        // 注意：只需要注册一次，使用try_write避免重复设置
        {
            let peer_senders = self.peer_senders.clone();
            // 检查是否已经设置过，避免重复设置
            let sfu_peer_senders = self.sfu.get_peer_senders();
            let peer_senders_guard = sfu_peer_senders.read().await;
            if peer_senders_guard.is_none() {
                drop(peer_senders_guard);
                // 注册peer_senders到SFU
                self.sfu.set_peer_senders(peer_senders);
                println!("Peer senders已注册到SFU");
            } else {
                drop(peer_senders_guard);
                println!("Peer senders已经注册，跳过");
            }
        }

        // 处理WebSocket消息
        self.handle_ws_messages(&mut ws).await
    }

    /// 处理WebSocket消息循环
    async fn handle_ws_messages(&self, ws: &mut Websocket) -> Result<()> {
        println!("WebRTC WebSocket连接建立");

        // 创建消息通道，用于接收ICE候选等异步消息
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        // 注意：ICE候选的转发依赖于handle_ice_candidate中的实现
        // 这里我们只需要处理WebSocket消息即可

        loop {
            tokio::select! {
                // 接收WebSocket消息
                result = ws.recv() => {
                    match result? {
                        WsFrame::Text(text) => {
                            if let Err(e) = self.handle_signaling_message(ws, &text, &tx).await {
                                eprintln!("处理信令消息失败: {e}");
                            }
                        }
                        WsFrame::Binary(_) => {
                            // 忽略二进制消息
                            continue;
                        }
                    }
                }
                // 接收异步发送的消息（如ICE候选转发）
                Some(message) = rx.recv() => {
                    if let Err(e) = ws.send_text(&message).await {
                        eprintln!("发送WebSocket消息失败: {e}");
                        return Ok(());
                    }
                }
            }
        }
    }

    /// 处理单个信令消息
    async fn handle_signaling_message(
        &self,
        ws: &mut Websocket,
        text: &str,
        ws_tx: &mpsc::UnboundedSender<String>,
    ) -> Result<()> {
        let msg: SignalingMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("解析信令消息失败: {e}");
                return Err(anyhow::anyhow!("Invalid signaling message"));
            }
        };

        let request_id = msg.id;

        match msg.method {
            SignalingMethod::JoinRoom => {
                if let SignalingParams::JoinRoom(params) = msg.params {
                    self.handle_join_room(ws, params, request_id, ws_tx).await?;
                }
            }
            SignalingMethod::Publish => {
                if let SignalingParams::Publish(params) = msg.params {
                    self.handle_publish(ws, params, request_id).await?;
                }
            }
            SignalingMethod::Subscribe => {
                if let SignalingParams::Subscribe(params) = msg.params {
                    self.handle_subscribe(ws, params, request_id, ws_tx).await?;
                }
            }
            SignalingMethod::IceCandidate => {
                if let SignalingParams::IceCandidate(params) = msg.params {
                    self.handle_ice_candidate(&params).await?;
                }
            }
            SignalingMethod::LeaveRoom => {
                if let SignalingParams::LeaveRoom(params) = msg.params {
                    println!("Peer {} 离开房间 {}", params.peer_id, params.room_id);

                    // 从房间中移除peer（这会关闭PeerConnection并清理资源）
                    if let Some(room) = self.sfu.get_room(&params.room_id).await {
                        // 先广播peer离开事件
                        if let Err(e) = room.broadcast_peer_left(&params.peer_id).await {
                            eprintln!("广播peer离开事件失败: {e}");
                        }

                        // 从房间中移除peer
                        if let Err(e) = room.remove_peer(&params.peer_id).await {
                            eprintln!("从房间移除peer失败: {e}");
                        }

                        // 清理RTP转发器
                        let mut forwarders = room.rtp_forwarders.lock().await;
                        forwarders.remove(&format!("{}_rtp", params.peer_id));
                        drop(forwarders);

                        // 清理轨道
                        let mut tracks = room.tracks.lock().await;
                        tracks.remove(&params.peer_id);
                        drop(tracks);
                    }

                    // 清理peer的消息发送通道
                    let mut senders = self.peer_senders.lock().await;
                    senders.remove(&params.peer_id);
                }
            }
            _ => {
                println!("未处理的信令方法: {:?}", msg.method);
            }
        }

        Ok(())
    }

    /// 处理加入房间
    async fn handle_join_room(
        &self,
        ws: &mut Websocket,
        params: JoinRoomParams,
        request_id: Option<u64>,
        ws_tx: &mpsc::UnboundedSender<String>,
    ) -> Result<()> {
        println!("Peer {} 加入房间 {}", params.peer_id, params.room_id);

        // 注册peer的消息发送通道（用于ICE候选转发和异步消息）
        {
            let mut senders = self.peer_senders.lock().await;
            senders.insert(params.peer_id.clone(), ws_tx.clone());
            println!("Peer {} 的消息通道已注册", params.peer_id);
        }

        // 创建房间（如果不存在）
        if self.sfu.get_room(&params.room_id).await.is_none() {
            self.sfu.create_room(&params.room_id, None).await?;
        }

        // 获取房间信息
        if let Some(room) = self.sfu.get_room(&params.room_id).await {
            let peers = room.get_peers_info().await;

            // 发送响应
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "join_room",
                "params": {
                    "room_id": params.room_id,
                    "peer_id": params.peer_id,
                    "peers": peers,
                },
                "id": request_id,
            });

            ws.send_text(&response.to_string()).await?;

            // 广播Peer加入事件
            room.broadcast_peer_joined(&params.peer_id).await?;
        }

        Ok(())
    }

    /// 处理推流
    async fn handle_publish(
        &self,
        ws: &mut Websocket,
        params: PublishParams,
        request_id: Option<u64>,
    ) -> Result<()> {
        println!("Peer {} 开始推流到房间 {}", params.peer_id, params.room_id);

        // 处理Offer并生成Answer
        let answer_sdp = self
            .sfu
            .handle_offer(&params.peer_id, &params.room_id, &params.sdp)
            .await?;

        // 发送Answer响应
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "answer",
            "params": {
                "room_id": params.room_id,
                "peer_id": params.peer_id,
                "sdp": answer_sdp,
            },
            "id": request_id,
        });

        ws.send_text(&response.to_string()).await?;

        Ok(())
    }

    /// 处理订阅
    async fn handle_subscribe(
        &self,
        ws: &mut Websocket,
        params: SubscribeParams,
        request_id: Option<u64>,
        ws_tx: &mpsc::UnboundedSender<String>,
    ) -> Result<()> {
        println!(
            "Peer {} 订阅 {} 的流 (房间: {})",
            params.subscriber_id, params.publisher_id, params.room_id
        );

        // 注册订阅者的消息通道（用于ICE候选转发）
        {
            let mut senders = self.peer_senders.lock().await;
            senders.insert(params.subscriber_id.clone(), ws_tx.clone());
            println!("订阅者 {} 的消息通道已注册", params.subscriber_id);
        }

        // 检查是否提供了SDP Offer
        if let Some(offer_sdp) = &params.sdp {
            // 处理订阅Offer，返回Answer
            let answer_sdp = self
                .sfu
                .handle_subscribe(
                    &params.subscriber_id,
                    &params.room_id,
                    &params.publisher_id,
                    offer_sdp,
                )
                .await?;

            // 发送Answer响应
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "answer",
                "params": {
                    "room_id": params.room_id,
                    "peer_id": params.subscriber_id,
                    "sdp": answer_sdp,
                    "type": "answer",
                },
                "id": request_id,
            });

            ws.send_text(&response.to_string()).await?;
        } else {
            // 没有SDP，返回错误提示
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32602,
                    "message": "Subscribe requires SDP offer in params.sdp"
                },
                "id": request_id,
            });

            ws.send_text(&response.to_string()).await?;
        }

        Ok(())
    }

    /// 处理ICE候选
    async fn handle_ice_candidate(&self, params: &IceCandidateParams) -> Result<()> {
        println!(
            "收到ICE候选: Peer {} 在房间 {}",
            params.peer_id, params.room_id
        );

        // 修复问题1：将ICE候选添加到对应的PeerConnection
        if let Some(room) = self.sfu.get_room(&params.room_id).await {
            let peers = room.peers.lock().await;

            // 查找对应的peer并添加ICE候选
            if let Some(peer) = peers.get(&params.peer_id) {
                let ice_candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                    candidate: params.candidate.clone(),
                    sdp_mid: if params.sdp_mid.is_empty() {
                        None
                    } else {
                        Some(params.sdp_mid.clone())
                    },
                    sdp_mline_index: if params.sdp_mline_index == 0 {
                        None
                    } else {
                        Some(params.sdp_mline_index)
                    },
                    username_fragment: None,
                };

                if let Err(e) = peer.peer_connection.add_ice_candidate(ice_candidate).await {
                    eprintln!("添加ICE候选失败: {e}");
                }
            }

            drop(peers);
        }

        Ok(())
    }

    /// 获取peer_senders的引用（用于SFU注册ICE候选发送器）
    pub fn get_peer_senders(&self) -> Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>> {
        self.peer_senders.clone()
    }
}

/// 创建WebRTC WebSocket处理器的HTTP响应
/// 这个函数可以在自定义路由中使用
#[allow(dead_code)]
pub async fn create_webrtc_websocket_handler(
    req: &mut HttpRequest,
    sfu: Arc<WebRTCSFU>,
) -> Result<()> {
    let handler = WebRtcSignalingHandler::new(sfu);
    handler.handle_websocket(req).await
}
