use crate::Session;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

type AgentRequestParts = (String, serde_json::Value, Vec<(String, String)>);
type DsfRequestParts = (String, serde_json::Value, Vec<(String, String)>, String);

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
#[serde(rename_all = "snake_case")]
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
    #[serde(rename = "xhigh")]
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
    ZCode,
    Dsf,
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

impl Default for OpenCodeSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Codex 会话状态
pub struct CodexSession {
    /// Codex app-server 的 WebSocket 连接
    pub ws: Option<crate::Websocket>,
    /// Codex app-server 的 thread ID
    pub thread_id: Option<String>,
    /// Codex app-server 的 JSON-RPC 请求 ID 计数器
    pub request_id: std::sync::atomic::AtomicI64,
}

impl CodexSession {
    pub fn new() -> Self {
        Self {
            ws: None,
            thread_id: None,
            request_id: std::sync::atomic::AtomicI64::new(0),
        }
    }

    /// 获取下一个 JSON-RPC 请求 ID
    pub fn next_request_id(&self) -> i64 {
        self.request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }
}

impl Default for CodexSession {
    fn default() -> Self {
        Self::new()
    }
}

/// ZCode app-server 会话状态
pub struct ZCodeSession {
    pub child: Option<Child>,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<tokio::io::Lines<BufReader<ChildStdout>>>,
    pub session_id: Option<String>,
    pub session_ready: bool,
    pub subscribed_session_id: Option<String>,
    pub request_id: i64,
    pub pending_events: VecDeque<serde_json::Value>,
}

impl ZCodeSession {
    pub fn new() -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            session_id: None,
            session_ready: false,
            subscribed_session_id: None,
            request_id: 0,
            pending_events: VecDeque::new(),
        }
    }

    pub fn next_request_id(&mut self) -> i64 {
        self.request_id += 1;
        self.request_id
    }
}

impl Default for ZCodeSession {
    fn default() -> Self {
        Self::new()
    }
}

/// DeepSeek Harness API gateway 会话状态
#[derive(Clone, Debug)]
pub struct DsfSession {
    pub session_id: Option<String>,
    pub selected_model: Option<String>,
    pub request_id: u64,
}

impl DsfSession {
    pub fn new() -> Self {
        Self {
            session_id: None,
            selected_model: None,
            request_id: 0,
        }
    }

    pub fn next_request_id(&mut self) -> String {
        self.request_id += 1;
        format!("potato-{}", self.request_id)
    }
}

impl Default for DsfSession {
    fn default() -> Self {
        Self::new()
    }
}

/// 提供商特定的会话状态
pub enum ProviderSession {
    /// OpenCode 会话状态
    OpenCode(OpenCodeSession),
    /// Codex 会话状态（Arc<tokio::Mutex<>> 包装，支持主线程与后台任务共享）
    Codex(std::sync::Arc<tokio::sync::Mutex<CodexSession>>),
    /// ZCode 会话状态（Arc<tokio::Mutex<>> 包装，支持流式后台任务）
    ZCode(std::sync::Arc<tokio::sync::Mutex<ZCodeSession>>),
    /// DeepSeek Harness 会话状态
    Dsf(DsfSession),
    /// 其他提供商无需特殊状态
    Other,
}

impl ProviderSession {
    /// 根据提供商类型创建对应的会话状态
    pub fn from_provider(provider: &LlmProvider) -> Self {
        match provider {
            LlmProvider::OpenCode => ProviderSession::OpenCode(OpenCodeSession::new()),
            LlmProvider::Codex => ProviderSession::Codex(std::sync::Arc::new(
                tokio::sync::Mutex::new(CodexSession::new()),
            )),
            LlmProvider::ZCode => ProviderSession::ZCode(std::sync::Arc::new(
                tokio::sync::Mutex::new(ZCodeSession::new()),
            )),
            LlmProvider::Dsf => ProviderSession::Dsf(DsfSession::new()),
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

    /// 获取 Codex 会话状态的 Arc 克隆
    pub fn as_codex_arc(&self) -> Option<std::sync::Arc<tokio::sync::Mutex<CodexSession>>> {
        match self {
            ProviderSession::Codex(arc) => Some(arc.clone()),
            _ => None,
        }
    }

    /// 获取 ZCode 会话状态的 Arc 克隆
    pub fn as_zcode_arc(&self) -> Option<std::sync::Arc<tokio::sync::Mutex<ZCodeSession>>> {
        match self {
            ProviderSession::ZCode(arc) => Some(arc.clone()),
            _ => None,
        }
    }

    /// 获取 DSF 会话状态的可变引用
    pub fn as_dsf_mut(&mut self) -> Option<&mut DsfSession> {
        match self {
            ProviderSession::Dsf(ref mut s) => Some(s),
            _ => None,
        }
    }

    /// 获取 DSF 会话状态的不可变引用
    pub fn as_dsf(&self) -> Option<&DsfSession> {
        match self {
            ProviderSession::Dsf(ref s) => Some(s),
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
            ProviderSession::Codex(_arc) => {
                // 注意：此方法在 Arc<Mutex<>> 下不安全，建议改用 as_codex_arc
                None
            }
            _ => None,
        }
    }

    /// 判断当前是否为 Codex 状态
    pub fn is_codex(&self) -> bool {
        matches!(self, ProviderSession::Codex(_))
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

#[derive(Debug, PartialEq)]
enum AgentStreamAction {
    Content(String),
    FinalContent(String),
    Done,
    Error(String),
    DsfRespond {
        rpc_id: String,
        result: serde_json::Value,
    },
    Ignore,
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
    /// 工作目录路径（源代码路径），仅 Agent 类 provider 使用
    /// OpenCode: 作为 POST /session?directory= 查询参数传递
    /// Codex: 作为 thread/start 和 turn/start 的 cwd 参数传递
    /// ZCode: 作为 session/create 的 workspace 路径传递
    /// DSF: 作为 session.create 的 cwd 参数传递
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
        if let Ok(available_models) = self.list_models().await {
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
    /// 仅 Agent 类 provider 使用：
    /// - OpenCode: 创建 session 时作为 `?directory=` 查询参数传递
    /// - Codex: 启动 thread 时作为 `cwd` 参数传递
    /// - ZCode: 创建 session 时作为 workspace 路径传递
    /// - DSF: 创建 session 时作为 `cwd` 参数传递
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
            // 部分 OpenAI 兼容服务不支持 /models/{model} 接口，返回空列表作为默认处理
            return Ok(vec![]);
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
            LlmProvider::Anthropic => Err(anyhow::anyhow!(
                "Anthropic does not have a standard model list API"
            )),
            LlmProvider::Ollama => {
                Self::list_models_ollama(&self.base_url, &mut self.session).await
            }
            LlmProvider::OpenCode => {
                Self::list_models_opencode(&self.base_url, &mut self.session).await
            }
            LlmProvider::Codex => self.list_models_codex().await,
            LlmProvider::ZCode => Err(anyhow::anyhow!(
                "ZCode does not expose a stable model list API"
            )),
            LlmProvider::Dsf => self.list_models_dsf().await,
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
        if self.provider == LlmProvider::ZCode {
            return self.chat_zcode().await;
        }
        if self.provider == LlmProvider::Dsf {
            return self.chat_dsf().await;
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
            let opencode = self
                .provider_session
                .as_opencode()
                .ok_or_else(|| anyhow::anyhow!("Expected OpenCode provider session"))?;
            opencode
                .session_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("OpenCode session not created"))?
                .clone()
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
            let opencode = self
                .provider_session
                .as_opencode()
                .ok_or_else(|| anyhow::anyhow!("Expected OpenCode provider session"))?;
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

    /// 确保 Codex WebSocket 连接已建立并完成初始化
    async fn ensure_codex_connected(&mut self) -> anyhow::Result<()> {
        let arc = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
        let codex = arc.lock().await;
        if codex.ws.is_some() {
            return Ok(());
        }
        drop(codex);

        // 将 http:// 或 https:// 转换为 ws:// 或 wss://
        let ws_url = self
            .base_url
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);

        // 构建 WebSocket 连接 headers，包含 Authorization
        let mut ws_headers = vec![];
        if let Some(ref api_key) = self.api_key {
            ws_headers.push(crate::Headers::Authorization(format!("Bearer {api_key}")));
        }

        let mut ws = crate::Websocket::connect(&ws_url, ws_headers).await?;

        // 1. 发送 initialize 请求
        let init_id = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?
            .lock()
            .await
            .next_request_id();
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

        let arc = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
        let saved_thread_id = {
            let mut codex = arc.lock().await;
            codex.ws = Some(ws);
            codex.thread_id.clone()
        };

        // 3. 如果有保存的 thread_id，尝试恢复线程
        if let Some(thread_id) = saved_thread_id {
            let req_id = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?
                .lock()
                .await
                .next_request_id();
            let mut resume_sent = false;
            {
                let arc = self
                    .provider_session
                    .as_codex_arc()
                    .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                let mut codex = arc.lock().await;
                let ws = codex
                    .ws
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
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
                } else {
                    resume_sent = true;
                }
            }
            if resume_sent {
                let res = {
                    let arc = self
                        .provider_session
                        .as_codex_arc()
                        .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                    let mut codex = arc.lock().await;
                    let ws = codex
                        .ws
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
                    match Self::recv_codex_jsonrpc_response(ws).await {
                        Ok(res) => res,
                        Err(e) => {
                            eprintln!("WARN: Failed to receive thread/resume response: {}", e);
                            codex.thread_id = None;
                            return Ok(());
                        }
                    }
                };
                let arc = self
                    .provider_session
                    .as_codex_arc()
                    .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                let mut codex = arc.lock().await;
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
    ///
    /// 根据 Codex 协议，服务端可能在响应前发送通知。此方法会：
    /// 1. 优先返回带 id 的 JSON-RPC 响应
    /// 2. 如果遇到 error 通知（如认证失败），立即返回错误
    /// 3. 忽略其他普通通知（如 thread/started）
    async fn recv_codex_jsonrpc_response(
        ws: &mut crate::Websocket,
    ) -> anyhow::Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, ws.recv()).await {
                Ok(Ok(crate::WsFrame::Text(text))) => {
                    let val: serde_json::Value = serde_json::from_str(&text)?;
                    // 有 id 字段的是响应，直接返回
                    if val.get("id").is_some() {
                        return Ok(val);
                    }
                    // 没有 id 但有 error 字段的是错误通知，立即报错
                    if val.get("error").is_some() {
                        return Err(anyhow::anyhow!(
                            "Codex server sent error notification: {}",
                            val["error"]
                        ));
                    }
                    // 其他通知（如 thread/started）忽略，继续等待响应
                }
                Ok(Ok(crate::WsFrame::Binary(_))) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "Timeout waiting for Codex JSON-RPC response"
                    ))
                }
            }
        }
    }

    /// 接收 Codex WebSocket 上的 JSON-RPC 通知（静态方法，避免借用冲突）
    async fn recv_codex_notification(
        ws: &mut crate::Websocket,
    ) -> anyhow::Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, ws.recv()).await {
                Ok(Ok(crate::WsFrame::Text(text))) => {
                    let val: serde_json::Value = serde_json::from_str(&text)?;
                    // 通知没有 id 字段，但有 method 字段
                    // 注意：error 通知也可能没有 id，需要处理
                    if val.get("method").is_some() || val.get("error").is_some() {
                        return Ok(val);
                    }
                }
                Ok(Ok(crate::WsFrame::Binary(_))) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(anyhow::anyhow!("Timeout waiting for Codex notification")),
            }
        }
    }

    /// Codex provider: 获取可用模型列表
    async fn list_models_codex(&mut self) -> anyhow::Result<Vec<ModelInfo>> {
        self.ensure_codex_connected().await?;

        let req_id = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?
            .lock()
            .await
            .next_request_id();
        {
            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let mut codex = arc.lock().await;
            let ws = codex
                .ws
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
            let req = serde_json::json!({
                "method": "model/list",
                "id": req_id,
                "params": {}
            });
            ws.send_text(&req.to_string()).await?;
        }

        let res = {
            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let mut codex = arc.lock().await;
            let ws = codex
                .ws
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
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

    /// DeepSeek Harness: 获取可用模型列表
    async fn list_models_dsf(&mut self) -> anyhow::Result<Vec<ModelInfo>> {
        let json = self.call_dsf("llm.models", serde_json::json!({})).await?;
        let mut models = Vec::new();
        if let Some(groups) = json["groups"].as_array() {
            for group in groups {
                let provider_id = group["id"].as_str().unwrap_or("unknown").to_string();
                if let Some(provider_models) = group["models"].as_array() {
                    for model in provider_models {
                        let model_id = model["id"].as_str().unwrap_or("").to_string();
                        if model_id.is_empty() {
                            continue;
                        }
                        let name = model["name"].as_str().unwrap_or(&model_id).to_string();
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

    /// 准备 Codex turn：确保连接、创建 thread（如需要）、发送 turn/start
    /// 返回确认 turn 已启动，此时 WebSocket 上即将开始流式通知
    async fn prepare_codex_turn(&mut self) -> anyhow::Result<()> {
        self.ensure_codex_connected().await?;

        // 如果没有 thread，先创建一个
        let arc = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
        let has_thread = {
            let codex = arc.lock().await;
            codex.thread_id.is_some()
        };
        if !has_thread {
            let (model, model_provider) = self.parse_codex_model();
            let req_id = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?
                .lock()
                .await
                .next_request_id();

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
                let arc = self
                    .provider_session
                    .as_codex_arc()
                    .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                let mut codex = arc.lock().await;
                let ws = codex
                    .ws
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
                let req = serde_json::json!({
                    "method": "thread/start",
                    "id": req_id,
                    "params": thread_params
                });
                ws.send_text(&req.to_string()).await?;
            }

            let res = {
                let arc = self
                    .provider_session
                    .as_codex_arc()
                    .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                let mut codex = arc.lock().await;
                let ws = codex
                    .ws
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
                Self::recv_codex_jsonrpc_response(ws).await?
            };
            if let Some(error) = res.get("error") {
                return Err(anyhow::anyhow!("Codex thread/start failed: {}", error));
            }

            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let mut codex = arc.lock().await;
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
            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let codex = arc.lock().await;
            codex
                .thread_id
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Codex thread not created"))?
                .clone()
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
            let last_msg = self
                .messages
                .last()
                .ok_or_else(|| anyhow::anyhow!("No message to send"))?;
            context.push_str(&format!("[用户]\n{}\n", last_msg.content));
            context
        } else {
            self.messages
                .last()
                .ok_or_else(|| anyhow::anyhow!("No message to send"))?
                .content
                .clone()
        };

        let req_id = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?
            .lock()
            .await
            .next_request_id();
        let cwd = self.working_directory.clone();
        {
            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let mut codex = arc.lock().await;
            let ws = codex
                .ws
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
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
            let arc = self
                .provider_session
                .as_codex_arc()
                .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
            let mut codex = arc.lock().await;
            let ws = codex
                .ws
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
            Self::recv_codex_jsonrpc_response(ws).await?
        };
        if let Some(error) = turn_res.get("error") {
            return Err(anyhow::anyhow!("Codex turn/start failed: {}", error));
        }

        Ok(())
    }

    /// Codex provider: 发送消息并获取完整响应
    async fn chat_codex(&mut self) -> anyhow::Result<String> {
        self.prepare_codex_turn().await?;

        // 收集流式响应
        let mut agent_text = String::new();
        let mut turn_completed = false;

        while !turn_completed {
            let notification = {
                let arc = self
                    .provider_session
                    .as_codex_arc()
                    .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;
                let mut codex = arc.lock().await;
                let ws = codex
                    .ws
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Codex WebSocket not connected"))?;
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

    /// Codex provider: 发送消息并获取流式响应
    async fn chat_stream_codex(
        &mut self,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        self.prepare_codex_turn().await?;

        // 克隆 Arc，供后台任务共享访问
        let codex_arc = self
            .provider_session
            .as_codex_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected Codex provider session"))?;

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(64);
        let messages =
            std::sync::Arc::new(std::sync::Mutex::new(std::mem::take(&mut self.messages)));

        // 启动后台任务监听 WebSocket 通知并实时转发
        tokio::spawn(async move {
            let mut assistant_content = String::new();
            let mut turn_completed = false;
            let mut error_occurred = false;

            while !turn_completed {
                let mut codex = codex_arc.lock().await;
                let ws = match codex.ws.as_mut() {
                    Some(ws) => ws,
                    None => {
                        error_occurred = true;
                        break;
                    }
                };
                match Self::recv_codex_notification(ws).await {
                    Ok(notification) => {
                        let method = notification["method"].as_str().unwrap_or("");
                        match method {
                            "item/agentMessage/delta" => {
                                if let Some(delta) = notification["params"]["delta"].as_str() {
                                    assistant_content.push_str(delta);
                                    // 实时更新历史记录
                                    if let Ok(mut msgs) = messages.lock() {
                                        if msgs
                                            .last()
                                            .is_some_and(|m| m.role == MessageRole::Assistant)
                                        {
                                            if let Some(last) = msgs.last_mut() {
                                                last.content = assistant_content.clone();
                                            }
                                        } else {
                                            msgs.push(ChatMessage::new(
                                                MessageRole::Assistant,
                                                assistant_content.clone(),
                                            ));
                                        }
                                    }
                                    if tx
                                        .send(StreamChunk::Content(delta.to_string()))
                                        .await
                                        .is_err()
                                    {
                                        // 接收端已关闭，提前结束
                                        turn_completed = true;
                                    }
                                }
                            }
                            "turn/completed" => {
                                turn_completed = true;
                            }
                            "item/completed" => {}
                            "turn/started" | "item/started" | "thread/started" => {}
                            "error" => {
                                let _ = tx
                                    .send(StreamChunk::Content(format!(
                                        "\n[Codex error: {}]",
                                        notification["params"]
                                    )))
                                    .await;
                                error_occurred = true;
                                turn_completed = true;
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamChunk::Content(format!(
                                "\n[Codex WebSocket error: {}]",
                                e
                            )))
                            .await;
                        error_occurred = true;
                        turn_completed = true;
                    }
                }
            }

            // 流结束时确保历史记录中已存在 Assistant 消息
            if !error_occurred {
                if let Ok(mut msgs) = messages.lock() {
                    if msgs.last().is_none_or(|m| m.role != MessageRole::Assistant) {
                        msgs.push(ChatMessage::new(MessageRole::Assistant, assistant_content));
                    }
                }
            }
            let _ = tx.send(StreamChunk::Done).await;
        });

        Ok(rx)
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

    // ==================== ZCode app-server stdio JSONL 协议支持 ====================

    fn zcode_command(&self) -> Vec<String> {
        let command = self.base_url.trim();
        if command.is_empty() {
            vec![
                "zcode".to_string(),
                "app-server".to_string(),
                "--stdio".to_string(),
            ]
        } else {
            let mut parts: Vec<String> = command
                .split_whitespace()
                .map(|part| part.to_string())
                .collect();
            if parts.len() == 1 {
                parts.push("app-server".to_string());
                parts.push("--stdio".to_string());
            }
            parts
        }
    }

    async fn ensure_zcode_process(&mut self) -> anyhow::Result<()> {
        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let mut zcode = zcode_arc.lock().await;
        if zcode.stdin.is_some() && zcode.stdout.is_some() {
            return Ok(());
        }

        let command = self.zcode_command();
        let program = command
            .first()
            .ok_or_else(|| anyhow::anyhow!("ZCode command is empty"))?;
        let mut child = tokio::process::Command::new(program);
        child
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        child.kill_on_drop(true);
        if let Some(model) = self.model.as_deref() {
            child.env("ZCODE_MODEL", model);
        }
        let mut child = child.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to start ZCode command '{}': {}",
                command.join(" "),
                e
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("ZCode stdin is not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("ZCode stdout is not available"))?;

        zcode.child = Some(child);
        zcode.stdin = Some(stdin);
        zcode.stdout = Some(BufReader::new(stdout).lines());
        zcode.session_ready = false;
        zcode.subscribed_session_id = None;
        Ok(())
    }

    async fn zcode_write_json(
        zcode: &mut ZCodeSession,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        let stdin = zcode
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ZCode stdin is not available"))?;
        stdin.write_all(value.to_string().as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn zcode_read_json(
        zcode: &mut ZCodeSession,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let stdout = zcode
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ZCode stdout is not available"))?;
        let line = tokio::time::timeout(timeout, stdout.next_line())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for ZCode JSONL message"))??
            .ok_or_else(|| anyhow::anyhow!("ZCode process closed stdout"))?;
        Ok(serde_json::from_str(&line)?)
    }

    async fn zcode_reply_to_server_request(
        zcode: &mut ZCodeSession,
        message: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let Some(id) = message.get("id").cloned() else {
            return Ok(());
        };
        let method = message["method"].as_str().unwrap_or("");
        let result = match method {
            "session/requestRuntimePreferences" => serde_json::json!({
                "nativeSearchEnhancementsEnabled": false,
                "memoryEnabled": false,
                "askUserQuestionAutoResolutionEnabled": false
            }),
            "interaction/requestPermission" => serde_json::json!({
                "decision": "deny",
                "reason": "potato AgentClientSession does not support interactive approvals"
            }),
            "interaction/requestUserInput" => serde_json::json!({
                "action": "decline",
                "reason": "potato AgentClientSession does not support interactive user input"
            }),
            _ => {
                let response = serde_json::json!({
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Unsupported ZCode server request: {method}")
                    }
                });
                return Self::zcode_write_json(zcode, response).await;
            }
        };
        let response = serde_json::json!({
            "id": id,
            "result": result
        });
        Self::zcode_write_json(zcode, response).await
    }

    async fn zcode_request(
        zcode: &mut ZCodeSession,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let id = zcode.next_request_id();
        let request = serde_json::json!({
            "id": id,
            "method": method,
            "params": params
        });
        Self::zcode_write_json(zcode, request).await?;

        loop {
            let message = Self::zcode_read_json(zcode, timeout).await?;
            if message.get("id").and_then(|v| v.as_i64()) == Some(id)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    return Err(anyhow::anyhow!("ZCode {method} failed: {}", error));
                }
                return Ok(message["result"].clone());
            }
            if message.get("method").and_then(|v| v.as_str()) == Some("session/event") {
                zcode.pending_events.push_back(message);
                continue;
            }
            if message.get("id").is_some() && message.get("method").is_some() {
                Self::zcode_reply_to_server_request(zcode, &message).await?;
            }
        }
    }

    async fn recv_zcode_event(zcode: &mut ZCodeSession) -> anyhow::Result<serde_json::Value> {
        if let Some(event) = zcode.pending_events.pop_front() {
            return Ok(event);
        }
        loop {
            let message = Self::zcode_read_json(zcode, std::time::Duration::from_secs(180)).await?;
            if message.get("method").and_then(|v| v.as_str()) == Some("session/event") {
                return Ok(message);
            }
            if message.get("id").is_some() && message.get("method").is_some() {
                Self::zcode_reply_to_server_request(zcode, &message).await?;
            }
        }
    }

    fn parse_zcode_event_action(event: &serde_json::Value) -> AgentStreamAction {
        let params = &event["params"];
        match params["type"].as_str().unwrap_or("") {
            "model.streaming" => {
                let payload = &params["payload"];
                if payload["kind"].as_str() == Some("text_delta") {
                    if let Some(delta) = payload["delta"].as_str() {
                        return AgentStreamAction::Content(delta.to_string());
                    }
                }
                AgentStreamAction::Ignore
            }
            "turn.completed" | "turn.terminal" => AgentStreamAction::Done,
            "turn.failed" => {
                let message = params["payload"]["error"]["message"]
                    .as_str()
                    .unwrap_or("ZCode turn failed")
                    .to_string();
                AgentStreamAction::Error(message)
            }
            _ => AgentStreamAction::Ignore,
        }
    }

    async fn ensure_zcode_session(&mut self) -> anyhow::Result<String> {
        self.ensure_zcode_process().await?;

        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let mut zcode = zcode_arc.lock().await;
        if zcode.session_ready {
            if let Some(session_id) = zcode.session_id.clone() {
                return Ok(session_id);
            }
        }

        let cwd = self
            .working_directory
            .clone()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
            })
            .ok_or_else(|| anyhow::anyhow!("Cannot resolve working directory for ZCode"))?;
        if let Some(session_id) = zcode.session_id.clone() {
            let resumed = Self::zcode_request(
                &mut zcode,
                "session/resume",
                serde_json::json!({
                    "sessionId": session_id,
                    "workspace": {
                        "workspacePath": cwd,
                        "workspaceKey": cwd
                    }
                }),
                std::time::Duration::from_secs(30),
            )
            .await
            .is_ok();
            if resumed {
                zcode.session_ready = true;
                return Ok(session_id);
            }
            zcode.session_id = None;
            zcode.subscribed_session_id = None;
        }

        let result = Self::zcode_request(
            &mut zcode,
            "session/create",
            serde_json::json!({
                "workspace": {
                    "workspacePath": cwd,
                    "workspaceKey": cwd
                },
                "mode": "yolo"
            }),
            std::time::Duration::from_secs(30),
        )
        .await?;
        let session_id = result["session"]["sessionId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("ZCode session/create response missing sessionId"))?
            .to_string();
        zcode.session_id = Some(session_id.clone());
        zcode.session_ready = true;
        Ok(session_id)
    }

    async fn ensure_zcode_subscribed(&mut self, session_id: &str) -> anyhow::Result<()> {
        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let mut zcode = zcode_arc.lock().await;
        if zcode.subscribed_session_id.as_deref() == Some(session_id) {
            return Ok(());
        }

        Self::zcode_request(
            &mut zcode,
            "session/subscribe",
            serde_json::json!({
                "sessionId": session_id,
                "deliveryKind": "desktop-continuous",
                "includeSnapshot": false,
                "afterSeq": 0
            }),
            std::time::Duration::from_secs(30),
        )
        .await?;
        zcode.subscribed_session_id = Some(session_id.to_string());
        Ok(())
    }

    async fn prepare_zcode_turn(&mut self) -> anyhow::Result<()> {
        let session_id = self.ensure_zcode_session().await?;
        self.ensure_zcode_subscribed(&session_id).await?;

        let content = self
            .messages
            .last()
            .ok_or_else(|| anyhow::anyhow!("No message to send"))?
            .content
            .clone();
        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let mut zcode = zcode_arc.lock().await;
        let result = Self::zcode_request(
            &mut zcode,
            "session/send",
            serde_json::json!({
                "sessionId": session_id,
                "content": content
            }),
            std::time::Duration::from_secs(30),
        )
        .await?;
        if result["accepted"].as_bool() == Some(false) {
            return Err(anyhow::anyhow!("ZCode session/send was not accepted"));
        }
        Ok(())
    }

    async fn chat_zcode(&mut self) -> anyhow::Result<String> {
        self.prepare_zcode_turn().await?;
        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let mut assistant_text = String::new();
        loop {
            let event = {
                let mut zcode = zcode_arc.lock().await;
                Self::recv_zcode_event(&mut zcode).await?
            };
            match Self::parse_zcode_event_action(&event) {
                AgentStreamAction::Content(text) => assistant_text.push_str(&text),
                AgentStreamAction::Done => break,
                AgentStreamAction::Error(message) => return Err(anyhow::anyhow!(message)),
                _ => {}
            }
        }
        self.messages.push(ChatMessage::new(
            MessageRole::Assistant,
            assistant_text.clone(),
        ));
        Ok(assistant_text)
    }

    async fn chat_stream_zcode(
        &mut self,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        self.prepare_zcode_turn().await?;
        let zcode_arc = self
            .provider_session
            .as_zcode_arc()
            .ok_or_else(|| anyhow::anyhow!("Expected ZCode provider session"))?;
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(64);
        let messages =
            std::sync::Arc::new(std::sync::Mutex::new(std::mem::take(&mut self.messages)));

        tokio::spawn(async move {
            let mut assistant_content = String::new();
            loop {
                let event = {
                    let mut zcode = zcode_arc.lock().await;
                    match Self::recv_zcode_event(&mut zcode).await {
                        Ok(event) => event,
                        Err(e) => {
                            let _ = tx
                                .send(StreamChunk::Content(format!(
                                    "\n[ZCode stdio error: {}]",
                                    e
                                )))
                                .await;
                            let _ = tx.send(StreamChunk::Done).await;
                            return;
                        }
                    }
                };
                match Self::parse_zcode_event_action(&event) {
                    AgentStreamAction::Content(text) => {
                        assistant_content.push_str(&text);
                        if let Ok(mut msgs) = messages.lock() {
                            if msgs
                                .last()
                                .is_some_and(|m| m.role == MessageRole::Assistant)
                            {
                                if let Some(last) = msgs.last_mut() {
                                    last.content = assistant_content.clone();
                                }
                            } else {
                                msgs.push(ChatMessage::new(
                                    MessageRole::Assistant,
                                    assistant_content.clone(),
                                ));
                            }
                        }
                        if tx.send(StreamChunk::Content(text)).await.is_err() {
                            return;
                        }
                    }
                    AgentStreamAction::Done => {
                        if let Ok(mut msgs) = messages.lock() {
                            if msgs.last().is_none_or(|m| m.role != MessageRole::Assistant) {
                                msgs.push(ChatMessage::new(
                                    MessageRole::Assistant,
                                    assistant_content.clone(),
                                ));
                            }
                        }
                        let _ = tx.send(StreamChunk::Done).await;
                        return;
                    }
                    AgentStreamAction::Error(message) => {
                        let _ = tx
                            .send(StreamChunk::Content(format!("\n[ZCode error: {message}]")))
                            .await;
                        let _ = tx.send(StreamChunk::Done).await;
                        return;
                    }
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    // ==================== DeepSeek Harness API gateway 协议支持 ====================

    fn dsf_url(&self, method: &str) -> String {
        format!("{}/api/{}", self.base_url.trim_end_matches('/'), method)
    }

    fn dsf_ws_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let ws_base = base
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1);
        format!("{}/api/{}", ws_base, path)
    }

    fn dsf_headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(ref key) = self.api_key {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        headers
    }

    fn next_dsf_rpc_id(&mut self) -> anyhow::Result<String> {
        self.provider_session
            .as_dsf_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected DSF provider session"))
            .map(|dsf| dsf.next_request_id())
    }

    fn build_dsf_request(
        &mut self,
        method: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<DsfRequestParts> {
        let rpc_id = self.next_dsf_rpc_id()?;
        let body = serde_json::json!({
            "type": "client-request",
            "rpcId": rpc_id,
            "method": method,
            "payload": payload
        });
        Ok((self.dsf_url(method), body, self.dsf_headers(), rpc_id))
    }

    async fn call_dsf(
        &mut self,
        method: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let (url, body, headers, rpc_id) = self.build_dsf_request(method, payload)?;
        let mut args = Vec::new();
        for (k, v) in headers {
            args.push(crate::Headers::Custom((k, v)));
        }
        let mut res = self.session.post_json(&url, body, args).await?;
        let body_data = res.body.data().await;
        let response_text = String::from_utf8_lossy(body_data).to_string();
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "DSF {method} failed {}: {}",
                res.http_code,
                response_text
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&response_text)?;
        if json["type"].as_str() != Some("server-response") {
            return Err(anyhow::anyhow!("DSF {method} returned invalid envelope"));
        }
        if json["rpcId"].as_str() != Some(rpc_id.as_str()) {
            return Err(anyhow::anyhow!(
                "DSF {method} rpcId mismatch: sent {}, got {}",
                rpc_id,
                json["rpcId"]
            ));
        }
        let result = &json["result"];
        if result["ok"].as_bool() == Some(true) {
            return Ok(result["value"].clone());
        }
        let error = &result["error"];
        let message = error["message"].as_str().unwrap_or("DSF business error");
        let code = error["code"].as_str().unwrap_or("unknown");
        Err(anyhow::anyhow!("DSF {method} failed: {code}: {message}"))
    }

    async fn dsf_respond(
        session: &mut Session,
        base_url: &str,
        api_key: &Option<String>,
        rpc_id: String,
        result: serde_json::Value,
    ) -> anyhow::Result<()> {
        let url = format!("{}/api/respond", base_url.trim_end_matches('/'));
        let mut args = vec![crate::Headers::Content_Type("application/json".to_string())];
        if let Some(key) = api_key {
            args.push(crate::Headers::Authorization(format!("Bearer {key}")));
        }
        let body = serde_json::json!({
            "type": "client-response",
            "rpcId": rpc_id,
            "result": result
        });
        let mut res = session.post_json(&url, body, args).await?;
        let body_data = res.body.data().await;
        if res.http_code != 200 {
            return Err(anyhow::anyhow!(
                "DSF respond failed {}: {}",
                res.http_code,
                String::from_utf8_lossy(body_data)
            ));
        }
        Ok(())
    }

    fn parse_dsf_model(&self) -> (String, String) {
        let model = self.model.as_deref().unwrap_or("deepseek-chat");
        if let Some((provider, model)) = model.split_once(':') {
            (provider.to_string(), model.to_string())
        } else {
            ("deepseek-official".to_string(), model.to_string())
        }
    }

    async fn ensure_dsf_session(&mut self) -> anyhow::Result<String> {
        if let Some(session_id) = self
            .provider_session
            .as_dsf()
            .and_then(|dsf| dsf.session_id.clone())
        {
            return Ok(session_id);
        }

        let mut payload = serde_json::json!({});
        if let Some(ref cwd) = self.working_directory {
            payload["cwd"] = serde_json::Value::String(cwd.clone());
        }
        let result = self.call_dsf("session.create", payload).await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("DSF session.create response missing sessionId"))?
            .to_string();
        let dsf = self
            .provider_session
            .as_dsf_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected DSF provider session"))?;
        dsf.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    async fn ensure_dsf_model(&mut self, session_id: &str) -> anyhow::Result<()> {
        let Some(model) = self.model.clone() else {
            return Ok(());
        };
        if self
            .provider_session
            .as_dsf()
            .and_then(|dsf| dsf.selected_model.as_deref())
            == Some(model.as_str())
        {
            return Ok(());
        }
        let (provider, model_id) = self.parse_dsf_model();
        let mut payload = serde_json::json!({
            "sessionId": session_id,
            "provider": provider,
            "model": model_id
        });
        if self.reasoning_effort != ReasoningEffort::Disabled {
            payload["reasoningEffort"] =
                serde_json::Value::String(self.reasoning_effort.as_str().to_string());
        }
        self.call_dsf("session.selectModel", payload).await?;
        let dsf = self
            .provider_session
            .as_dsf_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected DSF provider session"))?;
        dsf.selected_model = self.model.clone();
        Ok(())
    }

    async fn open_dsf_mux_ws(&mut self) -> anyhow::Result<crate::Websocket> {
        let mut headers = vec![];
        if let Some(ref api_key) = self.api_key {
            headers.push(crate::Headers::Authorization(format!("Bearer {api_key}")));
        }
        crate::Websocket::connect(&self.dsf_ws_url("events.mux"), headers).await
    }

    async fn send_dsf_prompt(&mut self, session_id: &str) -> anyhow::Result<()> {
        let content = self
            .messages
            .last()
            .ok_or_else(|| anyhow::anyhow!("No message to send"))?
            .content
            .clone();
        let result = self
            .call_dsf(
                "session.prompt",
                serde_json::json!({
                    "sessionId": session_id,
                    "mode": "queue",
                    "content": [
                        {
                            "type": "text",
                            "text": content
                        }
                    ]
                }),
            )
            .await?;
        if result["accepted"].as_bool() == Some(false) {
            return Err(anyhow::anyhow!("DSF session.prompt was not accepted"));
        }
        Ok(())
    }

    async fn prepare_dsf_turn(&mut self) -> anyhow::Result<(String, crate::Websocket)> {
        let session_id = self.ensure_dsf_session().await?;
        self.ensure_dsf_model(&session_id).await?;
        let ws = self.open_dsf_mux_ws().await?;
        self.send_dsf_prompt(&session_id).await?;
        Ok((session_id, ws))
    }

    async fn recv_dsf_ws_frame(ws: &mut crate::Websocket) -> anyhow::Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, ws.recv()).await {
                Ok(Ok(crate::WsFrame::Text(text))) => {
                    let val: serde_json::Value = serde_json::from_str(&text)?;
                    if val["type"].as_str() == Some("server-request") {
                        return Ok(val);
                    }
                }
                Ok(Ok(crate::WsFrame::Binary(_))) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(anyhow::anyhow!("Timeout waiting for DSF event")),
            }
        }
    }

    fn dsf_text_from_content_blocks(content: &serde_json::Value) -> String {
        let mut text = String::new();
        if let Some(blocks) = content.as_array() {
            for block in blocks {
                if block["type"].as_str() == Some("text") {
                    if let Some(part) = block["text"].as_str() {
                        text.push_str(part);
                    }
                }
            }
        }
        text
    }

    fn parse_dsf_server_request(
        envelope: &serde_json::Value,
        target_session_id: &str,
    ) -> AgentStreamAction {
        let rpc_id = envelope["rpcId"].as_str().unwrap_or("").to_string();
        let payload = &envelope["payload"];
        match payload["type"].as_str().unwrap_or("") {
            "session/event" => {
                if payload["sessionId"].as_str() != Some(target_session_id) {
                    return AgentStreamAction::Ignore;
                }
                let event = &payload["event"];
                match event["type"].as_str().unwrap_or("") {
                    "assistant/chunk" => {
                        let chunk = &event["data"]["chunk"];
                        if chunk["type"].as_str() == Some("text-delta") {
                            if let Some(text) = chunk["text"].as_str() {
                                return AgentStreamAction::Content(text.to_string());
                            }
                        }
                        AgentStreamAction::Ignore
                    }
                    "assistant/message" => {
                        let text = Self::dsf_text_from_content_blocks(
                            &event["data"]["message"]["content"],
                        );
                        if text.is_empty() {
                            AgentStreamAction::Ignore
                        } else {
                            AgentStreamAction::FinalContent(text)
                        }
                    }
                    "turn/end" => AgentStreamAction::Done,
                    _ => AgentStreamAction::Ignore,
                }
            }
            "approval/requested" => {
                let result = serde_json::json!({
                    "ok": true,
                    "value": {
                        "sessionId": payload["sessionId"],
                        "approvalId": payload["approvalId"],
                        "outcome": "rejected"
                    }
                });
                AgentStreamAction::DsfRespond { rpc_id, result }
            }
            "question/requested" => {
                let result = serde_json::json!({
                    "ok": false,
                    "error": {
                        "code": "cancelled",
                        "message": "potato AgentClientSession does not support interactive questions",
                        "details": {}
                    }
                });
                AgentStreamAction::DsfRespond { rpc_id, result }
            }
            "stream/error" => {
                let message = payload["error"]["message"]
                    .as_str()
                    .unwrap_or("DSF stream error")
                    .to_string();
                AgentStreamAction::Error(message)
            }
            _ => AgentStreamAction::Ignore,
        }
    }

    async fn chat_dsf(&mut self) -> anyhow::Result<String> {
        let (session_id, mut ws) = self.prepare_dsf_turn().await?;
        let mut assistant_text = String::new();
        let mut saw_delta = false;
        loop {
            let envelope = Self::recv_dsf_ws_frame(&mut ws).await?;
            match Self::parse_dsf_server_request(&envelope, &session_id) {
                AgentStreamAction::Content(text) => {
                    saw_delta = true;
                    assistant_text.push_str(&text);
                }
                AgentStreamAction::FinalContent(text) => {
                    if !saw_delta && assistant_text.is_empty() {
                        assistant_text.push_str(&text);
                    }
                }
                AgentStreamAction::DsfRespond { rpc_id, result } => {
                    Self::dsf_respond(
                        &mut self.session,
                        &self.base_url,
                        &self.api_key,
                        rpc_id,
                        result,
                    )
                    .await?;
                }
                AgentStreamAction::Done => break,
                AgentStreamAction::Error(message) => return Err(anyhow::anyhow!(message)),
                AgentStreamAction::Ignore => {}
            }
        }
        self.messages.push(ChatMessage::new(
            MessageRole::Assistant,
            assistant_text.clone(),
        ));
        Ok(assistant_text)
    }

    async fn chat_stream_dsf(
        &mut self,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let (session_id, mut ws) = self.prepare_dsf_turn().await?;
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(64);
        let messages =
            std::sync::Arc::new(std::sync::Mutex::new(std::mem::take(&mut self.messages)));

        tokio::spawn(async move {
            let mut respond_session = Session::new();
            let mut assistant_text = String::new();
            let mut saw_delta = false;
            loop {
                let envelope = match Self::recv_dsf_ws_frame(&mut ws).await {
                    Ok(envelope) => envelope,
                    Err(e) => {
                        let _ = tx
                            .send(StreamChunk::Content(format!(
                                "\n[DSF WebSocket error: {}]",
                                e
                            )))
                            .await;
                        let _ = tx.send(StreamChunk::Done).await;
                        return;
                    }
                };
                match Self::parse_dsf_server_request(&envelope, &session_id) {
                    AgentStreamAction::Content(text) => {
                        saw_delta = true;
                        assistant_text.push_str(&text);
                        if let Ok(mut msgs) = messages.lock() {
                            if msgs
                                .last()
                                .is_some_and(|m| m.role == MessageRole::Assistant)
                            {
                                if let Some(last) = msgs.last_mut() {
                                    last.content = assistant_text.clone();
                                }
                            } else {
                                msgs.push(ChatMessage::new(
                                    MessageRole::Assistant,
                                    assistant_text.clone(),
                                ));
                            }
                        }
                        if tx.send(StreamChunk::Content(text)).await.is_err() {
                            return;
                        }
                    }
                    AgentStreamAction::FinalContent(text) => {
                        if !saw_delta && assistant_text.is_empty() {
                            assistant_text.push_str(&text);
                            if let Ok(mut msgs) = messages.lock() {
                                msgs.push(ChatMessage::new(
                                    MessageRole::Assistant,
                                    assistant_text.clone(),
                                ));
                            }
                            if tx.send(StreamChunk::Content(text)).await.is_err() {
                                return;
                            }
                        }
                    }
                    AgentStreamAction::DsfRespond { rpc_id, result } => {
                        let _ = Self::dsf_respond(
                            &mut respond_session,
                            &base_url,
                            &api_key,
                            rpc_id,
                            result,
                        )
                        .await;
                    }
                    AgentStreamAction::Done => {
                        if let Ok(mut msgs) = messages.lock() {
                            if msgs.last().is_none_or(|m| m.role != MessageRole::Assistant) {
                                msgs.push(ChatMessage::new(
                                    MessageRole::Assistant,
                                    assistant_text.clone(),
                                ));
                            }
                        }
                        let _ = tx.send(StreamChunk::Done).await;
                        return;
                    }
                    AgentStreamAction::Error(message) => {
                        let _ = tx
                            .send(StreamChunk::Content(format!("\n[DSF error: {message}]")))
                            .await;
                        let _ = tx.send(StreamChunk::Done).await;
                        return;
                    }
                    AgentStreamAction::Ignore => {}
                }
            }
        });

        Ok(rx)
    }

    /// 发送消息并获取流式响应
    ///
    /// 流式响应过程中，助手回复会实时追加到历史记录的最后一项中，
    /// 每收到新数据即更新历史记录并转发给调用方。
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

        // Codex provider 使用 WebSocket JSON-RPC 协议，需要特殊处理
        if self.provider == LlmProvider::Codex {
            return self.chat_stream_codex().await;
        }
        if self.provider == LlmProvider::ZCode {
            return self.chat_stream_zcode().await;
        }
        if self.provider == LlmProvider::Dsf {
            return self.chat_stream_dsf().await;
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
        let messages =
            std::sync::Arc::new(std::sync::Mutex::new(std::mem::take(&mut self.messages)));

        // 启动后台任务解析流式响应
        tokio::spawn(async move {
            let mut stream = res.body.stream_data();
            let mut buffer = String::new();
            let mut assistant_content = String::new();
            while let Some(chunk) = stream.next().await {
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);
                let mut contents = Vec::new();
                match provider {
                    LlmProvider::OpenAI => {
                        while let Some(pos) = buffer.find("\n\n") {
                            let event = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();
                            if let Some(content) = Self::parse_openai_sse_chunk(&event) {
                                if !content.is_empty() {
                                    contents.push(content);
                                }
                            }
                        }
                    }
                    LlmProvider::Anthropic => {
                        while let Some(pos) = buffer.find("\n\n") {
                            let event = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();
                            if let Some(content) = Self::parse_anthropic_sse_chunk(&event) {
                                if !content.is_empty() {
                                    contents.push(content);
                                }
                            }
                        }
                    }
                    LlmProvider::Ollama => {
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].to_string();
                            buffer = buffer[pos + 1..].to_string();
                            if let Some(content) = Self::parse_ollama_ndjson_chunk(&line) {
                                if !content.is_empty() {
                                    contents.push(content);
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
                    LlmProvider::ZCode | LlmProvider::Dsf => {
                        // 不会走到这里，两个 provider 都使用各自的事件流协议
                    }
                }
                for content in contents {
                    assistant_content.push_str(&content);
                    // 更新历史记录最后一项（Assistant 消息）
                    if let Ok(mut msgs) = messages.lock() {
                        if msgs
                            .last()
                            .is_some_and(|m| m.role == MessageRole::Assistant)
                        {
                            if let Some(last) = msgs.last_mut() {
                                last.content = assistant_content.clone();
                            }
                        } else {
                            msgs.push(ChatMessage::new(
                                MessageRole::Assistant,
                                assistant_content.clone(),
                            ));
                        }
                    }
                    if tx.send(StreamChunk::Content(content)).await.is_err() {
                        return;
                    }
                }
            }
            // 流结束时确保历史记录中已存在 Assistant 消息
            if let Ok(mut msgs) = messages.lock() {
                if msgs.last().is_none_or(|m| m.role != MessageRole::Assistant) {
                    msgs.push(ChatMessage::new(MessageRole::Assistant, assistant_content));
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
        let (zcode_session_id, dsf_session_id, dsf_selected_model) = match &self.provider_session {
            ProviderSession::ZCode(arc) => {
                if let Ok(zcode) = arc.try_lock() {
                    (zcode.session_id.clone(), None, None)
                } else {
                    (None, None, None)
                }
            }
            ProviderSession::Dsf(dsf) => (None, dsf.session_id.clone(), dsf.selected_model.clone()),
            _ => (None, None, None),
        };
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
            "zcode_session_id": zcode_session_id,
            "dsf_session_id": dsf_session_id,
            "dsf_selected_model": dsf_selected_model,
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
        let zcode_session_id = state
            .get("zcode_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let dsf_session_id = state
            .get("dsf_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let dsf_selected_model = state
            .get("dsf_selected_model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut provider_session = ProviderSession::from_provider(&provider);
        if let ProviderSession::OpenCode(ref mut opencode) = provider_session {
            opencode.session_id = opencode_session_id;
            opencode.parent_id = opencode_parent_id;
        }
        // Codex 的 thread_id 在创建时设置（deserialize 是同步函数，无法 await lock）
        if provider == LlmProvider::Codex {
            let mut codex = CodexSession::new();
            codex.thread_id = codex_thread_id;
            provider_session =
                ProviderSession::Codex(std::sync::Arc::new(tokio::sync::Mutex::new(codex)));
        } else if provider == LlmProvider::ZCode {
            let mut zcode = ZCodeSession::new();
            zcode.session_id = zcode_session_id;
            provider_session =
                ProviderSession::ZCode(std::sync::Arc::new(tokio::sync::Mutex::new(zcode)));
        } else if provider == LlmProvider::Dsf {
            let mut dsf = DsfSession::new();
            dsf.session_id = dsf_session_id;
            dsf.selected_model = dsf_selected_model;
            provider_session = ProviderSession::Dsf(dsf);
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

    fn build_request(&self, stream: bool) -> anyhow::Result<AgentRequestParts> {
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
            LlmProvider::ZCode | LlmProvider::Dsf => Err(anyhow::anyhow!(
                "This provider uses a dedicated session protocol"
            )),
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
            LlmProvider::ZCode | LlmProvider::Dsf => Ok(String::new()),
        }
    }

    fn parse_openai_sse_chunk(event: &str) -> Option<String> {
        for line in event.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
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
            if let Some(data) = line.strip_prefix("data: ") {
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

#[cfg(test)]
mod tests {
    use super::{AgentClientSession, AgentStreamAction};

    #[test]
    fn zcode_event_parser_extracts_text_delta_and_done() {
        let delta = serde_json::json!({
            "method": "session/event",
            "params": {
                "sessionId": "sess_1",
                "seq": 1,
                "type": "model.streaming",
                "payload": {
                    "kind": "text_delta",
                    "delta": "hello"
                }
            }
        });
        assert_eq!(
            AgentClientSession::parse_zcode_event_action(&delta),
            AgentStreamAction::Content("hello".to_string())
        );

        let done = serde_json::json!({
            "method": "session/event",
            "params": {
                "sessionId": "sess_1",
                "seq": 2,
                "type": "turn.completed",
                "payload": {
                    "resultType": "success"
                }
            }
        });
        assert_eq!(
            AgentClientSession::parse_zcode_event_action(&done),
            AgentStreamAction::Done
        );
    }

    #[test]
    fn dsf_event_parser_extracts_chunk_and_final_message() {
        let chunk = serde_json::json!({
            "type": "server-request",
            "rpcId": "rpc-1",
            "method": "session/event",
            "payload": {
                "type": "session/event",
                "sessionId": "sess_1",
                "event": {
                    "type": "assistant/chunk",
                    "data": {
                        "chunk": {
                            "type": "text-delta",
                            "text": "hi"
                        }
                    }
                }
            }
        });
        assert_eq!(
            AgentClientSession::parse_dsf_server_request(&chunk, "sess_1"),
            AgentStreamAction::Content("hi".to_string())
        );

        let message = serde_json::json!({
            "type": "server-request",
            "rpcId": "rpc-2",
            "method": "session/event",
            "payload": {
                "type": "session/event",
                "sessionId": "sess_1",
                "event": {
                    "type": "assistant/message",
                    "data": {
                        "message": {
                            "content": [
                                { "type": "text", "text": "final" }
                            ]
                        }
                    }
                }
            }
        });
        assert_eq!(
            AgentClientSession::parse_dsf_server_request(&message, "sess_1"),
            AgentStreamAction::FinalContent("final".to_string())
        );
    }

    #[test]
    fn dsf_event_parser_builds_reject_approval_response() {
        let approval = serde_json::json!({
            "type": "server-request",
            "rpcId": "rpc-approval",
            "method": "approval/requested",
            "payload": {
                "type": "approval/requested",
                "sessionId": "sess_1",
                "approvalId": "approval_1",
                "toolName": "bash"
            }
        });
        assert_eq!(
            AgentClientSession::parse_dsf_server_request(&approval, "sess_1"),
            AgentStreamAction::DsfRespond {
                rpc_id: "rpc-approval".to_string(),
                result: serde_json::json!({
                    "ok": true,
                    "value": {
                        "sessionId": "sess_1",
                        "approvalId": "approval_1",
                        "outcome": "rejected"
                    }
                })
            }
        );
    }
}
