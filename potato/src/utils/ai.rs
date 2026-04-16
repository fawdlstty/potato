pub struct OpenAISender {
    id: String,
    object: String,
    model: String,
    role: String,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl OpenAISender {
    pub async fn new(
        id: impl Into<String>,
        object: impl Into<String>,
        model: impl Into<String>,
        role: impl Into<String>,
        buffer_size: usize,
    ) -> anyhow::Result<(Self, crate::HttpResponse)> {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
        let obj = Self {
            id: id.into(),
            object: object.into(),
            model: model.into(),
            role: role.into(),
            tx,
        };

        let root = serde_json::to_string(&serde_json::json!({
            "id": obj.id,
            "object": obj.object,
            "created": chrono::Utc::now().timestamp(),
            "model": obj.model,
            "choices": [{
                "index": 0,
                "delta": {
                    "role": obj.role,
                },
                "finish_reason": null,
            }]
        }))?;
        let payload = format!("data: {root}\n\n");
        obj.tx.send(payload.into_bytes()).await?;

        Ok((obj, crate::HttpResponse::sse(rx)))
    }

    pub async fn send(&self, message: impl Into<String>) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "id": self.id,
            "object": self.object,
            "created": chrono::Utc::now().timestamp(),
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": {
                    "content": message.into(),
                },
                "finish_reason": null,
            }]
        }))?;
        let payload = format!("data: {root}\n\n");
        self.tx.send(payload.into_bytes()).await?;
        Ok(())
    }

    pub async fn send_finish(&self, finish_reason: impl Into<String>) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "id": self.id,
            "object": self.object,
            "created": chrono::Utc::now().timestamp(),
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": finish_reason.into(),
            }]
        }))?;
        let payload = format!("data: {}\n\n", serde_json::to_string(&root)?);
        self.tx.send(payload.into_bytes()).await?;
        self.tx.send(b"data: [DONE]\n\n".to_vec()).await?;
        Ok(())
    }
}

pub struct ClaudeSender {
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl ClaudeSender {
    pub async fn new(
        id: impl Into<String>,
        model: impl Into<String>,
        role: impl Into<String>,
        buffer_size: usize,
    ) -> anyhow::Result<(Self, crate::HttpResponse)> {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
        let root = serde_json::to_string(&serde_json::json!({
            "type": "message_start",
            "message": {
                "id": id.into(),
                "type": "message",
                "role": role.into(),
                "model": model.into(),
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0
                }
            }
        }))?;
        let payload = format!("event: message_start\ndata: {root}\n\n");
        tx.send(payload.into_bytes()).await?;

        let root = serde_json::to_string(&serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "text",
                "text": ""
            }
        }))?;
        let payload = format!("event: content_block_start\ndata: {root}\n\n");
        tx.send(payload.into_bytes()).await?;

        Ok((Self { tx }, crate::HttpResponse::sse(rx)))
    }

    pub async fn send(&self, message: impl Into<String>) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "text": message.into()
            }
        }))?;
        let payload = format!("event: content_block_delta\ndata: {root}\n\n");
        self.tx.send(payload.into_bytes()).await?;
        Ok(())
    }

    pub async fn send_finish(&self) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "type": "content_block_stop",
            "index": 0
        }))?;
        let payload = format!("event: content_block_stop\ndata: {root}\n\n");
        self.tx.send(payload.into_bytes()).await?;

        let root = serde_json::to_string(&serde_json::json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": "end_turn",
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": 0
            }
        }))?;
        let payload = format!("event: message_delta\ndata: {root}\n\n");
        self.tx.send(payload.into_bytes()).await?;

        let root = serde_json::to_string(&serde_json::json!({
            "type": "message_stop"
        }))?;
        let payload = format!("event: message_stop\ndata: {root}\n\n");
        self.tx.send(payload.into_bytes()).await?;

        Ok(())
    }
}

pub struct OllamaSender {
    model: String,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl OllamaSender {
    pub async fn new(
        model: impl Into<String>,
        buffer_size: usize,
    ) -> anyhow::Result<(Self, crate::HttpResponse)> {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
        let model = model.into();

        let mut resp = crate::HttpResponse::sse(rx);
        resp.add_header("Content-Type".into(), "application/x-ndjson".into());

        Ok((Self { model, tx }, resp))
    }

    pub async fn send(&self, message: impl Into<String>) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "model": self.model,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "response": message.into(),
            "done": false
        }))?;
        // Ollama 使用 NDJSON 格式（newline-delimited JSON）
        let payload = format!("{root}\n");
        self.tx.send(payload.into_bytes()).await?;
        Ok(())
    }

    pub async fn send_finish(&self) -> anyhow::Result<()> {
        let root = serde_json::to_string(&serde_json::json!({
            "model": self.model,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "response": "",
            "done": true,
            "done_reason": "stop"
        }))?;
        let payload = format!("{root}\n");
        self.tx.send(payload.into_bytes()).await?;
        Ok(())
    }
}
