//! DataChannel封装模块

use anyhow::Result;
use bytes::BytesMut;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelMessage};

pub type DataChannelCallback = Arc<dyn Fn(RTCDataChannelMessage) + Send + Sync>;

/// DataChannel包装器
pub struct DataChannelWrapper {
    channel: Arc<dyn DataChannel>,
    label: String,
    callback: Arc<Mutex<Option<DataChannelCallback>>>,
    _sender: broadcast::Sender<RTCDataChannelMessage>,
}

impl DataChannelWrapper {
    /// 创建DataChannel包装器
    pub async fn new(channel: Arc<dyn DataChannel>) -> Result<Self> {
        let label = channel.label().await?;
        let (sender, _receiver) = broadcast::channel::<RTCDataChannelMessage>(256);
        let callback: Arc<Mutex<Option<DataChannelCallback>>> = Arc::new(Mutex::new(None));

        tokio::spawn({
            let channel = channel.clone();
            let callback = callback.clone();
            let sender = sender.clone();
            async move {
                while let Some(event) = channel.poll().await {
                    if let DataChannelEvent::OnMessage(msg) = event {
                        let _ = sender.send(msg.clone());
                        let callback = callback.lock().await.clone();
                        if let Some(callback) = callback {
                            callback(msg);
                        }
                    } else if matches!(
                        event,
                        DataChannelEvent::OnClose | DataChannelEvent::OnError
                    ) {
                        break;
                    }
                }
            }
        });

        Ok(Self {
            channel,
            label,
            callback,
            _sender: sender,
        })
    }

    /// 发送文本消息
    pub async fn send(&self, text: &str) -> Result<()> {
        self.channel.send_text(text).await?;
        Ok(())
    }

    /// 发送二进制数据
    pub async fn send_binary(&self, data: &[u8]) -> Result<()> {
        self.channel.send(BytesMut::from(data)).await?;
        Ok(())
    }

    /// 设置消息回调
    pub fn on_message<F>(&self, callback: F)
    where
        F: Fn(RTCDataChannelMessage) + Send + Sync + 'static,
    {
        if let Ok(mut callback_slot) = self.callback.try_lock() {
            *callback_slot = Some(Arc::new(callback));
        }
    }

    /// 获取标签
    pub fn label(&self) -> String {
        self.label.clone()
    }
}
