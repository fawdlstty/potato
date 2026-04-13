//! 媒体流处理模块
//!
//! 自动处理RTP包的接收、解析和分发

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use webrtc::rtp::packet::Packet as RtpPacket;

/// 媒体类型
#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    Video,
    Audio,
    Data,
}

/// 媒体流配置
#[derive(Debug, Clone)]
pub struct MediaStreamConfig {
    pub media_type: MediaType,
    pub buffer_size: usize,
    pub enable_retransmission: bool,
}

impl Default for MediaStreamConfig {
    fn default() -> Self {
        Self {
            media_type: MediaType::Video,
            buffer_size: 1000,
            enable_retransmission: true,
        }
    }
}

/// 媒体数据包
#[derive(Debug, Clone)]
pub struct MediaPacket {
    pub data: Vec<u8>,
    pub timestamp: u64,
    pub sequence: u32,
    pub is_keyframe: bool,
}

/// 媒体流处理器
pub struct MediaStreamHandler {
    config: MediaStreamConfig,
    tx: mpsc::Sender<MediaPacket>,
}

impl MediaStreamHandler {
    /// 创建新的媒体流处理器
    pub fn new(config: MediaStreamConfig) -> (Self, mpsc::Receiver<MediaPacket>) {
        let (tx, rx) = mpsc::channel(config.buffer_size);
        (Self { config, tx }, rx)
    }

    /// 发送媒体包（内部使用）
    #[allow(dead_code)]
    pub(crate) fn tx(&self) -> &mpsc::Sender<MediaPacket> {
        &self.tx
    }

    /// 处理接收到的RTP包
    #[allow(dead_code)]
    pub(crate) async fn handle_incoming_rtp(&self, rtp_packet: RtpPacket) -> Result<()> {
        let packet = MediaPacket {
            data: rtp_packet.payload.to_vec(),
            timestamp: rtp_packet.header.timestamp as u64,
            sequence: rtp_packet.header.sequence_number as u32,
            is_keyframe: self.is_keyframe(&rtp_packet),
        };

        if let Err(_) = self.tx.send(packet).await {
            // 接收端已关闭,忽略错误
        }
        Ok(())
    }

    /// 判断是否是关键帧
    #[allow(dead_code)]
    fn is_keyframe(&self, packet: &RtpPacket) -> bool {
        if self.config.media_type != MediaType::Video {
            return false;
        }
        // 简化的关键帧检测
        packet.payload.first().map_or(false, |b| (*b & 0x1F) == 5)
    }
}

/// 媒体流管理器
pub struct MediaStreamManager {
    streams: Arc<RwLock<HashMap<String, Arc<Mutex<MediaStreamHandler>>>>>,
    receivers: Arc<RwLock<HashMap<String, mpsc::Receiver<MediaPacket>>>>,
}

impl MediaStreamManager {
    pub fn new() -> Self {
        Self {
            streams: Arc::new(RwLock::new(HashMap::new())),
            receivers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册媒体流
    pub async fn register_stream(&self, stream_id: &str, config: MediaStreamConfig) -> Result<()> {
        let (handler, rx) = MediaStreamHandler::new(config);
        let handler = Arc::new(Mutex::new(handler));

        let mut streams = self.streams.write().await;
        streams.insert(stream_id.to_string(), handler);

        let mut receivers = self.receivers.write().await;
        receivers.insert(stream_id.to_string(), rx);

        Ok(())
    }

    /// 获取媒体流接收器
    pub async fn get_stream_receiver(
        &self,
        stream_id: &str,
    ) -> Option<mpsc::Receiver<MediaPacket>> {
        let mut receivers = self.receivers.write().await;
        receivers.remove(stream_id)
    }

    /// 移除媒体流
    pub async fn remove_stream(&self, stream_id: &str) -> Result<()> {
        let mut streams = self.streams.write().await;
        streams.remove(stream_id);

        let mut receivers = self.receivers.write().await;
        receivers.remove(stream_id);

        Ok(())
    }

    /// 分发RTP包
    #[allow(dead_code)]
    pub(crate) async fn dispatch_rtp_packet(
        &self,
        stream_id: &str,
        rtp_packet: RtpPacket,
    ) -> Result<()> {
        let streams = self.streams.read().await;
        if let Some(handler) = streams.get(stream_id) {
            let handler = handler.lock().await;
            handler.handle_incoming_rtp(rtp_packet).await?;
        }
        Ok(())
    }

    /// 直接分发媒体包（简化版本）
    pub(crate) async fn dispatch_rtp_packet_direct(
        &self,
        stream_id: &str,
        packet: MediaPacket,
    ) -> Result<()> {
        let streams = self.streams.read().await;
        if let Some(handler) = streams.get(stream_id) {
            let handler = handler.lock().await;
            if let Err(_) = handler.tx.send(packet).await {
                // 接收端已关闭
            }
        }
        Ok(())
    }
}
