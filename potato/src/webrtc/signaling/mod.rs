//! WebRTC信令模块
//!
//! 支持两种信令传输方式:
//! - WebSocket: 实时双向通信,适合浏览器客户端
//! - REST: 请求响应模式,适合服务端集成

pub mod rest;
pub mod websocket;

use serde::{Deserialize, Serialize};

/// 信令方法枚举
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SignalingMethod {
    // 房间管理
    CreateRoom,
    JoinRoom,
    LeaveRoom,

    // WebRTC信令
    Offer,
    Answer,
    IceCandidate,

    // 媒体控制
    Publish,
    Subscribe,
    Unpublish,
    Unsubscribe,

    // DataChannel
    CreateDataChannel,
    DataChannelMessage,
}

/// 信令消息结构(JSON-RPC 2.0格式)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SignalingMessage {
    pub jsonrpc: String,
    pub method: SignalingMethod,
    pub params: SignalingParams,
    pub id: Option<u64>,
}

/// 信令参数(使用untagged自动解析)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum SignalingParams {
    CreateRoom(CreateRoomParams),
    JoinRoom(JoinRoomParams),
    LeaveRoom(LeaveRoomParams),
    Offer(SdpParams),
    Answer(SdpParams),
    IceCandidate(IceCandidateParams),
    Publish(PublishParams),
    Subscribe(SubscribeParams),
    Unpublish(UnpublishParams),
    Unsubscribe(UnsubscribeParams),
    CreateDataChannel(CreateDataChannelParams),
    DataChannelMessage(DataChannelMessageParams),
}

/// 创建房间参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CreateRoomParams {
    pub room_id: String,
    pub max_peers: Option<u32>,
}

/// 加入房间参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinRoomParams {
    pub room_id: String,
    pub peer_id: String,
    // 服务器响应时添加的字段
    #[serde(default)]
    pub peers: Option<Vec<PeerInfoResponse>>,
}

/// Peer信息响应(用于JoinRoom响应)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PeerInfoResponse {
    pub id: String,
    pub is_publishing: bool,
}

/// 离开房间参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LeaveRoomParams {
    pub room_id: String,
    pub peer_id: String,
}

/// SDP参数(Offer/Answer)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SdpParams {
    pub room_id: String,
    pub peer_id: String,
    pub sdp: String,
}

/// ICE候选参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IceCandidateParams {
    pub room_id: String,
    pub peer_id: String,
    pub candidate: String,
    pub sdp_mid: String,
    pub sdp_mline_index: u16,
}

/// 推流参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PublishParams {
    pub room_id: String,
    pub peer_id: String,
    pub sdp: String,
}

/// 订阅参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubscribeParams {
    pub room_id: String,
    pub publisher_id: String,
    pub subscriber_id: String,
    pub sdp: Option<String>, // 添加SDP字段，用于携带Offer
    #[serde(rename = "type")]
    pub sdp_type: Option<String>, // "offer" 或 "answer"
}

/// 停止推流参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UnpublishParams {
    pub room_id: String,
    pub peer_id: String,
}

/// 停止订阅参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UnsubscribeParams {
    pub room_id: String,
    pub publisher_id: String,
    pub subscriber_id: String,
}

/// 创建DataChannel参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CreateDataChannelParams {
    pub room_id: String,
    pub peer_id: String,
    pub label: String,
}

/// DataChannel消息参数
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataChannelMessageParams {
    pub room_id: String,
    pub peer_id: String,
    pub label: String,
    pub data: Vec<u8>,
    pub is_text: bool,
}

impl SignalingMessage {
    /// 创建请求消息
    pub fn request(method: SignalingMethod, params: SignalingParams, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method,
            params,
            id: Some(id),
        }
    }

    /// 创建通知消息(无需响应)
    pub fn notification(method: SignalingMethod, params: SignalingParams) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method,
            params,
            id: None,
        }
    }
}
