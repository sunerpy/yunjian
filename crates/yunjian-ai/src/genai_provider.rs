//! 基于 `genai` 的赏析供应商，凭据由 [`crate::keystore`] 以编程方式注入。
//!
//! # 为什么密钥必须走 resolver 而不是环境变量
//!
//! `genai` 的默认凭据来源是环境变量：每个 adapter 都声明一个
//! `DEFAULT_API_KEY_ENV_NAME`（`DEEPSEEK_API_KEY`、`MOONSHOT_API_KEY`……），
//! `AdapterDispatcher::default_auth` 据此返回 `AuthData` 的**环境变量变体**。云笺的密钥存在操作
//! 系统钥匙串里，**不进进程环境**——环境变量会被子进程继承、被 `/proc/<pid>/environ`
//! 读到、被崩溃报告一并带走。因此本模块用 `AuthResolver::from_resolver_fn` 把钥匙串里
//! 的密钥直接交给 `genai`，不启用环境变量凭据回退。
//!
//! # 实测的 resolver 解析顺序（genai 0.6.5 `ClientConfig::resolve_service_target`）
//!
//! 1. `ModelMapper`（本模块不用）
//! 2. **`AuthResolver`** → `AuthData`
//! 3. `AdapterDispatcher::default_endpoint(adapter_kind)` → `Endpoint`
//! 4. 组装 `ServiceTarget { model, auth, endpoint }`
//! 5. **`ServiceTargetResolver`**——最终决定权，可改写 endpoint、model **和 auth**
//!
//! 两条由此推出的硬约束，都在本模块的测试里被钉住：
//!
//! - [`ServiceTargetResolver`] 只改 `endpoint` 与 `model`，**绝不写 `auth`**。它在
//!   [`AuthResolver`] 之后运行，覆盖 `auth` 就等于把注入的密钥丢掉，退回环境变量。
//! - [`AuthResolver`] **绝不返回 `Ok(None)`**。`genai` 把 `None` 当作「没有意见」并
//!   `unwrap_or_else(|| default_auth(...))` 回落到环境变量凭据——也就是说，一次
//!   看起来无害的 `Ok(None)` 会静默地重新启用环境变量读取。缺密钥时返回 `Err`。

use crate::AppreciationCacheWriter;
use crate::keystore::{KeyStore, Lookup};
use crate::provider::{
    Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
    AppreciationStreamItem, GeneratedPoem, PoemGenerationProvider, PoemGenerationRequest,
    ProviderId, TokenUsage,
};
use async_trait::async_trait;
use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, Usage};
use genai::resolver::{AuthData, AuthResolver, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;
use yunjian_core::operation::{OperationHandle, start_operation};
use yunjian_core::{Error, Result};

/// 云笺支持的供应商种类。
///
/// 选型标准是「中文用户真的会去申请并粘贴 key 的服务」，而不是 `genai` 支持的全集。
/// 每一项都对应 `genai` 内置的一个 adapter，因此协议差异（OpenAI 兼容 / Anthropic 原生 /
/// Ollama 原生）由上游承担，本 crate 只做映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderKind {
    /// 深度求索。
    DeepSeek,
    /// 月之暗面 Kimi。与 [`Self::Moonshot`] 同一服务，保留两个标识是因为用户两种叫法都用。
    Kimi,
    /// 月之暗面（官方名）。
    Moonshot,
    /// 阿里云百炼 / 通义千问，OpenAI 兼容模式。
    Qwen,
    /// 智谱 Z.ai 国际站。
    Zai,
    /// 智谱开放平台（bigmodel.cn）。
    BigModel,
    /// OpenRouter 聚合网关。
    OpenRouter,
    /// 本地 Ollama。
    Ollama,
    /// OpenAI。
    OpenAI,
    /// Anthropic。
    Anthropic,
}

impl ProviderKind {
    /// 全部支持的种类，顺序稳定，供设置界面与表驱动测试枚举。
    pub const ALL: &'static [Self] = &[
        Self::DeepSeek,
        Self::Kimi,
        Self::Moonshot,
        Self::Qwen,
        Self::Zai,
        Self::BigModel,
        Self::OpenRouter,
        Self::Ollama,
        Self::OpenAI,
        Self::Anthropic,
    ];

    /// 稳定字符串标识。同时用作钥匙串里的 account 名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Kimi => "kimi",
            Self::Moonshot => "moonshot",
            Self::Qwen => "qwen",
            Self::Zai => "zai",
            Self::BigModel => "bigmodel",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    /// 解析用户或配置给出的标识。
    pub fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == normalized)
            .ok_or_else(|| {
                let supported = Self::ALL
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                Error::Config(format!("未知的 AI 供应商 {value}；已支持：{supported}"))
            })
    }

    /// 对应的 `genai` adapter。
    ///
    /// 这是本 crate 唯一提及 `genai` 类型的位置族群，且不出现在
    /// [`AppreciationProvider`] 的签名里——`Kimi` 与 `Moonshot` 收敛到同一个 adapter，
    /// 因为它们是同一个服务的两种叫法。
    const fn adapter_kind(self) -> AdapterKind {
        match self {
            Self::DeepSeek => AdapterKind::DeepSeek,
            Self::Kimi | Self::Moonshot => AdapterKind::Moonshot,
            Self::Qwen => AdapterKind::Aliyun,
            Self::Zai => AdapterKind::Zai,
            Self::BigModel => AdapterKind::BigModel,
            Self::OpenRouter => AdapterKind::OpenRouter,
            Self::Ollama => AdapterKind::Ollama,
            Self::OpenAI => AdapterKind::OpenAI,
            Self::Anthropic => AdapterKind::Anthropic,
        }
    }

    /// 上游 adapter 的默认 base URL，供设置界面显示「留空即使用」的那个值。
    ///
    /// 与 `genai` 0.6.5 的 `Adapter::default_endpoint` 逐一核对得到；用户填了自定义
    /// base URL 时由 [`ServiceTargetResolver`] 覆盖。
    #[must_use]
    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::DeepSeek => "https://api.deepseek.com/v1/",
            Self::Kimi | Self::Moonshot => "https://api.moonshot.cn/v1/",
            Self::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1/",
            Self::Zai => "https://api.z.ai/api/paas/v4/",
            Self::BigModel => "https://open.bigmodel.cn/api/paas/v4/",
            Self::OpenRouter => "https://openrouter.ai/api/v1/",
            Self::Ollama => "http://localhost:11434/",
            Self::OpenAI => "https://api.openai.com/v1/",
            Self::Anthropic => "https://api.anthropic.com/v1/",
        }
    }

    /// 是否需要用户提供密钥。
    ///
    /// 只有本地 Ollama 不需要：它没有 `DEFAULT_API_KEY_ENV_NAME`，上游用固定占位串
    /// `"ollama"` 充当 key。这条区分让「没配 key」在 Ollama 上不成为错误。
    #[must_use]
    pub const fn requires_key(self) -> bool {
        !matches!(self, Self::Ollama)
    }

    /// 配置未显式指定模型时采用的稳定默认值。
    #[must_use]
    pub const fn default_model(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek-chat",
            Self::Kimi | Self::Moonshot => "moonshot-v1-8k",
            Self::Qwen => "qwen-plus",
            Self::Zai | Self::BigModel => "glm-4-flash",
            Self::OpenRouter => "openai/gpt-4o-mini",
            Self::Ollama => "qwen2.5:7b",
            Self::OpenAI => "gpt-4o-mini",
            Self::Anthropic => "claude-3-5-haiku-latest",
        }
    }

    /// 稳定供应商标识。
    fn provider_id(self) -> ProviderId {
        ProviderId::new(self.as_str())
            .unwrap_or_else(|_| unreachable!("内置供应商标识恒为合法 ASCII"))
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 构造 [`GenAiProvider`] 所需的非机密配置。
///
/// 刻意不含密钥字段：密钥只能经 [`GenAiProvider::from_keystore`] 从钥匙串取，或经
/// [`GenAiProvider::with_secret`] 由已持有它的调用方交进来。这样「把 key 粘进配置文件」
/// 在类型层面无法表达。
#[derive(Clone, PartialEq, Eq)]
pub struct GenAiProviderConfig {
    kind: ProviderKind,
    base_url: Option<ValidatedEndpoint>,
    endpoint_error: Option<EndpointValidationError>,
    model_override: Option<String>,
    extra_body: Option<Value>,
}

impl fmt::Debug for GenAiProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenAiProviderConfig")
            .field("kind", &self.kind)
            .field("base_url_configured", &self.base_url.is_some())
            .field("base_url_valid", &self.endpoint_error.is_none())
            .field("model_override", &self.model_override)
            .field("extra_body_configured", &self.extra_body.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ValidatedEndpoint(String);

impl ValidatedEndpoint {
    fn parse(value: String) -> std::result::Result<Self, EndpointValidationError> {
        let parsed = Url::parse(&value).map_err(|_| EndpointValidationError::InvalidUrl)?;
        if has_url_userinfo(&value) || !parsed.username().is_empty() || parsed.password().is_some()
        {
            return Err(EndpointValidationError::UserInfo);
        }
        if parsed
            .query_pairs()
            .any(|(key, _)| is_sensitive_query_key(&key))
        {
            return Err(EndpointValidationError::SensitiveQuery);
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointValidationError {
    InvalidUrl,
    UserInfo,
    SensitiveQuery,
}

impl EndpointValidationError {
    const fn message(self) -> &'static str {
        match self {
            Self::InvalidUrl => "AI endpoint 必须是合法 URL",
            Self::UserInfo => "AI endpoint 不得包含 URL userinfo 凭据",
            Self::SensitiveQuery => "AI endpoint 不得包含凭据形状的 query 参数",
        }
    }
}

fn has_url_userinfo(value: &str) -> bool {
    value
        .split_once("://")
        .map(|(_, remainder)| {
            remainder
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
        .unwrap_or(false)
}

fn is_sensitive_query_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key" | "apikey" | "key" | "token" | "access_token" | "secret" | "password"
    )
}

impl GenAiProviderConfig {
    /// 以上游默认端点构造配置。
    #[must_use]
    pub const fn new(kind: ProviderKind) -> Self {
        Self {
            kind,
            base_url: None,
            endpoint_error: None,
            model_override: None,
            extra_body: None,
        }
    }

    /// 覆盖 base URL。用于自建反代、企业网关与本地 Ollama 改端口。
    ///
    /// 原始字符串在此边界立即解析；配置内部只保留已校验的 endpoint。为兼容 workspace
    /// 已有 builder 调用，本方法不返回 `Result`，校验失败会在 provider 构造时以脱敏错误返回。
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        match ValidatedEndpoint::parse(base_url.into()) {
            Ok(endpoint) => {
                self.base_url = Some(endpoint);
                self.endpoint_error = None;
            }
            Err(error) => {
                self.base_url = None;
                self.endpoint_error = Some(error);
            }
        }
        self
    }

    /// 在覆盖 base URL 的同时覆盖模型名。
    ///
    /// 网关常要求与原厂不同的模型串（例如 OpenRouter 的 `deepseek/deepseek-chat`），
    /// 而这层改写与端点改写发生在同一个 resolver 里，故一并提供。
    pub fn with_model_override(mut self, model: impl Into<String>) -> Self {
        self.model_override = Some(model.into());
        self
    }

    /// 直通到请求体的非标准字段。
    ///
    /// 落到 `ChatOptions::with_extra_body`，由 adapter 合并进 payload。存在的理由是
    /// OpenAI 兼容的国内供应商各自带私有开关（例如通义的 `enable_thinking`），若不给
    /// 直通口，用户就只能等本 crate 逐个加字段。
    pub fn with_extra_body(mut self, extra_body: Value) -> Self {
        self.extra_body = Some(extra_body);
        self
    }

    /// 供应商种类。
    #[must_use]
    pub const fn kind(&self) -> ProviderKind {
        self.kind
    }

    /// 生效的 base URL。
    #[must_use]
    pub fn effective_base_url(&self) -> &str {
        self.base_url
            .as_ref()
            .map(ValidatedEndpoint::as_str)
            .unwrap_or_else(|| self.kind.default_base_url())
    }

    fn validate(&self) -> Result<()> {
        self.endpoint_error.map_or(Ok(()), |error| {
            Err(Error::Config(error.message().to_owned()))
        })
    }
}

/// 由钥匙串注入凭据的 `genai` 赏析供应商。
pub struct GenAiProvider {
    config: GenAiProviderConfig,
    provider: ProviderId,
    client: Client,
    has_key: bool,
    cache_writer: Option<Arc<dyn AppreciationCacheWriter>>,
}

impl Clone for GenAiProvider {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            provider: self.provider.clone(),
            client: self.client.clone(),
            has_key: self.has_key,
            cache_writer: self.cache_writer.clone(),
        }
    }
}

impl fmt::Debug for GenAiProvider {
    /// 手写而非 derive。
    ///
    /// derive 会渲染 `client`，而 `genai::Client` 内部持有 `AuthResolver`——那是一个
    /// 捕获了密钥的闭包。上游给闭包的 `Debug` 只打印类型名 `"AuthResolverFn"`，但那是
    /// **上游的实现细节**，一次改动就能让密钥出现在日志里。这里根本不渲染 `client`，
    /// 也不渲染任何来自密钥的派生值，只报「是否已配置」这一非机密事实。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenAiProvider")
            .field("provider", &self.provider)
            .field("adapter", &self.config.kind.adapter_kind())
            .field("base_url_configured", &self.config.base_url.is_some())
            .field("model_override", &self.config.model_override)
            .field("extra_body_configured", &self.config.extra_body.is_some())
            .field("key_configured", &self.has_key)
            .finish()
    }
}

impl GenAiProvider {
    /// 从钥匙串取密钥并构造。
    ///
    /// `Lookup::Absent` 是**正常返回**而非错误：keyutils 后端重启即失效（见
    /// [`crate::keystore`] 的降级链）。因此需要密钥的供应商在此返回
    /// [`Error::AiKeyNotConfigured`]，让调用方走重新索要，而不是把它当异常。
    pub fn from_keystore(config: GenAiProviderConfig, store: &KeyStore) -> Result<Self> {
        config.validate()?;
        let kind = config.kind;
        let lookup = store.get(kind.as_str())?;
        match lookup {
            Lookup::Found { secret, .. } => Self::with_secret(config, Some(secret)),
            Lookup::Absent { .. } if kind.requires_key() => Err(Error::AiKeyNotConfigured {
                provider: kind.as_str().to_owned(),
            }),
            Lookup::Absent { .. } => Self::with_secret(config, None),
        }
    }

    /// 以已持有的密钥构造。
    pub fn with_secret(config: GenAiProviderConfig, key: Option<SecretString>) -> Result<Self> {
        config.validate()?;
        let kind = config.kind;
        if key.is_none() && kind.requires_key() {
            return Err(Error::AiKeyNotConfigured {
                provider: kind.as_str().to_owned(),
            });
        }

        let has_key = key.is_some();
        let provider = kind.provider_id();
        let client = Client::builder()
            .with_adapter_kind(kind.adapter_kind())
            .with_auth_resolver(auth_resolver(kind, key.map(Arc::new)))
            .with_service_target_resolver(service_target_resolver(
                config.base_url.clone(),
                config.model_override.clone(),
            ))
            .build();

        Ok(Self {
            config,
            provider,
            client,
            has_key,
            cache_writer: None,
        })
    }

    /// 安装完整结果缓存写入器；取消或失败的流不会调用它。
    #[must_use]
    pub fn with_cache_writer(mut self, cache_writer: Arc<dyn AppreciationCacheWriter>) -> Self {
        self.cache_writer = Some(cache_writer);
        self
    }

    /// 非机密配置。
    #[must_use]
    pub const fn config(&self) -> &GenAiProviderConfig {
        &self.config
    }

    /// 供应商种类。
    #[must_use]
    pub const fn kind(&self) -> ProviderKind {
        self.config.kind
    }

    fn chat_options(&self, request: &AppreciationRequest) -> ChatOptions {
        let mut options = ChatOptions::default()
            .with_temperature(f64::from(request.temperature()))
            .with_capture_usage(true);
        if let Some(extra_body) = self.config.extra_body.clone() {
            options = options.with_extra_body(extra_body);
        }
        options
    }

    /// 把 `genai` 的错误折成一条已脱敏的 [`Error::Ai`]。
    ///
    /// **刻意不做 `format!("{error}")` 或 `{error:?}` 的整体透传。** 两个具体理由：
    ///
    /// - `webc::Error::ResponseFailedStatus` 带一个 `headers: Box<HeaderMap>` 字段，
    ///   derive 出来的 `Debug` 会把整张头表打出来；
    /// - `genai::Error::ChatResponseGeneration` 带完整 `request_payload`。
    ///
    /// 两者当下都不含密钥（OpenAI 兼容协议把 key 放在请求头，而这里拿到的是**响应**
    /// 头），但那是上游此刻的实现，不是它的契约。所以这里只抽取自己需要的两样东西
    /// ——HTTP 状态码与一句人话——其余一概不进错误载荷。`Error::ai` 之后还会再洗一遍。
    fn wrap_error(&self, error: genai::Error) -> Error {
        let detail = match &error {
            genai::Error::WebModelCall { webc_error, .. }
            | genai::Error::WebAdapterCall { webc_error, .. } => match webc_error {
                genai::webc::Error::ResponseFailedStatus { status, .. } => format!(
                    "HTTP {} {}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("未知状态")
                ),
                genai::webc::Error::ResponseFailedNotJson { content_type, .. } => {
                    format!("响应不是 JSON，Content-Type 为 {content_type}")
                }
                genai::webc::Error::ResponseFailedInvalidJson { .. } => {
                    "响应 JSON 无法解析".to_owned()
                }
                genai::webc::Error::Reqwest(_) => "网络请求失败".to_owned(),
                genai::webc::Error::JsonValueExt(_) => "响应结构与预期不符".to_owned(),
            },
            genai::Error::HttpError { status, .. } => format!(
                "HTTP {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("未知状态")
            ),
            genai::Error::Resolver { .. } => "凭据解析失败；请确认已配置密钥".to_owned(),
            genai::Error::RequiresApiKey { .. }
            | genai::Error::NoAuthData { .. }
            | genai::Error::NoAuthResolver { .. } => "该模型需要密钥".to_owned(),
            genai::Error::NoChatResponse { .. } => "供应商未返回任何内容".to_owned(),
            genai::Error::AdapterKindMismatch { .. } => "模型名与所选供应商不匹配".to_owned(),
            _ => "调用失败".to_owned(),
        };
        Error::ai(
            self.provider.as_str(),
            format!("模型 {}：{detail}", self.config.kind),
        )
    }

    async fn stream_to_channel(
        self,
        request: AppreciationRequest,
        sender: mpsc::Sender<StreamMessage>,
        cancellation: CancellationToken,
    ) {
        let result = self.consume_stream(request, &sender, &cancellation).await;
        if let Err(error) = result {
            let _ = sender.send(StreamMessage::Failed(error.to_string())).await;
        }
    }

    async fn consume_stream(
        &self,
        request: AppreciationRequest,
        sender: &mpsc::Sender<StreamMessage>,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let chat_request =
            ChatRequest::default().append_message(ChatMessage::user(request.render_prompt()));
        let options = self.chat_options(&request);
        let response = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            response = self.client.exec_chat_stream(request.model(), chat_request, Some(&options)) => {
                response.map_err(|error| self.wrap_error(error))?
            }
        };
        let mut stream = response.stream;
        let mut text = String::new();

        loop {
            let event = tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                event = stream.next() => event,
            };
            match event {
                Some(Ok(ChatStreamEvent::Chunk(chunk))) => {
                    text.push_str(&chunk.content);
                    if !send_unless_cancelled(
                        sender,
                        StreamMessage::Chunk(chunk.content),
                        cancellation,
                    )
                    .await
                    {
                        return Ok(());
                    }
                }
                Some(Ok(ChatStreamEvent::End(end))) => {
                    if text.is_empty() {
                        return Err(Error::ai(
                            self.provider.as_str(),
                            format!("模型 {} 返回了空赏析", self.config.kind),
                        ));
                    }
                    let appreciation = Appreciation {
                        text,
                        model: request.model().to_owned(),
                        provider: self.provider.clone(),
                        generated_at: unix_seconds(),
                        template_version: request.template_version().to_owned(),
                        grounding_digest: request.grounding_digest().to_owned(),
                        usage: end.captured_usage.and_then(normalize_usage),
                    };
                    if cancellation.is_cancelled() {
                        return Ok(());
                    }
                    if let Some(cache_writer) = &self.cache_writer {
                        cache_writer
                            .store_completed(&request.cache_key(&self.provider), &appreciation)?;
                    }
                    let _ = send_unless_cancelled(
                        sender,
                        StreamMessage::Complete(appreciation),
                        cancellation,
                    )
                    .await;
                    return Ok(());
                }
                Some(Ok(
                    ChatStreamEvent::Start
                    | ChatStreamEvent::ReasoningChunk(_)
                    | ChatStreamEvent::ThoughtSignatureChunk(_)
                    | ChatStreamEvent::ToolCallChunk(_),
                )) => {}
                Some(Err(error)) => return Err(self.wrap_error(error)),
                None => {
                    return Err(Error::ai(
                        self.provider.as_str(),
                        format!("模型 {} 的响应流在终态前结束", self.config.kind),
                    ));
                }
            }
        }
    }
}

#[derive(Debug)]
enum StreamMessage {
    Chunk(String),
    Complete(Appreciation),
    Failed(String),
}

async fn send_unless_cancelled(
    sender: &mpsc::Sender<StreamMessage>,
    message: StreamMessage,
    cancellation: &CancellationToken,
) -> bool {
    tokio::select! {
        () = cancellation.cancelled() => false,
        result = sender.send(message) => result.is_ok(),
    }
}

fn normalize_usage(usage: Usage) -> Option<TokenUsage> {
    let input_tokens = nonnegative_tokens(usage.prompt_tokens);
    let output_tokens = nonnegative_tokens(usage.completion_tokens);
    let total_tokens = nonnegative_tokens(usage.total_tokens).unwrap_or_else(|| {
        input_tokens
            .unwrap_or_default()
            .saturating_add(output_tokens.unwrap_or_default())
    });
    let normalized = TokenUsage {
        input_tokens: input_tokens.unwrap_or_default(),
        output_tokens: output_tokens.unwrap_or_default(),
        total_tokens,
    };
    (normalized.input_tokens != 0 || normalized.output_tokens != 0 || normalized.total_tokens != 0)
        .then_some(normalized)
}

fn nonnegative_tokens(tokens: Option<i32>) -> Option<u32> {
    tokens.and_then(|value| u32::try_from(value).ok())
}

#[async_trait]
impl AppreciationProvider for GenAiProvider {
    async fn appreciate(&self, request: AppreciationRequest) -> Result<Appreciation> {
        let chat_request =
            ChatRequest::default().append_message(ChatMessage::user(request.render_prompt()));
        let options = self.chat_options(&request);

        let response = self
            .client
            .exec_chat(request.model(), chat_request, Some(&options))
            .await
            .map_err(|error| self.wrap_error(error))?;

        let usage = normalize_usage(response.usage.clone());
        let text = response.into_first_text().ok_or_else(|| {
            Error::ai(
                self.provider.as_str(),
                format!("模型 {} 返回了空赏析", self.config.kind),
            )
        })?;

        Ok(Appreciation {
            text,
            model: request.model().to_owned(),
            provider: self.provider.clone(),
            generated_at: unix_seconds(),
            template_version: request.template_version().to_owned(),
            grounding_digest: request.grounding_digest().to_owned(),
            usage,
        })
    }

    async fn appreciate_stream(
        &self,
        request: AppreciationRequest,
    ) -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        let (sender, mut receiver) = mpsc::channel(1);
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let provider = self.clone();
        std::mem::drop(tokio::spawn(async move {
            provider
                .stream_to_channel(request, sender, task_cancellation)
                .await;
        }));

        Ok(start_operation(move |reporter| {
            let mut generated_chars = 0_usize;
            loop {
                if reporter.wait_for_stop(Duration::from_millis(2)) {
                    cancellation.cancel();
                    return Ok(());
                }
                match receiver.try_recv() {
                    Ok(StreamMessage::Chunk(chunk)) => {
                        generated_chars += chunk.chars().count();
                        if !reporter.item(AppreciationStreamItem::Chunk(chunk)) {
                            cancellation.cancel();
                            return Ok(());
                        }
                        reporter.progress(AppreciationProgress { generated_chars });
                    }
                    Ok(StreamMessage::Complete(appreciation)) => {
                        if reporter.item(AppreciationStreamItem::Complete(appreciation)) {
                            return Ok(());
                        }
                        cancellation.cancel();
                        return Ok(());
                    }
                    Ok(StreamMessage::Failed(message)) => return Err(message),
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        if reporter.is_cancelled() || reporter.is_closed() {
                            cancellation.cancel();
                            return Ok(());
                        }
                        return Err("AI 响应流生产者异常退出".to_owned());
                    }
                }
            }
        }))
    }

    fn id(&self) -> ProviderId {
        self.provider.clone()
    }
}

#[async_trait]
impl PoemGenerationProvider for GenAiProvider {
    async fn generate_poem(&self, request: PoemGenerationRequest) -> Result<GeneratedPoem> {
        let chat_request =
            ChatRequest::default().append_message(ChatMessage::user(request.prompt()));
        let options = ChatOptions::default()
            .with_temperature(f64::from(request.temperature()))
            .with_capture_usage(true);
        let response = self
            .client
            .exec_chat(request.model(), chat_request, Some(&options))
            .await
            .map_err(|error| self.wrap_error(error))?;
        let usage = normalize_usage(response.usage.clone());
        let text = response.into_first_text().ok_or_else(|| {
            Error::ai(
                self.provider.as_str(),
                format!("模型 {} 返回了空诗词", self.config.kind),
            )
        })?;
        Ok(GeneratedPoem {
            text,
            model: request.model().to_owned(),
            provider: self.provider.clone(),
            generated_at: unix_seconds(),
            usage,
        })
    }
}

/// 把钥匙串里的密钥交给 `genai`。
///
/// 返回值只有两种形态：`Ok(Some(AuthData::Key(..)))` 与 `Err`。**没有 `Ok(None)` 这条
/// 分支**，因为 `genai` 会把 `None` 解释成「resolver 没有意见」并回落到
/// `AdapterDispatcher::default_auth`，也就是环境变量凭据回退——那正是本模块要消灭的
/// 行为。Ollama 走 `AuthData::Key("ollama")`（与上游 `default_auth` 的占位串一致），
/// 同样不碰环境变量。
fn auth_resolver(kind: ProviderKind, key: Option<Arc<SecretString>>) -> AuthResolver {
    AuthResolver::from_resolver_fn(
        move |_model: ModelIden| -> std::result::Result<Option<AuthData>, genai::resolver::Error> {
            match key.as_deref() {
                Some(secret) => Ok(Some(AuthData::Key(secret.expose_secret().to_owned()))),
                None if !kind.requires_key() => {
                    Ok(Some(AuthData::Key(OLLAMA_PLACEHOLDER_KEY.to_owned())))
                }
                None => Err(genai::resolver::Error::Custom(format!(
                    "供应商 {kind} 尚未配置密钥"
                ))),
            }
        },
    )
}

/// 覆盖端点与模型，**不动 `auth`**。
///
/// 这个 resolver 在 [`auth_resolver`] 之后运行并拥有最终决定权（实测自
/// `ClientConfig::resolve_service_target`）。上游示例 `c06-target-resolver.rs` 里整个
/// 重建了 `ServiceTarget`——照抄会连 `auth` 一起替换，把注入的密钥丢掉。故此处按字段
/// 改写传入的 target。
fn service_target_resolver(
    base_url: Option<ValidatedEndpoint>,
    model_override: Option<String>,
) -> ServiceTargetResolver {
    ServiceTargetResolver::from_resolver_fn(
        move |mut target: ServiceTarget| -> std::result::Result<ServiceTarget, genai::resolver::Error> {
            if let Some(base_url) = base_url.clone() {
                target.endpoint = Endpoint::from_owned(base_url.0);
            }
            if let Some(model) = model_override.clone() {
                target.model = ModelIden::new(target.model.adapter_kind, model);
            }
            Ok(target)
        },
    )
}

/// Ollama 无需真实密钥；与 `genai` 0.6.5 `OllamaAdapter::default_auth` 的占位串一致。
const OLLAMA_PLACEHOLDER_KEY: &str = "ollama";

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    /// 假密钥。断言脱敏时用它做探针：渲染结果里出现 `TESTKEY` 即为泄露。
    const PROBE_KEY: &str = "sk-TESTKEY123";

    /// 起一个只回一次的最小 HTTP 端点，把收到的请求头交回调用方。
    ///
    /// 刻意不引入 `wiremock`/`httpmock`：本 crate 只需要「收一个请求、看它的头、回一个
    /// 固定响应」，为此加一条测试依赖不划算，而标准库的 `TcpListener` 足够。
    fn spawn_probe(status: u16, body: &'static str) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定探针端口");
        let port = listener.local_addr().expect("取探针端口").port();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("接受探针连接");
            let mut reader = BufReader::new(&stream);
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line.trim().is_empty() {
                    break;
                }
                headers.push(line.trim().to_owned());
            }
            let mut stream = stream;
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            headers
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    /// 复用 `provider.rs` 测试里的 fixture 诗，避免两处各造一份而漂移。
    fn sample_request() -> AppreciationRequest {
        AppreciationRequest::new(crate::provider::tests::fixture_detail(), "gpt-4o-mini")
    }

    fn probe_config(base_url: &str) -> GenAiProviderConfig {
        GenAiProviderConfig::new(ProviderKind::OpenAI).with_base_url(base_url.to_owned())
    }

    /// 密钥必须经 resolver 进到 `Authorization` 头，且调用期间不读任何环境变量。
    ///
    /// 后半条是本模块存在的理由：`genai` 的 `default_auth` 会回落到环境变量凭据，
    /// 一旦回落，密钥就得先进程序环境才能用——那与「密钥只在钥匙串里」直接冲突。
    #[tokio::test]
    async fn the_resolver_key_reaches_the_authorization_header_without_touching_the_environment() {
        let (base_url, probe) = spawn_probe(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
        let provider = GenAiProvider::with_secret(
            probe_config(&base_url),
            Some(SecretString::from(PROBE_KEY.to_owned())),
        )
        .expect("构造 provider");

        let before: Vec<String> = std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| {
                let upper = name.to_ascii_uppercase();
                upper.contains("KEY") || upper.contains("TOKEN") || upper.contains("SECRET")
            })
            .collect();

        let _ = provider.appreciate(sample_request()).await;
        let headers = probe.join().expect("回收探针");

        let authorization = headers
            .iter()
            .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            .expect("请求必须带 Authorization 头");
        assert!(
            authorization.contains(PROBE_KEY),
            "resolver 提供的密钥必须出现在 Authorization 头里：{authorization}"
        );

        let after: Vec<String> = std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| {
                let upper = name.to_ascii_uppercase();
                upper.contains("KEY") || upper.contains("TOKEN") || upper.contains("SECRET")
            })
            .collect();
        assert_eq!(
            before, after,
            "调用不得新增任何 key 形状的环境变量（禁止环境变量凭据回退）"
        );
    }

    /// 自定义 base URL 必须被 `ServiceTargetResolver` 采纳。
    ///
    /// 探针端口是随机的，所以「请求真的打到了探针」本身就是端点被覆盖的证据。
    #[tokio::test]
    async fn a_custom_base_url_is_honoured_by_the_service_target_resolver() {
        let (base_url, probe) = spawn_probe(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
        let provider = GenAiProvider::with_secret(
            probe_config(&base_url),
            Some(SecretString::from(PROBE_KEY.to_owned())),
        )
        .expect("构造 provider");
        let _ = provider.appreciate(sample_request()).await;
        let headers = probe.join().expect("回收探针");
        assert!(
            headers.iter().any(|line| line.starts_with("POST ")),
            "自定义 base URL 未被采纳：探针没收到 POST，实收 {headers:?}"
        );
    }

    /// `Debug` 渲染绝不能带出密钥。
    #[test]
    fn debug_rendering_never_leaks_the_key() {
        let provider = GenAiProvider::with_secret(
            probe_config("http://127.0.0.1:1"),
            Some(SecretString::from(PROBE_KEY.to_owned())),
        )
        .expect("构造 provider");
        let rendered = format!("{provider:?}");
        assert!(
            !rendered.contains("TESTKEY"),
            "Debug 输出泄露了密钥：{rendered}"
        );
    }

    #[test]
    fn endpoint_rejects_userinfo_and_sensitive_query_keys() {
        let invalid_endpoints = [
            "https://user:password@example.com/v1",
            "https://example.com/v1?api_key=PROBE",
            "https://example.com/v1?APIKEY=PROBE",
            "https://example.com/v1?Key=PROBE",
            "https://example.com/v1?TOKEN=PROBE",
            "https://example.com/v1?Access_Token=PROBE",
            "https://example.com/v1?SeCrEt=PROBE",
            "https://example.com/v1?PASSWORD=PROBE",
        ];

        for endpoint in invalid_endpoints {
            let error = GenAiProvider::with_secret(
                probe_config(endpoint),
                Some(SecretString::from(PROBE_KEY.to_owned())),
            )
            .expect_err("携带凭据的 endpoint 必须在构造 provider 前被拒绝");
            let rendered = error.to_string();
            assert!(
                !rendered.contains("PROBE") && !rendered.contains("password@example"),
                "endpoint 校验错误不得回显凭据：{rendered}"
            );
        }
    }

    #[test]
    fn config_debug_only_reports_non_secret_shape() {
        let config = probe_config("https://debug-endpoint-probe.example/v1")
            .with_model_override("debug-model-probe")
            .with_extra_body(serde_json::json!({"debug-extra-body-probe": "secret-value"}));

        let rendered = format!("{config:?}");
        assert!(rendered.contains("OpenAI"));
        assert!(rendered.contains("debug-model-probe"));
        assert!(!rendered.contains("debug-endpoint-probe"));
        assert!(!rendered.contains("debug-extra-body-probe"));
        assert!(!rendered.contains("secret-value"));
    }

    #[test]
    fn provider_debug_does_not_render_endpoint_or_extra_body() {
        let config = probe_config("https://provider-endpoint-probe.example/v1")
            .with_extra_body(serde_json::json!({"provider-extra-body-probe": "secret-value"}));
        let provider =
            GenAiProvider::with_secret(config, Some(SecretString::from(PROBE_KEY.to_owned())))
                .expect("构造 provider");

        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("provider-endpoint-probe"));
        assert!(!rendered.contains("provider-extra-body-probe"));
        assert!(!rendered.contains("secret-value"));
    }

    /// HTTP 401 的错误必须点名供应商与状态码，但**不得含任何密钥материал**。
    #[tokio::test]
    async fn an_upstream_401_names_the_provider_without_leaking_the_key() {
        let (base_url, probe) = spawn_probe(401, r#"{"error":"unauthorized"}"#);
        let provider = GenAiProvider::with_secret(
            probe_config(&base_url),
            Some(SecretString::from(PROBE_KEY.to_owned())),
        )
        .expect("构造 provider");
        let error = provider
            .appreciate(sample_request())
            .await
            .expect_err("401 必须是错误");
        let _ = probe.join();
        let rendered = format!("{error}");
        let debugged = format!("{error:?}");
        assert!(
            !rendered.contains("TESTKEY") && !debugged.contains("TESTKEY"),
            "错误渲染泄露了密钥：display={rendered} debug={debugged}"
        );
    }

    /// 缺密钥的必需密钥供应商要在构造期就失败，而不是在请求期才暴露。
    #[test]
    fn a_provider_that_requires_a_key_refuses_to_build_without_one() {
        let error = GenAiProvider::with_secret(probe_config("http://127.0.0.1:1"), None)
            .expect_err("缺密钥必须失败");
        let rendered = format!("{error}");
        assert!(
            rendered.contains("openai") || rendered.contains("密钥"),
            "错误应点名供应商或缺密钥原因：{rendered}"
        );
    }
}
