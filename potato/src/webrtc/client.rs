//! WebRTC客户端实现

use crate::webrtc::datachannel::DataChannelWrapper;
use crate::webrtc::media::{MediaPacket, MediaStreamConfig, MediaStreamManager, MediaType};
use crate::webrtc::room::PeerInfo;
use crate::webrtc::signaling::websocket::WebSocketSignaling;
use crate::webrtc::signaling::{JoinRoomParams, SignalingMethod, SignalingParams};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceServer, RTCPeerConnectionIceEvent, RTCSessionDescription,
};

/// 房间信息
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub room_id: String,
    pub peers: Vec<PeerInfo>,
}

#[derive(Default)]
struct ClientContext {
    room_id: Option<String>,
    peer_id: Option<String>,
}

struct ClientPeerHandler {
    signaling: Arc<WebSocketSignaling>,
    media_streams: Arc<MediaStreamManager>,
    context: Arc<RwLock<ClientContext>>,
}

#[async_trait]
impl PeerConnectionEventHandler for ClientPeerHandler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        let Ok(candidate) = event.candidate.to_json() else {
            return;
        };
        let (room_id, peer_id) = {
            let context = self.context.read().await;
            (
                context.room_id.clone().unwrap_or_default(),
                context.peer_id.clone().unwrap_or_default(),
            )
        };
        if room_id.is_empty() || peer_id.is_empty() {
            return;
        }

        let params = crate::webrtc::signaling::IceCandidateParams {
            room_id,
            peer_id,
            candidate: candidate.candidate,
            sdp_mid: candidate.sdp_mid.unwrap_or_default(),
            sdp_mline_index: candidate.sdp_mline_index.unwrap_or(0),
        };
        if let Err(e) = self
            .signaling
            .send(crate::webrtc::signaling::SignalingMessage::notification(
                SignalingMethod::IceCandidate,
                SignalingParams::IceCandidate(params),
            ))
            .await
        {
            eprintln!("发送 ICE 候选失败: {e}");
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let media_streams = self.media_streams.clone();
        tokio::spawn(async move {
            let stream_id_str = track.stream_id().await;
            let kind = track.kind().await.to_string().to_lowercase();
            let media_type = if kind.contains("audio") {
                "audio"
            } else {
                "video"
            };
            let (publisher_id, resolved_media_type) = match stream_id_str.rfind('_') {
                Some(index) => (&stream_id_str[..index], &stream_id_str[index + 1..]),
                None => ("unknown", media_type),
            };
            let stream_id = format!("{publisher_id}_{resolved_media_type}");

            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    let packet = MediaPacket {
                        data: packet.payload.to_vec(),
                        timestamp: packet.header.timestamp as u64,
                        sequence: packet.header.sequence_number as u32,
                        is_keyframe: false,
                    };
                    if let Err(e) = media_streams
                        .dispatch_rtp_packet_direct(&stream_id, packet)
                        .await
                    {
                        eprintln!("发送媒体包失败: {e}");
                    }
                } else if matches!(event, TrackRemoteEvent::OnEnded) {
                    break;
                }
            }
        });
    }
}

/// WebRTC客户端
#[allow(dead_code)]
pub struct WebRTCClient {
    peer_connection: Arc<dyn PeerConnection>,
    signaling: Arc<WebSocketSignaling>,
    media_streams: Arc<MediaStreamManager>,
    context: Arc<RwLock<ClientContext>>,
    pub room: RoomInfo,
    pub is_publishing: bool,
    pub subscriptions: Vec<String>,
}

impl WebRTCClient {
    /// 创建WebRTC客户端
    pub async fn new(signaling_url: &str, ice_servers: Vec<String>) -> Result<Self> {
        let signaling = Arc::new(WebSocketSignaling::connect(signaling_url).await?);
        let media_streams = Arc::new(MediaStreamManager::new());
        let context = Arc::new(RwLock::new(ClientContext::default()));

        let config = RTCConfigurationBuilder::new()
            .with_ice_servers(vec![RTCIceServer {
                urls: if ice_servers.is_empty() {
                    vec!["stun:stun.l.google.com:19302".to_string()]
                } else {
                    ice_servers
                },
                ..Default::default()
            }])
            .build();

        let handler = Arc::new(ClientPeerHandler {
            signaling: signaling.clone(),
            media_streams: media_streams.clone(),
            context: context.clone(),
        });
        let peer_connection: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_configuration(config)
                .with_handler(handler)
                .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
                .build()
                .await?,
        );

        Ok(Self {
            peer_connection,
            signaling,
            media_streams,
            context,
            room: RoomInfo {
                room_id: String::new(),
                peers: Vec::new(),
            },
            is_publishing: false,
            subscriptions: Vec::new(),
        })
    }

    /// 进入房间(自动创建+加入)
    pub async fn enter_room(&mut self, room_id: &str) -> Result<()> {
        let peer_id = uuid::Uuid::new_v4().to_string();
        let params = SignalingParams::JoinRoom(JoinRoomParams {
            room_id: room_id.to_string(),
            peer_id: peer_id.clone(),
            peers: None,
        });

        let response = self
            .signaling
            .request(SignalingMethod::JoinRoom, params)
            .await?;

        {
            let mut context = self.context.write().await;
            context.room_id = Some(room_id.to_string());
            context.peer_id = Some(peer_id.clone());
        }
        self.room.room_id = room_id.to_string();

        if let SignalingParams::JoinRoom(join_response) = response.params {
            if let Some(peers) = join_response.peers {
                self.room.peers = peers
                    .into_iter()
                    .map(|p| PeerInfo {
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
        let (room_id, peer_id) = {
            let context = self.context.read().await;
            (context.room_id.clone(), context.peer_id.clone())
        };

        if let (Some(room_id), Some(peer_id)) = (room_id, peer_id) {
            println!("离开房间: {}", room_id);
            let params = crate::webrtc::signaling::LeaveRoomParams {
                room_id: room_id.clone(),
                peer_id: peer_id.clone(),
            };
            if let Err(e) = self
                .signaling
                .send(crate::webrtc::signaling::SignalingMessage::notification(
                    SignalingMethod::LeaveRoom,
                    SignalingParams::LeaveRoom(params),
                ))
                .await
            {
                eprintln!("发送离开房间信令失败: {e}");
            }

            let mut context = self.context.write().await;
            context.room_id = None;
            context.peer_id = None;
            self.room.peers.clear();
            self.is_publishing = false;
            self.subscriptions.clear();
        }
        Ok(())
    }

    /// 开始推流
    pub async fn publish(&mut self) -> Result<()> {
        let offer = self.peer_connection.create_offer(None).await?;
        self.peer_connection
            .set_local_description(offer.clone())
            .await?;

        let (room_id, peer_id) = {
            let context = self.context.read().await;
            (
                context
                    .room_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Not in a room"))?,
                context
                    .peer_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Not in a room - no peer_id"))?,
            )
        };

        let params = crate::webrtc::signaling::PublishParams {
            room_id: room_id.clone(),
            peer_id: peer_id.clone(),
            sdp: offer.sdp.clone(),
        };
        let response = self
            .signaling
            .request(SignalingMethod::Publish, SignalingParams::Publish(params))
            .await?;

        if let SignalingParams::Answer(answer_params) = response.params {
            let answer = RTCSessionDescription::answer(answer_params.sdp)?;
            self.peer_connection.set_remote_description(answer).await?;
        }

        self.is_publishing = true;
        println!("推流已启动 (peer: {})", peer_id);
        Ok(())
    }

    /// 创建DataChannel
    pub async fn chat(&self, label: &str) -> Result<DataChannelWrapper> {
        let dc = self
            .peer_connection
            .create_data_channel(label, None)
            .await?;
        DataChannelWrapper::new(dc).await
    }

    /// 订阅远程媒体流
    pub async fn subscribe(&mut self, publisher_id: &str) -> Result<SubscriptionBuilder<'_>> {
        let room_id = {
            let context = self.context.read().await;
            context
                .room_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Not in a room"))?
        };

        Ok(SubscriptionBuilder {
            client: self,
            publisher_id: publisher_id.to_string(),
            room_id,
        })
    }
}

/// 订阅构建器
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
        let offer = self.client.peer_connection.create_offer(None).await?;
        self.client
            .peer_connection
            .set_local_description(offer.clone())
            .await?;

        // 复用已在 join_room 注册的 peer_id 作为订阅者身份；服务端归属
        // 校验要求 subscriber_id 属于本信令连接，随机 uuid 会被拒绝
        let subscriber_id = {
            let context = self.client.context.read().await;
            context
                .peer_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Not in a room - no peer_id"))?
        };
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

        if let SignalingParams::Answer(answer_params) = response.params {
            let answer = RTCSessionDescription::answer(answer_params.sdp)?;
            self.client
                .peer_connection
                .set_remote_description(answer)
                .await?;
        }

        let stream_id = format!("{}_{media_type}", self.publisher_id);
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
        let pc = self.peer_connection.clone();
        tokio::spawn(async move {
            if let Err(e) = pc.close().await {
                eprintln!("关闭PeerConnection失败: {e}");
            }
        });
    }
}
