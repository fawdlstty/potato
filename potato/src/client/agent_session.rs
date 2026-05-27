use crate::Session;
use serde::{Deserialize, Serialize};

/// 简单的 URL 编码，用于编码文件路径中的特殊字符
fn url_encode_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b':'
            | b'\\' => {
                result.push(byte as char);
            }
            b' ' => result.push_str("%20"),
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// 统一的思考强度级别
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReasoningEffort {
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

impl ReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningEffort::Disabled => "disabled",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
            ReasoningEffort::Max => "max",
        }
    }
}

impl std::fmt::Display for ReasoningEffort {
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
    pub ts_micros: i64,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        let content = content.into();
        let ts_micros = chrono::Utc::now().timestamp_micros();
        Self {
            role,
            ts_micros,
            content,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }
}

/// LLM 提供商类型
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LlmProvider {
    OpenAI,
    Anthropic,
    Ollama,
    OpenCode,
    Codex,
}

/// OpenCode 会话状态
#[derive(Clone, Debug)]
pub struct OpenCodeSession {
    /// OpenCode serve 的 session ID
    pub session_id: Option<String>,
    /// OpenCode serve 的上一条消息 ID（用于构建消息链）
    pub parent_id: Option<String>,
}

impl OpenCodeSession {
    pub fn new() -> Self {
        Self {
            session_id: None,
            parent_id: None,
        }
    }
}

/// Codex 会话状态
pub struct CodexSession {
    /// Codex app-server 的 WebSocket 连接
    pub ws: Option<crate::Websocket>,
    /// Codex app-server 的 thread ID
    pub thread_id: Option<String>,
    /// Codex app-server 的 JSON-RPC 请求 ID 计数器
    pub request_id: i64,
}

impl CodexSession {
    pub fn new() -> Self {
        Self {
            ws: None,
            thread_id: None,
            request_id: 0,
        }
    }

    /// 获取下一个 JSON-RPC 请求 ID
    pub fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }
}

/// 提供商特定的会话状态
pub enum ProviderSession {
    /// OpenCode 会话状态
    OpenCode(OpenCodeSession),
    /// Codex 会话状态
    Codex(CodexSession),
    /// 其他提供商无需特殊状态
    Other,
}

impl ProviderSession {
    /// 根据提供商类型创建对应的会话状态
    pub fn from_provider(provider: &LlmProvider) -> Self {
        match provider {
            LlmProvider::OpenCode => ProviderSession::OpenCode(OpenCodeSession::new()),
            LlmProvider::Codex => ProviderSession::Codex(CodexSession::new()),
            _ => ProviderSession::Other,
        }
    }

    /// 获取 OpenCode 会话状态的可变引用
    pub fn as_opencode_mut(&mut self) -> Option<&mut OpenCodeSession> {
        match self {
            ProviderSession::OpenCode(ref mut s) => Some(s),
            _ => None,
        }
    }

    /// 获取 Codex 会话状态的可变引用
    pub fn as_codex_mut(&mut self) -> Option<&mut CodexSession> {
        match self {
            ProviderSession::Codex(ref mut s) => Some(s),
            _ => None,
        }
    }

    /// 获取 OpenCode 会话状态的不可变引用
    pub fn as_opencode(&self) -> Option<&OpenCodeSession> {
        match self {
            ProviderSession::OpenCode(ref s) => Some(s),
            _ => None,
        }
    }

    /// 获取 Codex 会话状态的不可变引用
    pub fn as_codex(&self) -> Option<&CodexSession> {
        match self {
            ProviderSession::Codex(ref s) => Some(s),
            _ => None,
        }
    }
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
    /// 思考等级（推理强度），仅部分提供商的推理模型支持
    reasoning_effort: ReasoningEffort,
    /// 工作目录路径（源代码路径），仅 OpenCode 和 Codex provider 使用
    /// OpenCode: 作为 POST /session?directory= 查询参数传递
    /// Codex: 作为 thread/start 和 turn/start 的 cwd 参数传递
    working_directory: Option<String>,
    /// 提供商特定的会话状态
    provider_session: ProviderSession,
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
        let provider_session = ProviderSession::from_provider(&provider);
        Self {
            provider,
            base_url: base_url.into(),
            api_key,
            model: None,
            session: Session::new(),
            messages: Vec::new(),
            reasoning_effort: ReasoningEffort::Medium,
            working_directory: None,
            provider_session,
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

    /// 设置思考等级（推理强度）
    ///
    /// 仅部分提供商的推理模型支持此参数，如 OpenAI 的 o1/o3 系列模型。
    /// 如果传入的等级不被当前模型支持，API 调用可能会报错。
    ///
    /// # 参数
    /// - `effort`: 思考等级
    pub fn set_reasoning_effort(&mut self, effort: ReasoningEffort) {
        self.reasoning_effort = effort;
    }

    /// 获取当前设置的思考等级
    pub fn reasoning_effort(&self) -> &ReasoningEffort {
        &self.reasoning_effort
    }

    /// 设置工作目录（源代码路径）
    ///
    /// 仅 OpenCode 和 Codex provider 使用：
    /// - OpenCode: 创建 session 时作为 `?directory=` 查询参数传递
    /// - Codex: 启动 thread 时作为 `cwd` 参数传递
    ///
    /// # 参数
    /// - `path`: 工作目录的绝对路径
    pub fn set_working_directory(&mut self, path: Option<String>) {
        self.working_directory = path;
    }

    /// 获取当前设置的工作目录
    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }

    /// 获取当前模型支持的所有思考等级
    ///
    /// 通过查询模型详情 API 来获取支持的思考等级列表。
    /// 如果当前模型不支持思考等级或无法获取信息，则返回空列表。
    pub async fn list_reasoning_efforts(&mut self) -> anyhow::Result<Vec<String>> {
        let model = self.ensure_model()?.to_string();
        match self.provider {
            LlmProvider::OpenAI => self.list_reasoning_efforts_openai(&model).await,
            // Anthropic、Ollama、OpenCode、Codex 目前不通过此参数控制推理强度
            _ => Ok(vec![]),
        }
    }

    async fn list_reasoning_efforts_openai(&mut self, model: &str) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/models/{}", self.base_url, model);
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(ref key) = self.api_key {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        let mut args = Vec::new();
        for (k, v) in headers {
            args.push(crate::Headers::Custom((k, v)));
        }
        let mut res = self.session.get(&url, args).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "Failed to get model info: HTTP {}",
                res.http_code
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&response_text)?;

        // 尝试从模型详情中提取支持的思考等级
        // 如果 API 返回了 reasoning_effort 相关字段则使用，否则返回默认值
        let mut efforts = Vec::new();
        if let Some(capabilities) = json.get("capabilities") {
            if let Some(reasoning) = capabilities.get("reasoning") {
                if let Some(effort_levels) =
                    reasoning.get("effort_levels").and_then(|v| v.as_array())
                {
                    for level in effort_levels {
                        if let Some(s) = level.as_str() {
                            efforts.push(s.to_string());
                        }
                    }
                }
            }
        }
        Ok(efforts)
    }

    /// 获取可用模型列表
    pub async fn list_models(&mut self) -> anyhow::Result<Vec<ModelInfo>> {
        match self.provider {
            LlmProvider::OpenAI => {
                Self::list_models_openai(&self.base_url, &self.api_key, &mut self.session).await
            }
            LlmProvider::Anthropic => Ok(vec![]), // Anthropic 没有标准模型列表 API
            LlmProvider::Ollama => {
                Self::list_models_ollama(&self.base_url, &mut self.session).await
            }
            LlmProvider::OpenCode => {
                Self::list_models_opencode(&self.base_url, &mut self.session).await
            }
            LlmProvider::Codex => self.list_models_codex().await,
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
        let url = format!("{}/models", base_url);
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

    /// 设置会话历史消息（覆盖现有消息，保留 system prompt）
    pub fn set_messages(&mut self, messages: Vec<ChatMessage>) {
        self.messages = self
            .messages
            .drain(..)
            .filter(|m| m.role == MessageRole::System)
            .collect();
        self.messages.extend(messages);
    }

    /// 发送消息并获取完整响应（非流式）
    pub async fn chat(&mut self, message: impl Into<String>) -> anyhow::Result<String> {
        let user_msg = ChatMessage::new(MessageRole::User, message);
        self.messages.push(user_msg);

        // OpenCode provider 需要特殊处理：先创建 session，再发送消息
        if self.provider == LlmProvider::OpenCode {
            return self.chat_opencode(false).await;
        }

        // Codex provider 使用 WebSocket JSON-RPC 协议
        if self.provider == LlmProvider::Codex {
            return self.chat_codex().await;
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

    /// OpenCode serve 专用：创建 session 并发送消息
    async fn chat_opencode(&mut self, _stream: bool) -> anyhow::Result<String> {
        // 如果还没有 opencode session，先创建一个
        {
            let opencode = self
                .provider_session
                .as_opencode_mut()
                .ok_or_else(|| anyhow::anyhow!("Expected OpenCode provider session"))?;
            if opencode.session_id.is_none() {
                let mut create_url = format!("{}/session", self.base_url);
                // 如果设置了工作目录，作为 query 参数传递
                if let Some(ref dir) = self.working_directory {
                    create_url = format!("{}?directory={}", create_url, url_encode_path(dir));
                }
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
                opencode.session_id = Some(
                    create_json["id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("OpenCode session response missing id"))?
                        .to_string(),
                );
            }
        }

        let session_id = {
            let opencode = self.provider_session.as_opencode().unwrap();
            opencode.session_id.as_ref().unwrap().clone()
        };
        let url = format!("{}/session/{}/message", self.base_url, session_id);

        // 构建 parts：当前用户消息
        let last_msg_content = {
            let last_msg = self
                .messages
                .last()
                .ok_or_else(|| anyhow::anyhow!("No message to send"))?;
            last_msg.content.clone()
        };
        let parts = serde_json::json!([{"type": "text", "text": last_msg_content}]);

        // 解析 model 为 providerID 和 modelID
        let (provider_id, model_id) = self.parse_opencode_model()?;

        let parent_id = {
            let opencode = self.provider_session.as_opencode().unwrap();
            opencode.parent_id.clone()
        };

        let mut body = serde_json::json!({
            "parts": parts,
            "model": {
                "providerID": provider_id,
                "modelID": model_id,
            },
        });

        // 如果有 parentID，添加到请求体中
        if let Some(ref parent_id) = parent_id {
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
        {
            let opencode = self
                .provider_session
                .as_opencode_mut()
                .ok_or_else(|| anyhow::anyhow!("Expected OpenCode provider session"))?;
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response_text) {
                if let Some(parent_id) = json["info"]["parentID"].as_str() {
                    opencode.parent_id = Some(parent_id.to_string());
                }
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

    // ==================== Codex app-server WebSocket 协议支持 ====================

    /// 获取 Codex 会话的可变引用
    fn codex_session_mut(&mut self) -> anyhow::Result<&mut CodexSession> {
        self.provider_session
            .as_codex_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))
    }

    /// 获取下一个 JSON-RPC 请求 ID
    fn next_codex_request_id(&mut self) -> i64 {
        self.codex_session_mut().unwrap().next_request_id()
    }

    /// 确保 Codex WebSocket 连接已建立并完成初始化
    async fn ensure_codex_connected(&mut self) -> anyhow::Result<()> {
        let codex = self.codex_session_mut()?;
        if codex.ws.is_some() {
            return Ok(());
        }

        // 将 http:// 或 https:// 转换为 ws:// 或 wss://
        let ws_url = self
            .base_url
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);

        let mut ws = crate::Websocket::connect(&ws_url, vec![]).await?;

        // 1. 发送 initialize 请求
        let init_id = self.next_codex_request_id();
        let init_req = serde_json::json!({
            "method": "initialize",
            "id": init_id,
            "params": {
                "clientInfo": {
                    "name": "potato_agent",
                    "title": "Potato Agent Client",
                    "version": "0.1.0"
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }
        });
        ws.send_text(&init_req.to_string()).await?;

        // 等待 initialize 响应
        let init_res = Self::recv_codex_jsonrpc_response(&mut ws).await?;
        if init_res.get("error").is_some() {
            return Err(anyhow::anyhow!(
                "Codex initialize failed: {}",
                init_res["error"]
            ));
        }

        // 2. 发送 initialized 通知
        let init_notify = serde_json::json!({
            "method": "initialized",
            "params": {}
        });
        ws.send_text(&init_notify.to_string()).await?;

        let codex = self.codex_session_mut()?;
        codex.ws = Some(ws);

        // 3. 如果有保存的 thread_id，尝试恢复线程
        if let Some(thread_id) = codex.thread_id.clone() {
            let req_id = self.next_codex_request_id();
            {
                let codex = self.codex_session_mut()?;
                let ws = codex.ws.as_mut().unwrap();
                let req = serde_json::json!({
                    "method": "thread/resume",
                    "id": req_id,
                    "params": {
                        "threadId": thread_id
                    }
                });
                if let Err(e) = ws.send_text(&req.to_string()).await {
                    eprintln!("WARN: Failed to send thread/resume: {}", e);
                    codex.thread_id = None;
                }
            }
            let codex = self.codex_session_mut()?;
            if codex.thread_id.is_some() {
                let res = {
                    let codex = self.codex_session_mut()?;
                    let ws = codex.ws.as_mut().unwrap();
                    match Self::recv_codex_jsonrpc_response(ws).await {
                        Ok(res) => res,
                        Err(e) => {
                            eprintln!("WARN: Failed to receive thread/resume response: {}", e);
                            codex.thread_id = None;
                            return Ok(());
                        }
                    }
                };
                let codex = self.codex_session_mut()?;
                if res.get("error").is_some() {
                    eprintln!(
                        "WARN: Codex thread/resume failed: {}, will create new thread",
                        res["error"]
                    );
                    codex.thread_id = None;
                }
            }
        }
        Ok(())
    }

    /// 接收 Codex WebSocket 上的 JSON-RPC 响应（静态方法，避免借用冲突）
    async fn recv_codex_jsonrpc_response(
        ws: &mut crate::Websocket,
    ) -> anyhow::Result<serde_json::Value> {
        loop {
            match ws.recv().await? {
                crate::WsFrame::Text(text) => {
                    let val: serde_json::Value = serde_json::from_str(&text)?;
                    // 忽略通知，只返回响应（有 id 字段的）
                    if val.get("id").is_some() {
                        return Ok(val);
                    }
                }
                crate::WsFrame::Binary(_) => {}
            }
        }
    }

    /// 接收 Codex WebSocket 上的 JSON-RPC 通知（静态方法，避免借用冲突）
    async fn recv_codex_notification(
        ws: &mut crate::Websocket,
    ) -> anyhow::Result<serde_json::Value> {
        loop {
            match ws.recv().await? {
                crate::WsFrame::Text(text) => {
                    let val: serde_json::Value = serde_json::from_str(&text)?;
                    // 通知没有 id 字段，或者 method 字段存在
                    if val.get("method").is_some() {
                        return Ok(val);
                    }
                }
                crate::WsFrame::Binary(_) => {}
            }
        }
    }

    /// Codex provider: 获取可用模型列表
    async fn list_models_codex(&mut self) -> anyhow::Result<Vec<ModelInfo>> {
        self.ensure_codex_connected().await?;

        let req_id = self.next_codex_request_id();
        {
            let codex = self.codex_session_mut()?;
            let ws = codex.ws.as_mut().unwrap();
            let req = serde_json::json!({
                "method": "model/list",
                "id": req_id,
                "params": {}
            });
            ws.send_text(&req.to_string()).await?;
        }

        let res = {
            let codex = self.codex_session_mut()?;
            let ws = codex.ws.as_mut().unwrap();
            Self::recv_codex_jsonrpc_response(ws).await?
        };
        if let Some(error) = res.get("error") {
            return Err(anyhow::anyhow!("Codex model/list failed: {}", error));
        }

        let mut models = Vec::new();
        if let Some(data) = res["result"]["data"].as_array() {
            for item in data {
                let id = item["id"].as_str().unwrap_or("").to_string();
                let display_name = item["displayName"].as_str().unwrap_or(&id).to_string();
                if !id.is_empty() {
                    models.push(ModelInfo {
                        id: id.clone(),
                        name: display_name,
                        provider_id: "codex".to_string(),
                    });
                }
            }
        }
        Ok(models)
    }

    /// Codex provider: 发送消息并获取完整响应
    async fn chat_codex(&mut self) -> anyhow::Result<String> {
        self.ensure_codex_connected().await?;

        // 如果没有 thread，先创建一个
        let codex = self.codex_session_mut()?;
        if codex.thread_id.is_none() {
            let (model, model_provider) = self.parse_codex_model();
            let req_id = self.next_codex_request_id();

            let mut thread_params = serde_json::json!({
                "ephemeral": true
            });
            if let Some(model) = model {
                thread_params["model"] = serde_json::Value::String(model);
            }
            if let Some(model_provider) = model_provider {
                thread_params["modelProvider"] = serde_json::Value::String(model_provider);
            }
            if let Some(ref cwd) = self.working_directory {
                thread_params["cwd"] = serde_json::Value::String(cwd.clone());
            }

            {
                let codex = self.codex_session_mut()?;
                let ws = codex.ws.as_mut().unwrap();
                let req = serde_json::json!({
                    "method": "thread/start",
                    "id": req_id,
                    "params": thread_params
                });
                ws.send_text(&req.to_string()).await?;
            }

            let res = {
                let codex = self.codex_session_mut()?;
                let ws = codex.ws.as_mut().unwrap();
                Self::recv_codex_jsonrpc_response(ws).await?
            };
            if let Some(error) = res.get("error") {
                return Err(anyhow::anyhow!("Codex thread/start failed: {}", error));
            }

            let codex = self.codex_session_mut()?;
            codex.thread_id = Some(
                res["result"]["thread"]["id"]
                    .as_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Codex thread/start response missing thread.id")
                    })?
                    .to_string(),
            );
        }

        let thread_id = {
            let codex = self.codex_session_mut()?;
            codex.thread_id.as_ref().unwrap().clone()
        };

        // 构建输入：如果有历史消息（除了最后一条），将历史作为上下文附加
        let input_text = if self.messages.len() > 1 {
            let mut context = String::new();
            for msg in &self.messages[..self.messages.len() - 1] {
                match msg.role {
                    MessageRole::System => {
                        context.push_str(&format!("[系统提示]\n{}\n\n", msg.content));
                    }
                    MessageRole::User => {
                        context.push_str(&format!("[用户]\n{}\n\n", msg.content));
                    }
                    MessageRole::Assistant => {
                        context.push_str(&format!("[助手]\n{}\n\n", msg.content));
                    }
                }
            }
            let last_msg = self.messages.last().unwrap();
            context.push_str(&format!("[用户]\n{}\n", last_msg.content));
            context
        } else {
            self.messages.last().unwrap().content.clone()
        };

        let req_id = self.next_codex_request_id();
        let cwd = self.working_directory.clone();
        {
            let codex = self.codex_session_mut()?;
            let ws = codex.ws.as_mut().unwrap();
            let mut turn_params = serde_json::json!({
                "threadId": thread_id,
                "input": [
                    {
                        "type": "text",
                        "text": input_text
                    }
                ]
            });
            // 如果设置了工作目录，传递给 turn/start 作为 cwd
            if let Some(ref cwd) = cwd {
                turn_params["cwd"] = serde_json::Value::String(cwd.clone());
            }
            let req = serde_json::json!({
                "method": "turn/start",
                "id": req_id,
                "params": turn_params
            });
            ws.send_text(&req.to_string()).await?;
        }

        // 等待 turn/start 响应
        let turn_res = {
            let codex = self.codex_session_mut()?;
            let ws = codex.ws.as_mut().unwrap();
            Self::recv_codex_jsonrpc_response(ws).await?
        };
        if let Some(error) = turn_res.get("error") {
            return Err(anyhow::anyhow!("Codex turn/start failed: {}", error));
        }

        // 收集流式响应
        let mut agent_text = String::new();
        let mut turn_completed = false;

        while !turn_completed {
            let notification = {
                let codex = self.codex_session_mut()?;
                let ws = codex.ws.as_mut().unwrap();
                Self::recv_codex_notification(ws).await?
            };
            let method = notification["method"].as_str().unwrap_or("");

            match method {
                "item/agentMessage/delta" => {
                    if let Some(delta) = notification["params"]["delta"].as_str() {
                        agent_text.push_str(delta);
                    }
                }
                "turn/completed" => {
                    turn_completed = true;
                }
                "item/completed" => {
                    // 可以在这里处理完成的 item
                }
                "turn/started" | "item/started" | "thread/started" => {
                    // 忽略这些通知
                }
                "error" => {
                    return Err(anyhow::anyhow!(
                        "Codex error notification: {}",
                        notification["params"]
                    ));
                }
                _ => {
                    // 忽略其他通知
                }
            }
        }

        self.messages
            .push(ChatMessage::new(MessageRole::Assistant, agent_text.clone()));
        Ok(agent_text)
    }

    /// 解析 Codex 的 model 配置字符串
    /// 格式: "providerID:modelID" 或纯 "modelID"
    /// 返回 (model, model_provider)
    fn parse_codex_model(&self) -> (Option<String>, Option<String>) {
        let model = match self.model.as_deref() {
            Some(m) => m,
            None => return (None, None),
        };
        if let Some(pos) = model.find(':') {
            let provider_id = model[..pos].to_string();
            let model_id = model[pos + 1..].to_string();
            (Some(model_id), Some(provider_id))
        } else {
            (Some(model.to_string()), None)
        }
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
                    LlmProvider::Codex => {
                        // 不会走到这里，Codex 使用 WebSocket 协议
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
    /// 序列化内容包括：provider、base_url、api_key、model、messages、opencode_session_id、opencode_parent_id
    /// 注意：Session（HTTP 连接）不会被序列化，反序列化后会重新创建
    pub fn serialize(&self) -> anyhow::Result<String> {
        let opencode = self.provider_session.as_opencode();
        let codex = self.provider_session.as_codex();
        let state = serde_json::json!({
            "provider": self.provider,
            "base_url": self.base_url,
            "api_key": self.api_key,
            "model": self.model,
            "messages": self.messages,
            "reasoning_effort": self.reasoning_effort,
            "working_directory": self.working_directory,
            "opencode_session_id": opencode.and_then(|s| s.session_id.as_ref()),
            "opencode_parent_id": opencode.and_then(|s| s.parent_id.as_ref()),
            "codex_thread_id": codex.and_then(|s| s.thread_id.as_ref()),
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
        let reasoning_effort = state
            .get("reasoning_effort")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(ReasoningEffort::Medium);
        let working_directory = state
            .get("working_directory")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let opencode_session_id = state
            .get("opencode_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let opencode_parent_id = state
            .get("opencode_parent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let codex_thread_id = state
            .get("codex_thread_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut provider_session = ProviderSession::from_provider(&provider);
        if let ProviderSession::OpenCode(ref mut opencode) = provider_session {
            opencode.session_id = opencode_session_id;
            opencode.parent_id = opencode_parent_id;
        }
        if let ProviderSession::Codex(ref mut codex) = provider_session {
            codex.thread_id = codex_thread_id;
        }

        Ok(Self {
            provider,
            base_url,
            api_key,
            model,
            session: Session::new(),
            messages,
            reasoning_effort,
            working_directory,
            provider_session,
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
                let url = format!("{}/chat/completions", self.base_url);
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
                if self.reasoning_effort == ReasoningEffort::Disabled {
                    body["thinking"] = serde_json::json!({"type": "disabled"});
                } else {
                    body["thinking"] = serde_json::json!({"type": "enabled"});
                    body["reasoning_effort"] = self.reasoning_effort.as_str().into();
                }
                Ok((url, body, headers))
            }
            LlmProvider::Anthropic => {
                let url = format!("{}/messages", self.base_url);
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
                if let Some(system) = system_msg {
                    body["system"] = serde_json::Value::String(system);
                }
                headers.push((
                    "x-api-key".to_string(),
                    self.api_key.clone().unwrap_or_default(),
                ));
                headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
                if self.reasoning_effort == ReasoningEffort::Disabled {
                    body["thinking"] = serde_json::json!({"type": "disabled"});
                } else {
                    body["thinking"] = serde_json::json!({"type": "enabled"});
                    body["output_config"] =
                        serde_json::json!({ "effort": self.reasoning_effort.as_str() })
                }
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
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": stream,
                });
                Ok((url, body, headers))
            }
            LlmProvider::OpenCode => {
                // OpenCode 使用独立的 chat_opencode 方法处理，这里保留兼容逻辑
                let url = format!("{}/session/message", self.base_url);
                let body = serde_json::json!({});
                Ok((url, body, headers))
            }
            LlmProvider::Codex => {
                // Codex 使用 WebSocket JSON-RPC 协议，不通过 HTTP 发送
                let url = self.base_url.clone();
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
            LlmProvider::Codex => {
                // Codex 使用 WebSocket 协议，响应通过通知接收
                Ok(String::new())
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
