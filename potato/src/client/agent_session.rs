use crate::Session;
use serde::{Deserialize, Serialize};

/// 统一的思考强度级别
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ThinkingMode {
    /// 禁用思考模式
    Disabled,
    /// 低强度思考（快速响应）
    Low,
    /// 中等强度思考
    Medium,
    /// 高强度思考
    High,
    /// 极高强度思考
    XHigh,
    /// 最大强度思考（最耗资源）
    Max,
}

impl ThinkingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingMode::Disabled => "disabled",
            ThinkingMode::Low => "low",
            ThinkingMode::Medium => "medium",
            ThinkingMode::High => "high",
            ThinkingMode::XHigh => "xhigh",
            ThinkingMode::Max => "max",
        }
    }
}

impl std::fmt::Display for ThinkingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 消息角色
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 单条会话消息
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// LLM 提供商类型
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LlmProvider {
    OpenAI,
    Anthropic,
    Ollama,
    OpenCode,
}

/// 流式响应的单个数据块
#[derive(Clone, Debug)]
pub enum StreamChunk {
    /// 文本内容块
    Content(String),
    /// 流结束
    Done,
}

/// 可用的模型信息
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider_id: String,
}

/// Agent 客户端会话，支持多轮对话
pub struct AgentClientSession {
    provider: LlmProvider,
    base_url: String,
    api_key: Option<String>,
    model: Option<String>,
    session: Session,
    messages: Vec<ChatMessage>,
    /// OpenCode serve 的 session ID（仅 OpenCode provider 使用）
    opencode_session_id: Option<String>,
    /// OpenCode serve 的上一条消息 ID（用于构建消息链）
    opencode_parent_id: Option<String>,
    /// 思考强度模式
    thinking_mode: ThinkingMode,
}

impl AgentClientSession {
    /// 创建客户端会话
    ///
    /// # 参数
    /// - `provider`: LLM 提供商类型
    /// - `base_url`: API 基础地址，如 `https://api.openai.com`
    /// - `api_key`: API 密钥，Ollama 等无需密钥的可传 None
    pub fn new(
        provider: LlmProvider,
        base_url: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            provider,
            base_url: base_url.into(),
            api_key,
            model: None,
            session: Session::new(),
            messages: Vec::new(),
            opencode_session_id: None,
            opencode_parent_id: None,
            thinking_mode: ThinkingMode::High, // 默认高强度思考
        }
    }

    /// 添加系统提示词
    pub fn set_system_prompt(&mut self, prompt: impl Into<String>) {
        self.messages
            .push(ChatMessage::new(MessageRole::System, prompt));
    }

    /// 异步设置模型并验证其是否在可用列表中
    pub async fn set_model(&mut self, model: impl Into<String>) -> anyhow::Result<()> {
        let model = model.into();
        // Anthropic provider 没有标准模型列表 API，跳过验证
        if self.provider != LlmProvider::Anthropic {
            let available_models = self.list_models().await?;
            let exists = available_models.iter().any(|m| m.id == model);
            if !exists {
                return Err(anyhow::anyhow!(
                    "Model '{model}' is invalid. Use list_models() to get valid models"
                ));
            }
        }
        self.model = Some(model);
        Ok(())
    }

    /// 获取当前设置的模型
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// 获取可用模型列表
    pub async fn list_models(&mut self) -> anyhow::Result<Vec<ModelInfo>> {
        match self.provider {
            LlmProvider::OpenCode => {
                Self::list_models_opencode(&self.base_url, &mut self.session).await
            }
            LlmProvider::OpenAI => {
                Self::list_models_openai(&self.base_url, &self.api_key, &mut self.session).await
            }
            LlmProvider::Ollama => {
                Self::list_models_ollama(&self.base_url, &mut self.session).await
            }
            LlmProvider::Anthropic => Ok(vec![]), // Anthropic 没有标准模型列表 API
        }
    }

    async fn list_models_opencode(
        base_url: &str,
        session: &mut Session,
    ) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{}/config/providers", base_url);
        let mut res = session.get(&url, vec![]).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to list OpenCode models: HTTP {}",
                res.http_code
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&response_text)?;
        let mut models = Vec::new();
        if let Some(providers) = json["providers"].as_array() {
            for provider in providers {
                let provider_id = provider["id"].as_str().unwrap_or("unknown").to_string();
                if let Some(provider_models) = provider["models"].as_object() {
                    for (model_id, model_info) in provider_models {
                        let name = model_info["name"].as_str().unwrap_or(model_id).to_string();
                        models.push(ModelInfo {
                            id: format!("{}:{}", provider_id, model_id),
                            name,
                            provider_id: provider_id.clone(),
                        });
                    }
                }
            }
        }
        Ok(models)
    }

    async fn list_models_openai(
        base_url: &str,
        api_key: &Option<String>,
        session: &mut Session,
    ) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{}/v1/models", base_url);
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(ref key) = api_key {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        let mut args = Vec::new();
        for (k, v) in headers {
            args.push(crate::Headers::Custom((k, v)));
        }
        let mut res = session.get(&url, args).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to list OpenAI models: HTTP {}",
                res.http_code
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&response_text)?;
        let mut models = Vec::new();
        if let Some(data) = json["data"].as_array() {
            for item in data {
                let id = item["id"].as_str().unwrap_or("").to_string();
                if !id.is_empty() {
                    models.push(ModelInfo {
                        id: id.clone(),
                        name: id,
                        provider_id: "openai".to_string(),
                    });
                }
            }
        }
        Ok(models)
    }

    async fn list_models_ollama(
        base_url: &str,
        session: &mut Session,
    ) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{}/api/tags", base_url);
        let mut res = session.get(&url, vec![]).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to list Ollama models: HTTP {}",
                res.http_code
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&response_text)?;
        let mut models = Vec::new();
        if let Some(data) = json["models"].as_array() {
            for item in data {
                let id = item["name"].as_str().unwrap_or("").to_string();
                if !id.is_empty() {
                    models.push(ModelInfo {
                        id: id.clone(),
                        name: id,
                        provider_id: "ollama".to_string(),
                    });
                }
            }
        }
        Ok(models)
    }

    /// 检查模型是否已设置
    fn ensure_model(&self) -> anyhow::Result<&str> {
        self.model
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Model not set. Call set_model() first."))
    }

    /// 获取当前会话历史
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// 清空会话历史（保留 system prompt）
    pub fn clear_messages(&mut self) {
        let system_msgs: Vec<ChatMessage> = self
            .messages
            .drain(..)
            .filter(|m| m.role == MessageRole::System)
            .collect();
        self.messages = system_msgs;
    }

    /// 发送消息并获取完整响应（非流式）
    pub async fn chat(&mut self, message: impl Into<String>) -> anyhow::Result<String> {
        let user_msg = ChatMessage::new(MessageRole::User, message);
        self.messages.push(user_msg);

        // OpenCode provider 需要特殊处理：先创建 session，再发送消息
        if self.provider == LlmProvider::OpenCode {
            return self.chat_opencode(false).await;
        }

        let (url, body, headers) = self.build_request(false)?;
        let mut args = Vec::new();
        for (k, v) in headers {
            args.push(crate::Headers::Custom((k, v)));
        }

        let mut res = self.session.post_json(&url, body, args).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "HTTP error {}: {}",
                res.http_code,
                response_text
            ));
        }

        let content = self.parse_response(&response_text)?;

        self.messages
            .push(ChatMessage::new(MessageRole::Assistant, content.clone()));
        Ok(content)
    }

    /// 获取底层 Session 的可变引用，用于高级操作（如强制重连）
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// 设置思考强度
    pub fn set_thinking_mode(&mut self, mode: ThinkingMode) {
        self.thinking_mode = mode;
    }

    /// 获取当前思考强度
    pub fn thinking_mode(&self) -> &ThinkingMode {
        &self.thinking_mode
    }

    /// OpenCode serve 专用：创建 session 并发送消息
    async fn chat_opencode(&mut self, _stream: bool) -> anyhow::Result<String> {
        // 如果还没有 opencode session，先创建一个
        if self.opencode_session_id.is_none() {
            let create_url = format!("{}/session", self.base_url);
            let create_body = serde_json::json!({"title": "potato-agent-session"});
            let mut create_res = self
                .session
                .post_json(&create_url, create_body, vec![])
                .await?;
            let create_data = create_res.body.data().await;
            let create_text = String::from_utf8_lossy(create_data).to_string();
            if create_res.http_code != 200 {
                return Err(anyhow::anyhow!(
                    "OpenCode create session failed {}: {}",
                    create_res.http_code,
                    create_text
                ));
            }
            let create_json: serde_json::Value = serde_json::from_str(&create_text)?;
            self.opencode_session_id = Some(
                create_json["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("OpenCode session response missing id"))?
                    .to_string(),
            );
        }

        let session_id = self.opencode_session_id.as_ref().unwrap();
        let url = format!("{}/session/{}/message", self.base_url, session_id);

        // 构建 parts：当前用户消息
        let last_msg = self
            .messages
            .last()
            .ok_or_else(|| anyhow::anyhow!("No message to send"))?;
        let parts = serde_json::json!([{"type": "text", "text": last_msg.content}]);

        // 解析 model 为 providerID 和 modelID
        let (provider_id, model_id) = self.parse_opencode_model()?;

        let mut body = serde_json::json!({
            "parts": parts,
            "model": {
                "providerID": provider_id,
                "modelID": model_id,
            },
        });

        // 如果有 parentID，添加到请求体中
        if let Some(ref parent_id) = self.opencode_parent_id {
            body["parentID"] = serde_json::Value::String(parent_id.clone());
        }

        // 发送请求，如果返回空响应则重试（OpenCode 服务端偶数次请求可能返回空）
        let mut response_text = String::new();
        for attempt in 0..3 {
            let mut res = self.session.post_json(&url, body.clone(), vec![]).await?;
            let body_data = res.body.data().await;
            response_text = String::from_utf8_lossy(body_data).to_string();
            if res.http_code != 200 {
                return Err(anyhow::anyhow!(
                    "OpenCode message failed {}: {}",
                    res.http_code,
                    response_text
                ));
            }
            if !response_text.trim().is_empty() {
                break;
            }
            // 空响应，等待后重试
            if attempt < 2 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                // 强制重新连接，使用新连接重试
                self.session.force_reconnect();
            }
        }

        let content = self.parse_opencode_response(&response_text)?;

        // 从响应中提取 parentID（用户消息的 ID），作为下一次请求的 parentID
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if let Some(parent_id) = json["info"]["parentID"].as_str() {
                self.opencode_parent_id = Some(parent_id.to_string());
            }
        }

        self.messages
            .push(ChatMessage::new(MessageRole::Assistant, content.clone()));
        Ok(content)
    }

    /// 解析 OpenCode 的 model 配置字符串为 providerID 和 modelID
    fn parse_opencode_model(&self) -> anyhow::Result<(String, String)> {
        let model = self.ensure_model()?;
        // 格式: "providerID:modelID" 或直接用 model 字段作为 modelID，provider 默认为 opencode
        if let Some(pos) = model.find(':') {
            let provider_id = model[..pos].to_string();
            let model_id = model[pos + 1..].to_string();
            Ok((provider_id, model_id))
        } else {
            Ok(("opencode".to_string(), model.to_string()))
        }
    }

    /// 解析 OpenCode serve 的响应文本
    fn parse_opencode_response(&self, text: &str) -> anyhow::Result<String> {
        if text.trim().is_empty() {
            return Err(anyhow::anyhow!("OpenCode response is empty"));
        }
        let json: serde_json::Value = serde_json::from_str(text)?;
        let parts = json["parts"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("OpenCode response missing parts"))?;
        let mut result = String::new();
        for part in parts {
            if let Some(text) = part["text"].as_str() {
                result.push_str(text);
            }
        }
        Ok(result)
    }

    /// 发送消息并获取流式响应
    pub async fn chat_stream(
        &mut self,
        message: impl Into<String>,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let user_msg = ChatMessage::new(MessageRole::User, message);
        self.messages.push(user_msg);

        // OpenCode provider 暂不支持流式响应，使用非流式方式模拟
        if self.provider == LlmProvider::OpenCode {
            let content = self.chat_opencode(false).await?;
            let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(64);
            tokio::spawn(async move {
                for line in content.lines() {
                    if tx
                        .send(StreamChunk::Content(line.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = tx.send(StreamChunk::Done).await;
            });
            return Ok(rx);
        }

        let (url, body, headers) = self.build_request(true)?;
        let mut args = Vec::new();
        for (k, v) in headers {
            args.push(crate::Headers::Custom((k, v)));
        }

        let mut res = self.session.post_json(&url, body, args).await?;
        if res.http_code != 200 {
            let body_data = res.body.data().await;
            return Err(anyhow::anyhow!(
                "HTTP error {}: {}",
                res.http_code,
                String::from_utf8_lossy(body_data)
            ));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(64);
        let provider = self.provider.clone();

        // 启动后台任务解析流式响应
        tokio::spawn(async move {
            let mut stream = res.body.stream_data();
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);
                match provider {
                    LlmProvider::OpenAI => {
                        while let Some(pos) = buffer.find("\n\n") {
                            let event = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();
                            if let Some(content) = Self::parse_openai_sse_chunk(&event) {
                                if content.is_empty() {
                                    continue;
                                }
                                if tx.send(StreamChunk::Content(content)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    LlmProvider::Anthropic => {
                        while let Some(pos) = buffer.find("\n\n") {
                            let event = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();
                            if let Some(content) = Self::parse_anthropic_sse_chunk(&event) {
                                if content.is_empty() {
                                    continue;
                                }
                                if tx.send(StreamChunk::Content(content)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    LlmProvider::Ollama => {
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].to_string();
                            buffer = buffer[pos + 1..].to_string();
                            if let Some(content) = Self::parse_ollama_ndjson_chunk(&line) {
                                if content.is_empty() {
                                    continue;
                                }
                                if tx.send(StreamChunk::Content(content)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    LlmProvider::OpenCode => {
                        // 不会走到这里，已在前面处理
                    }
                }
            }
            let _ = tx.send(StreamChunk::Done).await;
        });

        Ok(rx)
    }

    /// 完成一轮流式对话后，将助手回复追加到历史
    pub fn append_assistant_message(&mut self, content: impl Into<String>) {
        self.messages
            .push(ChatMessage::new(MessageRole::Assistant, content));
    }

    /// 将会话状态序列化为 JSON 字符串
    ///
    /// 序列化内容包括：provider、base_url、api_key、model、messages、opencode_session_id、opencode_parent_id、thinking_mode
    /// 注意：Session（HTTP 连接）不会被序列化，反序列化后会重新创建
    pub fn serialize(&self) -> anyhow::Result<String> {
        let state = serde_json::json!({
            "provider": self.provider,
            "base_url": self.base_url,
            "api_key": self.api_key,
            "model": self.model,
            "messages": self.messages,
            "opencode_session_id": self.opencode_session_id,
            "opencode_parent_id": self.opencode_parent_id,
            "thinking_mode": self.thinking_mode,
        });
        Ok(state.to_string())
    }

    /// 从 JSON 字符串反序列化恢复会话状态
    ///
    /// # 参数
    /// - `json`: 由 `serialize()` 生成的 JSON 字符串
    ///
    /// # 返回值
    /// 恢复后的 AgentClientSession，包含之前的所有记忆（messages）
    pub fn deserialize(json: &str) -> anyhow::Result<Self> {
        let state: serde_json::Value = serde_json::from_str(json)?;

        let provider: LlmProvider = serde_json::from_value(
            state
                .get("provider")
                .ok_or_else(|| anyhow::anyhow!("missing provider field"))?
                .clone(),
        )?;
        let base_url = state
            .get("base_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing base_url field"))?
            .to_string();
        let api_key = state
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let model = state
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let messages: Vec<ChatMessage> = serde_json::from_value(
            state
                .get("messages")
                .ok_or_else(|| anyhow::anyhow!("missing messages field"))?
                .clone(),
        )?;
        let opencode_session_id = state
            .get("opencode_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let opencode_parent_id = state
            .get("opencode_parent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let thinking_mode: ThinkingMode = state
            .get("thinking_mode")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(ThinkingMode::High); // 默认为高强度

        Ok(Self {
            provider,
            base_url,
            api_key,
            model,
            session: Session::new(),
            messages,
            opencode_session_id,
            opencode_parent_id,
            thinking_mode,
        })
    }

    fn build_request(
        &self,
        stream: bool,
    ) -> anyhow::Result<(String, serde_json::Value, Vec<(String, String)>)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(ref key) = self.api_key {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }

        match self.provider {
            LlmProvider::OpenAI => {
                let url = format!("{}/v1/chat/completions", self.base_url);
                let messages: Vec<serde_json::Value> = self
                    .messages
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "role": m.role.as_str(),
                            "content": m.content,
                        })
                    })
                    .collect();
                let mut body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": stream,
                });

                // OpenAI 兼容 API 使用 reasoning_effort 控制思考强度
                match &self.thinking_mode {
                    ThinkingMode::Disabled => {
                        // 禁用思考模式
                        body["thinking"] = serde_json::json!({"type": "disabled"});
                    }
                    m => {
                        body["reasoning_effort"] =
                            serde_json::Value::String(m.as_str().to_string());
                        // 启用思考模式
                        body["thinking"] = serde_json::json!({"type": "enabled"});
                    }
                }

                Ok((url, body, headers))
            }
            LlmProvider::Anthropic => {
                let url = format!("{}/v1/messages", self.base_url);
                let system_msg = self
                    .messages
                    .iter()
                    .find(|m| m.role == MessageRole::System)
                    .map(|m| m.content.clone());
                let messages: Vec<serde_json::Value> = self
                    .messages
                    .iter()
                    .filter(|m| m.role != MessageRole::System)
                    .map(|m| {
                        serde_json::json!({
                            "role": m.role.as_str(),
                            "content": m.content,
                        })
                    })
                    .collect();
                let mut body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "max_tokens": 4096,
                    "stream": stream,
                });

                // Anthropic 使用 output_config 控制思考强度
                match &self.thinking_mode {
                    ThinkingMode::Disabled => {
                        // 禁用思考模式
                        body["thinking"] = serde_json::json!({"type": "disabled"});
                    }
                    m => {
                        body["output_config"] = serde_json::json!({"effort": m.as_str()});
                        // 启用思考模式
                        body["thinking"] = serde_json::json!({"type": "enabled"});
                    }
                }

                if let Some(system) = system_msg {
                    body["system"] = serde_json::Value::String(system);
                }
                headers.push((
                    "x-api-key".to_string(),
                    self.api_key.clone().unwrap_or_default(),
                ));
                headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
                Ok((url, body, headers))
            }
            LlmProvider::Ollama => {
                let url = format!("{}/api/chat", self.base_url);
                let messages: Vec<serde_json::Value> = self
                    .messages
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "role": m.role.as_str(),
                            "content": m.content,
                        })
                    })
                    .collect();
                let mut body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": stream,
                });

                // Ollama 使用 think 参数控制思考强度
                body["think"] = match &self.thinking_mode {
                    ThinkingMode::Disabled => serde_json::Value::Bool(false),
                    m => serde_json::Value::String(m.as_str().to_string()),
                };

                Ok((url, body, headers))
            }
            LlmProvider::OpenCode => {
                // OpenCode 使用独立的 chat_opencode 方法处理，这里保留兼容逻辑
                let url = format!("{}/session/message", self.base_url);
                let body = serde_json::json!({});
                Ok((url, body, headers))
            }
        }
    }

    fn parse_response(&self, text: &str) -> anyhow::Result<String> {
        match self.provider {
            LlmProvider::OpenAI => {
                let json: serde_json::Value = serde_json::from_str(text)?;
                let content = json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("");
                Ok(content.to_string())
            }
            LlmProvider::OpenCode => self.parse_opencode_response(text),
            LlmProvider::Anthropic => {
                let json: serde_json::Value = serde_json::from_str(text)?;
                let mut result = String::new();
                if let Some(contents) = json["content"].as_array() {
                    for item in contents {
                        if item["type"].as_str() == Some("text") {
                            if let Some(text) = item["text"].as_str() {
                                result.push_str(text);
                            }
                        }
                    }
                }
                Ok(result)
            }
            LlmProvider::Ollama => {
                let json: serde_json::Value = serde_json::from_str(text)?;
                let content = json["message"]["content"].as_str().unwrap_or("");
                Ok(content.to_string())
            }
        }
    }

    fn parse_openai_sse_chunk(event: &str) -> Option<String> {
        for line in event.lines() {
            if line.starts_with("data: ") {
                let data = &line[6..];
                if data == "[DONE]" {
                    return Some(String::new());
                }
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                        return Some(content.to_string());
                    }
                }
            }
        }
        None
    }

    fn parse_anthropic_sse_chunk(event: &str) -> Option<String> {
        for line in event.lines() {
            if line.starts_with("data: ") {
                let data = &line[6..];
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(text) = json["delta"]["text"].as_str() {
                        return Some(text.to_string());
                    }
                }
            }
        }
        None
    }

    fn parse_ollama_ndjson_chunk(line: &str) -> Option<String> {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json["done"].as_bool().unwrap_or(false) {
                return Some(String::new());
            }
            if let Some(content) = json["message"]["content"].as_str() {
                return Some(content.to_string());
            }
        }
        None
    }
}
