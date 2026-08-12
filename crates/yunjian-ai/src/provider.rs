//! AI 赏析供应商的公共边界与事实 grounding。

use async_trait::async_trait;
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::fmt;
use yunjian_core::operation::OperationHandle;
use yunjian_core::{Error, PoemDetail, Result};

/// 默认赏析模板文件名。
pub const APPRECIATION_TEMPLATE_FILE: &str = "appreciation.1.0.0.md";

/// 默认赏析模板的语义版本。
pub const APPRECIATION_TEMPLATE_VERSION: &str = "1.0.0";

/// 默认温度；固定为零以得到稳定、可缓存的输出。
pub const DEFAULT_TEMPERATURE: f32 = 0.0;

/// 编译期内嵌的默认赏析模板。
pub const APPRECIATION_TEMPLATE: PromptTemplate = PromptTemplate {
    name: "appreciation",
    file_name: APPRECIATION_TEMPLATE_FILE,
    version: APPRECIATION_TEMPLATE_VERSION,
    source: include_str!("../prompts/appreciation.1.0.0.md"),
};

/// 不依赖具体模型 SDK 的供应商标识。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    /// 校验并构造供应商标识。
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(Error::Config("AI 供应商标识不能为空".to_owned()));
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(Error::Config(format!(
                "AI 供应商标识只能包含 ASCII 字母、数字、点、连字符和下划线：{trimmed}"
            )));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// 返回稳定字符串标识。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 一个已注册、编译期内嵌的提示词模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTemplate {
    name: &'static str,
    file_name: &'static str,
    version: &'static str,
    source: &'static str,
}

impl PromptTemplate {
    /// 注册模板并验证文件名中的 semver 与显式版本一致。
    pub fn register(
        name: &'static str,
        file_name: &'static str,
        version: &'static str,
        source: &'static str,
    ) -> Result<Self> {
        validate_semver(version)?;
        let expected_file_name = format!("{name}.{version}.md");
        if file_name != expected_file_name {
            return Err(Error::Config(format!(
                "提示词模板文件名 {file_name} 与显式版本 {version} 不匹配；应为 {expected_file_name}"
            )));
        }
        if source.trim().is_empty() {
            return Err(Error::Config(format!("提示词模板 {file_name} 不能为空")));
        }
        Ok(Self {
            name,
            file_name,
            version,
            source,
        })
    }

    /// 返回模板用途名。
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// 返回携带 semver 的文件名。
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        self.file_name
    }

    /// 返回显式模板版本。
    #[must_use]
    pub const fn version(self) -> &'static str {
        self.version
    }

    /// 返回编译期内嵌的模板正文。
    #[must_use]
    pub const fn source(self) -> &'static str {
        self.source
    }
}

/// 一次赏析生成请求及其不可变事实 grounding。
#[derive(Debug, Clone)]
pub struct AppreciationRequest {
    poem: PoemDetail,
    model: String,
    style: Option<String>,
    temperature: f32,
    template: PromptTemplate,
    grounding: String,
    grounding_digest: String,
}

impl AppreciationRequest {
    /// 使用默认零温度与默认版本模板构造请求。
    #[must_use]
    pub fn new(poem: PoemDetail, model: impl Into<String>) -> Self {
        Self::with_template(poem, model, APPRECIATION_TEMPLATE)
    }

    /// 使用指定的已验证模板构造请求。
    #[must_use]
    pub fn with_template(
        poem: PoemDetail,
        model: impl Into<String>,
        template: PromptTemplate,
    ) -> Self {
        let grounding = render_grounding(&poem);
        let grounding_digest = blake3::hash(grounding.as_bytes()).to_hex().to_string();
        Self {
            poem,
            model: model.into(),
            style: None,
            temperature: DEFAULT_TEMPERATURE,
            template,
            grounding,
            grounding_digest,
        }
    }

    /// 增加面向模型的赏析风格约束。
    #[must_use]
    pub fn with_style(mut self, style: Option<String>) -> Self {
        self.style = style.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        self
    }

    /// 返回被赏析的完整作品详情。
    #[must_use]
    pub const fn poem(&self) -> &PoemDetail {
        &self.poem
    }

    /// 返回调用方选择的模型标识。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回可选的赏析风格约束。
    #[must_use]
    pub fn style(&self) -> Option<&str> {
        self.style.as_deref()
    }

    /// 返回确定性生成温度。
    #[must_use]
    pub const fn temperature(&self) -> f32 {
        self.temperature
    }

    /// 返回提示词模板版本。
    #[must_use]
    pub const fn template_version(&self) -> &'static str {
        self.template.version()
    }

    /// 返回由语料事实渲染的 grounding 块。
    #[must_use]
    pub fn grounding(&self) -> &str {
        &self.grounding
    }

    /// 返回 grounding 的 BLAKE3 摘要。
    #[must_use]
    pub fn grounding_digest(&self) -> &str {
        &self.grounding_digest
    }

    /// 将 grounding 填入版本化模板。
    #[must_use]
    pub fn render_prompt(&self) -> String {
        let prompt = self
            .template
            .source()
            .replace("{{grounding}}", self.grounding());
        self.style().map_or(prompt.clone(), |style| {
            format!("{prompt}\n\n输出风格要求：{style}")
        })
    }

    /// 构造包含模板版本与 grounding 摘要的稳定缓存键。
    #[must_use]
    pub fn cache_key(&self, provider: &ProviderId) -> String {
        let mut hasher = Hasher::new();
        for component in [
            provider.as_str(),
            self.model(),
            self.style().unwrap_or(""),
            self.template.name(),
            self.template_version(),
            self.grounding_digest(),
        ] {
            hasher.update(component.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(&self.temperature.to_bits().to_le_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

/// 已完成的 AI 赏析及生成溯源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appreciation {
    /// 生成的赏析正文。
    pub text: String,
    /// 实际使用的模型标识。
    pub model: String,
    /// 实际使用的供应商。
    pub provider: ProviderId,
    /// 生成完成时的 Unix 时间戳（秒）。
    pub generated_at: u64,
    /// 生成时使用的模板 semver。
    pub template_version: String,
    /// 生成时使用的 grounding 摘要。
    pub grounding_digest: String,
    /// 供应商返回的 token 用量；供应商未报告时为空。
    pub usage: Option<TokenUsage>,
}

/// 一次生成由供应商报告的标准化 token 用量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 输入 prompt 消耗的 token 数。
    pub input_tokens: u32,
    /// 输出 completion 消耗的 token 数。
    pub output_tokens: u32,
    /// 供应商报告或由输入输出相加得到的 token 总数。
    pub total_tokens: u32,
}

/// 一次不带持久化能力的诗词生成请求。
#[derive(Debug, Clone)]
pub struct PoemGenerationRequest {
    prompt: String,
    model: String,
    temperature: f32,
}

impl PoemGenerationRequest {
    /// 构造诗词生成请求。
    #[must_use]
    pub fn new(prompt: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            model: model.into(),
            temperature: 0.7,
        }
    }

    /// 覆盖采样温度。
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature.clamp(0.0, 2.0);
        self
    }

    /// 返回提交给模型的完整提示词。
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// 返回模型标识。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回采样温度。
    #[must_use]
    pub const fn temperature(&self) -> f32 {
        self.temperature
    }
}

/// 模型生成的诗词文本及调用溯源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedPoem {
    /// 模型返回的文本。
    pub text: String,
    /// 实际使用的模型标识。
    pub model: String,
    /// 实际使用的供应商。
    pub provider: ProviderId,
    /// 生成完成时的 Unix 时间戳（秒）。
    pub generated_at: u64,
    /// 供应商返回的 token 用量；供应商未报告时为空。
    pub usage: Option<TokenUsage>,
}

/// 流式赏析的可合并进度快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppreciationProgress {
    /// 已生成的 Unicode 字符数。
    pub generated_chars: usize,
}

/// 流式赏析中不可丢弃的增量结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AppreciationStreamItem {
    /// 一段增量文本。
    Chunk(String),
    /// 带完整生成溯源的最终赏析。
    Complete(Appreciation),
}

/// 隔离具体模型 SDK 的赏析供应商边界。
#[async_trait]
pub trait AppreciationProvider: Send + Sync {
    /// 生成完整赏析。
    async fn appreciate(&self, request: AppreciationRequest) -> Result<Appreciation>;

    /// 启动流式赏析并返回全工作区统一的长任务句柄。
    async fn appreciate_stream(
        &self,
        request: AppreciationRequest,
    ) -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>>;

    /// 返回稳定供应商标识。
    fn id(&self) -> ProviderId;
}

/// 不提供任何写入入口的诗词生成供应商边界。
#[async_trait]
pub trait PoemGenerationProvider: Send + Sync {
    /// 生成一首诗词；调用方负责验证格式与格律。
    async fn generate_poem(&self, request: PoemGenerationRequest) -> Result<GeneratedPoem>;
}

/// 同时支持赏析与作诗的 AI 供应商。
pub trait AiProvider: AppreciationProvider + PoemGenerationProvider {}

impl<T> AiProvider for T where T: AppreciationProvider + PoemGenerationProvider {}

/// 未配置密钥时仍可安装的空供应商。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NullProvider {
    provider: ProviderId,
}

impl NullProvider {
    /// 为指定供应商构造空实现。
    pub fn new(provider: impl Into<String>) -> Result<Self> {
        Ok(Self {
            provider: ProviderId::new(provider)?,
        })
    }

    fn no_key(&self) -> Error {
        Error::AiKeyNotConfigured {
            provider: self.provider.to_string(),
        }
    }
}

#[async_trait]
impl AppreciationProvider for NullProvider {
    async fn appreciate(&self, _request: AppreciationRequest) -> Result<Appreciation> {
        Err(self.no_key())
    }

    async fn appreciate_stream(
        &self,
        _request: AppreciationRequest,
    ) -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        Err(self.no_key())
    }

    fn id(&self) -> ProviderId {
        self.provider.clone()
    }
}

#[async_trait]
impl PoemGenerationProvider for NullProvider {
    async fn generate_poem(&self, _request: PoemGenerationRequest) -> Result<GeneratedPoem> {
        Err(self.no_key())
    }
}

fn validate_semver(version: &str) -> Result<()> {
    let components = version.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(Error::Config(format!(
            "提示词模板版本必须是 major.minor.patch 数字格式：{version}"
        )));
    }
    Ok(())
}

fn render_grounding(detail: &PoemDetail) -> String {
    let poem = &detail.poem;
    let mut lines = vec![
        format!("作品：{}", poem.title),
        format!("作者：{}", poem.author),
        format!(
            "朝代：{}（上游原串：{}）",
            poem.dynasty.canonical, poem.dynasty.raw
        ),
        format!("词牌：{}", poem.ci_tune.as_deref().unwrap_or("无（诗）")),
        format!("正文：{}", poem.body),
        format!(
            "平仄（{}）：{}",
            detail.tones.book.display_name(),
            detail.tones.display()
        ),
    ];

    if detail.rhyme_groups.is_empty() {
        lines.push("韵部：语料未给出".to_owned());
    } else {
        lines.extend(detail.rhyme_groups.iter().map(|membership| {
            format!(
                "韵部：{} {} {}（可信度：{}）",
                membership.book.display_name(),
                membership.group,
                membership.tone.display_name(),
                membership.confidence.display_name()
            )
        }));
    }

    lines.push("历代集评（公有领域，保留出处）：".to_owned());
    if detail.commentaries.is_empty() {
        lines.push("- 无".to_owned());
    } else {
        lines.extend(detail.commentaries.iter().map(|entry| {
            let citation = &entry.citation;
            format!(
                "- {}——{}《{}》，{}（{}，成书不晚于 {} 年；{}）",
                entry.text,
                citation.author,
                citation.work.trim_matches(['《', '》']),
                citation.dynasty.canonical,
                citation.dynasty.raw,
                citation.work_completed_by,
                citation.source_note
            )
        }));
    }
    lines.join("\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        APPRECIATION_TEMPLATE, APPRECIATION_TEMPLATE_FILE, APPRECIATION_TEMPLATE_VERSION,
        Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
        AppreciationStreamItem, NullProvider, PromptTemplate, ProviderId,
    };
    use async_trait::async_trait;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    use yunjian_core::operation::{Event, OperationHandle, next_event, start_operation};
    use yunjian_core::{
        AuthorRecord, CommentaryCitation, CommentaryEntry, DynastyLabel, Error, PoemDetail,
        PoemRecord, Provenance, RhymeBook, RhymeConfidence, RhymeGroupMembership, RhymeTone, Tone,
        ToneAnnotation, ToneCell, ToneLine,
    };

    pub(crate) fn fixture_detail() -> PoemDetail {
        let dynasty = DynastyLabel {
            canonical: "宋".to_owned(),
            raw: "北宋".to_owned(),
        };
        PoemDetail {
            poem: PoemRecord {
                stable_id: "poem-fixture".to_owned(),
                content_hash: "content-fixture".to_owned(),
                title: "念奴娇·赤壁怀古".to_owned(),
                title_raw: "念奴娇 赤壁怀古".to_owned(),
                ci_tune: Some("念奴娇".to_owned()),
                author: "苏轼".to_owned(),
                dynasty: dynasty.clone(),
                genre: "ci".to_owned(),
                body: "大江东去，浪淘尽，千古风流人物。".to_owned(),
                body_original: "大江東去，浪淘盡，千古風流人物。".to_owned(),
                script: "traditional".to_owned(),
                first_line: "大江东去".to_owned(),
                last_chars: vec!["去".to_owned(), "物".to_owned()],
                line_count: 2,
                char_count: 14,
                work_group: "work-fixture".to_owned(),
                edition_group: "edition-fixture".to_owned(),
            },
            author: AuthorRecord {
                name: "苏轼".to_owned(),
                dynasties: vec![dynasty.clone()],
                poem_count: 1,
            },
            tones: ToneAnnotation {
                book: RhymeBook::Cilin,
                lines: vec![ToneLine {
                    line_index: 0,
                    text: "大江东去".to_owned(),
                    cells: vec![
                        ToneCell {
                            character: "大".to_owned(),
                            tone: Tone::Oblique,
                            readings: vec!["departing".to_owned()],
                        },
                        ToneCell {
                            character: "江".to_owned(),
                            tone: Tone::Level,
                            readings: vec!["level".to_owned()],
                        },
                    ],
                }],
                unknown_count: 0,
                either_count: 0,
            },
            rhyme_groups: vec![RhymeGroupMembership {
                book: RhymeBook::Cilin,
                group: "第十八部".to_owned(),
                tone: RhymeTone::Entering,
                confidence: RhymeConfidence::Unambiguous,
            }],
            work_group_siblings: Vec::new(),
            attribution_conflict: None,
            provenance: Provenance {
                source_locator: "fixture:1".to_owned(),
                source_locator_kind: "native".to_owned(),
                source: "fixture".to_owned(),
                revision: "test".to_owned(),
                kind: "原文".to_owned(),
                license: "Public Domain".to_owned(),
                license_class: "public_domain".to_owned(),
            },
            tags: vec!["豪放".to_owned()],
            commentaries: vec![CommentaryEntry {
                id: "commentary-fixture".to_owned(),
                text: "自有横槊气概，固是英雄本色。".to_owned(),
                citation: CommentaryCitation {
                    work: "《古今词论》".to_owned(),
                    author: "王又华".to_owned(),
                    dynasty: DynastyLabel {
                        canonical: "清".to_owned(),
                        raw: "清".to_owned(),
                    },
                    work_completed_by: 1680,
                    source_note: "卷一，通行本".to_owned(),
                },
            }],
        }
    }

    fn ready<T>(future: impl Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("fixture future should be ready on first poll"),
        }
    }

    #[test]
    fn provider_request_defaults_to_deterministic_versioned_template() {
        let request = AppreciationRequest::new(fixture_detail(), "fixture-model");
        assert_eq!(request.temperature(), 0.0);
        assert_eq!(request.template_version(), APPRECIATION_TEMPLATE_VERSION);
        assert_eq!(
            APPRECIATION_TEMPLATE.file_name(),
            APPRECIATION_TEMPLATE_FILE
        );
        assert!(!request.template_version().is_empty());
        assert!(
            PromptTemplate::register(
                "appreciation",
                APPRECIATION_TEMPLATE_FILE,
                APPRECIATION_TEMPLATE_VERSION,
                APPRECIATION_TEMPLATE.source(),
            )
            .is_ok()
        );
        assert!(
            PromptTemplate::register(
                "appreciation",
                "appreciation.1.0.1.md",
                APPRECIATION_TEMPLATE_VERSION,
                APPRECIATION_TEMPLATE.source(),
            )
            .is_err(),
            "文件名 semver 漂移时必须被检测"
        );
    }

    #[test]
    fn provider_grounding_contains_known_facts_and_preserved_citation() {
        let request = AppreciationRequest::new(fixture_detail(), "fixture-model");
        let grounding = request.grounding();
        for fact in [
            "作者：苏轼",
            "朝代：宋（上游原串：北宋）",
            "词牌：念奴娇",
            "平仄（词林正韵）：仄平",
            "韵部：词林正韵 第十八部 入",
            "自有横槊气概，固是英雄本色。",
            "《古今词论》",
            "王又华",
            "卷一，通行本",
        ] {
            assert!(grounding.contains(fact), "grounding 缺少事实：{fact}");
        }
        let prompt = request.render_prompt();
        assert!(prompt.contains("不得断言 grounding 中没有的典故或归属"));
        assert!(prompt.contains(grounding));
        assert_eq!(request.grounding_digest().len(), 64);
    }

    #[test]
    fn provider_cache_key_includes_template_version() {
        let detail = fixture_detail();
        let current = AppreciationRequest::new(detail.clone(), "fixture-model");
        let next_template = PromptTemplate::register(
            "appreciation",
            "appreciation.1.0.1.md",
            "1.0.1",
            APPRECIATION_TEMPLATE.source(),
        )
        .expect("matching version");
        let next = AppreciationRequest::with_template(detail, "fixture-model", next_template);
        let provider = ProviderId::new("fixture").expect("valid provider id");
        assert_ne!(current.cache_key(&provider), next.cache_key(&provider));
    }

    #[test]
    fn provider_null_provider_returns_typed_no_key_outcome() {
        let provider = NullProvider::new("deepseek").expect("valid provider id");
        let error =
            ready(provider.appreciate(AppreciationRequest::new(fixture_detail(), "deepseek-chat")))
                .expect_err("NullProvider must not generate text");
        match error {
            Error::AiKeyNotConfigured { provider } => assert_eq!(provider, "deepseek"),
            other => panic!("expected typed no-key outcome, got {other:?}"),
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FixtureProvider;

    #[async_trait]
    impl AppreciationProvider for FixtureProvider {
        async fn appreciate(
            &self,
            request: AppreciationRequest,
        ) -> yunjian_core::Result<Appreciation> {
            Ok(Appreciation {
                text: "fixture".to_owned(),
                model: request.model().to_owned(),
                provider: self.id(),
                generated_at: 1,
                template_version: request.template_version().to_owned(),
                grounding_digest: request.grounding_digest().to_owned(),
                usage: None,
            })
        }

        async fn appreciate_stream(
            &self,
            request: AppreciationRequest,
        ) -> yunjian_core::Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>>
        {
            let result = self.appreciate(request).await?;
            Ok(start_operation(move |reporter| {
                reporter.item(AppreciationStreamItem::Complete(result));
                Ok(())
            }))
        }

        fn id(&self) -> ProviderId {
            ProviderId::new("fixture").expect("valid provider id")
        }
    }

    fn assert_core_handle(_: &OperationHandle<AppreciationProgress, AppreciationStreamItem>) {}

    #[test]
    fn provider_stream_uses_core_operation_handle() {
        let provider = FixtureProvider;
        let handle = ready(
            provider.appreciate_stream(AppreciationRequest::new(fixture_detail(), "fixture-model")),
        )
        .expect("stream starts");
        assert_core_handle(&handle);
        assert!(matches!(
            next_event(&handle, 200),
            Some(Event::Item(AppreciationStreamItem::Complete(Appreciation {
                text,
                ..
            }))) if text == "fixture"
        ));
        assert_eq!(next_event(&handle, 200), Some(Event::Done));

        let source = include_str!("provider.rs");
        assert!(!source.contains(concat!("pub struct Operation", "Handle")));
        assert!(!source.contains(concat!("pub enum Operation", "Handle")));
        assert!(!source.contains(concat!("genai", "::")));
    }
}
