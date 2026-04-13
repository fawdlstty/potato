//! DataChannel封装模块
//!
//! 提供简化的DataChannel API

use anyhow::Result;
use bytes::Bytes;
use std::sync::Arc;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

/// DataChannel包装器
pub struct DataChannelWrapper {
    channel: Arc<RTCDataChannel>,
}

impl DataChannelWrapper {
    /// 创建DataChannel包装器
    pub fn new(channel: Arc<RTCDataChannel>) -> Self {
        Self { channel }
    }

    /// 发送文本消息
    pub async fn send(&self, text: &str) -> Result<()> {
        self.channel.send(&Bytes::from(text.to_string())).await?;
        Ok(())
    }

    /// 发送二进制数据
    pub async fn send_binary(&self, data: &[u8]) -> Result<()> {
        self.channel.send(&Bytes::copy_from_slice(data)).await?;
        Ok(())
    }

    /// 设置消息回调
    pub fn on_message<F>(&self, callback: F)
    where
        F: Fn(DataChannelMessage) + Send + Sync + 'static,
    {
        let cb = Arc::new(callback);
        self.channel.on_message(Box::new(move |msg| {
            let cb = Arc::clone(&cb);
            Box::pin(async move {
                cb(msg);
            })
        }));
    }

    /// 获取标签
    pub fn label(&self) -> String {
        self.channel.label().to_string()
    }
}
