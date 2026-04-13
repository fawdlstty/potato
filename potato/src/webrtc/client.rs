//! WebRTC客户端实现
//!
//! 提供简化的WebRTC客户端API

use crate::webrtc::datachannel::DataChannelWrapper;
use crate::webrtc::media::{MediaPacket, MediaStreamConfig, MediaStreamManager, MediaType};
use crate::webrtc::room::PeerInfo;
use crate::webrtc::signaling::websocket::WebSocketSignaling;
use crate::webrtc::signaling::{JoinRoomParams, SignalingMethod, SignalingParams};
use anyhow::Result;
use std::sync::Arc;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::RTCPeerConnection;

/// 房间信息
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub room_id: String,
    pub peers: Vec<PeerInfo>,
}

/// WebRTC客户端
#[allow(dead_code)]
pub struct WebRTCClient {
    api: webrtc::api::API,
    peer_connection: Arc<RTCPeerConnection>,
    signaling: Arc<WebSocketSignaling>,
    media_streams: Arc<MediaStreamManager>,
    current_room: Option<String>,
    current_peer_id: Option<String>, // 添加当前peer_id
    pub room: RoomInfo,
    pub is_publishing: bool,
    pub subscriptions: Vec<String>,
}

impl WebRTCClient {
    /// 创建WebRTC客户端
    pub async fn new(signaling_url: &str, ice_servers: Vec<String>) -> Result<Self> {
        let api = APIBuilder::new().build();

        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: if ice_servers.is_empty() {
                    vec!["stun:stun.l.google.com:19302".to_string()]
                } else {
                    ice_servers
                },
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);
        let signaling = Arc::new(WebSocketSignaling::connect(signaling_url).await?);

        // 创建客户端实例
        let client = Self {
            api,
            peer_connection: peer_connection.clone(),
            signaling,
            media_streams: Arc::new(MediaStreamManager::new()),
            current_room: None,
            current_peer_id: None,
            room: RoomInfo {
                room_id: String::new(),
                peers: Vec::new(),
            },
            is_publishing: false,
            subscriptions: Vec::new(),
        };

        // 设置on_track回调（只设置一次，动态匹配stream_id）
        let media_streams = client.media_streams.clone();
        peer_connection.on_track(Box::new(move |track, _receiver, _streams| {
            let media_streams = media_streams.clone();

            Box::pin(async move {
                println!("收到远程轨道: {} (类型: {:?})", track.id(), track.kind());

                // 修复问题3：使用更可靠的stream_id解析方法
                // track.stream_id() 应该与订阅时注册的stream_id格式一致
                // 格式为: "{publisher_id}_{media_type}"
                let stream_id_str = track.stream_id();

                // 直接从stream_id中解析，假设格式为 "{publisher_id}_{media_type}"
                // 例如: "abc123_video" 或 "def456_audio"
                // 修复：使用更安全的解析方法
                let parts: Vec<&str> = stream_id_str.rsplitn(2, '_').collect();
                let (publisher_id, media_type) = if parts.len() == 2 {
                    // rsplitn会将字符串从右边分割，所以parts[0]是media_type, parts[1]是publisher_id
                    let media_type = parts[0];
                    let publisher_id = parts[1];
                    (publisher_id, media_type)
                } else {
                    // 如果解析失败，使用track.kind()作为备选
                    let kind_str = track.kind().to_string();
                    let media_type = if kind_str.contains("Video") || kind_str.contains("video") {
                        "video"
                    } else if kind_str.contains("Audio") || kind_str.contains("audio") {
                        "audio"
                    } else {
                        "unknown"
                    };
                    ("unknown", media_type)
                };

                let stream_id = format!("{}_{}", publisher_id, media_type);
                println!(
                    "媒体流 stream_id: {} (from track.stream_id: {})",
                    stream_id, stream_id_str
                );

                // 从 RTP 包接收并分发
                let mut buf = [0u8; 1500];
                loop {
                    match track.read(&mut buf).await {
                        Ok((packet, _)) => {
                            // packet 是 RTP Packet
                            let packet_data = MediaPacket {
                                data: packet.payload.to_vec(),
                                timestamp: packet.header.timestamp as u64,
                                sequence: packet.header.sequence_number as u32,
                                is_keyframe: false, // 需要编解码器信息
                            };

                            // 通过 manager 分发到对应的 stream
                            if let Err(e) = media_streams
                                .dispatch_rtp_packet_direct(&stream_id, packet_data)
                                .await
                            {
                                eprintln!("发送媒体包失败: {}", e);
                            }
                        }
                        Err(_) => {
                            println!("轨道 {} 结束", track.id());
                            break;
                        }
                    }
                }
            })
        }));

        Ok(client)
    }

    /// 进入房间(自动创建+加入)
    pub async fn enter_room(&mut self, room_id: &str) -> Result<()> {
        // 发送加入房间请求
        let peer_id = uuid::Uuid::new_v4().to_string();
        self.current_peer_id = Some(peer_id.clone());

        let params = SignalingParams::JoinRoom(JoinRoomParams {
            room_id: room_id.to_string(),
            peer_id: peer_id.clone(),
            peers: None, // 请求时不需要
        });

        let response = self
            .signaling
            .request(SignalingMethod::JoinRoom, params)
            .await?;

        self.current_room = Some(room_id.to_string());
        self.room.room_id = room_id.to_string();

        // 从服务器响应中获取房间内的用户列表
        if let SignalingParams::JoinRoom(join_response) = response.params {
            if let Some(peers) = join_response.peers {
                self.room.peers = peers
                    .into_iter()
                    .map(|p| crate::webrtc::room::PeerInfo {
                        id: p.id,
                        is_publishing: p.is_publishing,
                    })
                    .collect();
                println!("房间内有 {} 个用户", self.room.peers.len());
            }
        }

        println!("加入房间: {} (peer: {})", room_id, peer_id);
        Ok(())
    }

    /// 离开房间
    pub async fn exit_room(&mut self) -> Result<()> {
        if let Some(room_id) = &self.current_room {
            println!("离开房间: {}", room_id);

            // 修复：发送LeaveRoom信令通知服务器
            if let Some(peer_id) = &self.current_peer_id {
                let params = crate::webrtc::signaling::LeaveRoomParams {
                    room_id: room_id.clone(),
                    peer_id: peer_id.clone(),
                };

                // 发送离开房间消息（不等待响应）
                if let Err(e) = self
                    .signaling
                    .send(crate::webrtc::signaling::SignalingMessage::notification(
                        SignalingMethod::LeaveRoom,
                        SignalingParams::LeaveRoom(params),
                    ))
                    .await
                {
                    eprintln!("发送离开房间信令失败: {}", e);
                }
            }

            self.current_room = None;
            self.current_peer_id = None;
            self.room.peers.clear();
            self.is_publishing = false;
            self.subscriptions.clear();
        }
        Ok(())
    }

    /// 开始推流
    pub async fn publish(&mut self) -> Result<()> {
        // 1. 创建默认媒体轨道（简化实现，创建空轨道）
        // 注意：实际应用中需要从摄像头/麦克风或文件获取媒体数据

        // 2. 创建 Offer
        let offer = self.peer_connection.create_offer(None).await?;
        self.peer_connection
            .set_local_description(offer.clone())
            .await?;

        // 3. 通过信令发送 Offer
        let room_id = self
            .current_room
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Not in a room"))?;

        // 修复：使用enter_room时生成的peer_id，而不是生成新的
        let peer_id = self
            .current_peer_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Not in a room - no peer_id"))?;

        let params = crate::webrtc::signaling::PublishParams {
            room_id: room_id.clone(),
            peer_id: peer_id.clone(),
            sdp: offer.sdp.clone(),
        };

        let response = self
            .signaling
            .request(SignalingMethod::Publish, SignalingParams::Publish(params))
            .await?;

        // 4. 接收 Answer
        if let SignalingParams::Answer(answer_params) = response.params {
            let answer =
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(
                    answer_params.sdp,
                )?;
            self.peer_connection.set_remote_description(answer).await?;
        }

        // 5. 启动 ICE 候选交换
        self.setup_ice_candidate_handling().await?;

        self.is_publishing = true;
        println!("推流已启动 (peer: {})", peer_id);
        Ok(())
    }

    /// 设置 ICE 候选处理
    async fn setup_ice_candidate_handling(&self) -> Result<()> {
        let signaling = self.signaling.clone();
        let room_id = self.current_room.clone().unwrap_or_default();
        let peer_id = self.current_peer_id.clone().unwrap_or_default();

        self.peer_connection
            .on_ice_candidate(Box::new(move |candidate| {
                let signaling = signaling.clone();
                let room_id = room_id.clone();
                let peer_id = peer_id.clone();

                Box::pin(async move {
                    if let Some(candidate) = candidate {
                        // 发送 ICE 候选到服务器
                        let candidate_json = match candidate.to_json() {
                            Ok(json) => json,
                            Err(_) => return,
                        };

                        let params = crate::webrtc::signaling::IceCandidateParams {
                            room_id: room_id.clone(),
                            peer_id: peer_id.clone(),
                            candidate: candidate_json.candidate,
                            sdp_mid: candidate_json.sdp_mid.unwrap_or_default(),
                            sdp_mline_index: candidate_json.sdp_mline_index.unwrap_or(0),
                        };

                        if let Err(e) = signaling
                            .send(crate::webrtc::signaling::SignalingMessage::notification(
                                SignalingMethod::IceCandidate,
                                SignalingParams::IceCandidate(params),
                            ))
                            .await
                        {
                            eprintln!("发送 ICE 候选失败: {}", e);
                        }
                    }
                })
            }));

        Ok(())
    }

    /// 创建DataChannel
    pub async fn chat(&self, label: &str) -> Result<DataChannelWrapper> {
        let dc = self
            .peer_connection
            .create_data_channel(label, None)
            .await?;
        // create_data_channel已经返回Arc<RTCDataChannel>
        Ok(DataChannelWrapper::new(dc))
    }

    /// 订阅远程媒体流
    pub async fn subscribe(&mut self, publisher_id: &str) -> Result<SubscriptionBuilder<'_>> {
        let room_id = self
            .current_room
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Not in a room"))?;

        Ok(SubscriptionBuilder {
            client: self,
            publisher_id: publisher_id.to_string(),
            room_id,
        })
    }
}

/// 订阅构建器
/// 修复问题4：简化API设计，移除未使用的回调字段
pub struct SubscriptionBuilder<'a> {
    client: &'a mut WebRTCClient,
    publisher_id: String,
    room_id: String,
}

impl<'a> SubscriptionBuilder<'a> {
    /// 完成视频订阅
    pub async fn video(self) -> Result<MediaStreamSubscription> {
        self.subscribe_with_media_type("video").await
    }

    /// 完成音频订阅
    pub async fn audio(self) -> Result<MediaStreamSubscription> {
        self.subscribe_with_media_type("audio").await
    }

    async fn subscribe_with_media_type(self, media_type: &str) -> Result<MediaStreamSubscription> {
        // 1. 创建Offer
        let offer = self.client.peer_connection.create_offer(None).await?;
        self.client
            .peer_connection
            .set_local_description(offer.clone())
            .await?;

        // 2. 发送订阅请求（带SDP Offer）
        let subscriber_id = uuid::Uuid::new_v4().to_string();
        let params = crate::webrtc::signaling::SubscribeParams {
            room_id: self.room_id.clone(),
            publisher_id: self.publisher_id.clone(),
            subscriber_id: subscriber_id.clone(),
            sdp: Some(offer.sdp.clone()),
            sdp_type: Some("offer".to_string()),
        };

        let response = self
            .client
            .signaling
            .request(
                SignalingMethod::Subscribe,
                SignalingParams::Subscribe(params),
            )
            .await?;

        // 3. 接收 Answer
        if let SignalingParams::Answer(answer_params) = response.params {
            let answer =
                webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(
                    answer_params.sdp,
                )?;
            self.client
                .peer_connection
                .set_remote_description(answer)
                .await?;
        }

        // 4. 创建媒体流处理器
        let stream_id = format!("{}_{}", self.publisher_id, media_type);
        let config = if media_type == "video" {
            MediaStreamConfig {
                media_type: MediaType::Video,
                buffer_size: 2000,
                ..Default::default()
            }
        } else {
            MediaStreamConfig {
                media_type: MediaType::Audio,
                buffer_size: 1000,
                ..Default::default()
            }
        };

        self.client
            .media_streams
            .register_stream(&stream_id, config)
            .await?;

        // 注意：on_track回调已经在new()中设置，这里不需要重复设置

        // 5. 添加到订阅列表
        self.client.subscriptions.push(self.publisher_id.clone());

        Ok(MediaStreamSubscription {
            stream_id,
            media_streams: self.client.media_streams.clone(),
        })
    }
}

/// 媒体流订阅
pub struct MediaStreamSubscription {
    stream_id: String,
    media_streams: Arc<MediaStreamManager>,
}

impl MediaStreamSubscription {
    /// 接收媒体包
    pub async fn recv(&mut self) -> Option<MediaPacket> {
        // 注意：recv 只能调用一次，因为 receiver 被移动了
        if let Some(mut rx) = self
            .media_streams
            .get_stream_receiver(&self.stream_id)
            .await
        {
            rx.recv().await
        } else {
            None
        }
    }
}

impl Drop for WebRTCClient {
    fn drop(&mut self) {
        // 自动清理资源
        println!("WebRTCClient dropped, 清理资源");

        // PeerConnection会在drop时自动关闭
        // 但我们可以显式关闭以确保资源释放
        let pc = self.peer_connection.clone();
        tokio::spawn(async move {
            if let Err(e) = pc.close().await {
                eprintln!("关闭PeerConnection失败: {}", e);
            }
        });
    }
}
