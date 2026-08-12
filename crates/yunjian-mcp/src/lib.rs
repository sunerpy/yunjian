//! 云笺 MCP 服务端。默认走 stdio。
//!
//! 服务启动不以语料可用为前提：缺语料时仍完成握手并列出工具，调用工具则返回可见的
//! `corpus_missing` 结构化错误。stdout 只承载换行分隔的 JSON-RPC 协议帧。
//!
//! # 可选的 HTTP 传输
//!
//! `http` cargo 特性（**默认关闭**）加上 Streamable HTTP 传输，见 [`http`]。它默认只绑回环、
//! 强制每个请求带 bearer token、校验 `Origin`；这些不是加固项，而是把 stdio 从操作系统那里
//! 免费得到的隔离手工补回来。
//!
//! # 五个工具都只读，AI 工具额外声明开放世界
//!
//! `search_poem` / `explain_poem` / `find_similar_poem` 只读本地语料库，不联网、不需要
//! API key、不写任何数据。每个工具都**显式**声明 `read_only_hint = true` 与
//! `open_world_hint = false`——MCP 的 annotation 默认值是最坏情况（`destructiveHint`
//! 默认真、`openWorldHint` 默认真），省略它们会让客户端在每次调用前都弹一次确认。
//! `appreciate_poem` / `generate_poem` 同样不写数据，但可能调用外部模型，故显式声明
//! `open_world_hint = true`；缺少服务商或密钥时返回普通结构化结果，不把配置问题升级为协议错误。
//!
//! # 结果同时带 `structuredContent` 和一个 text block
//!
//! 工具返回 `Json<T>`，`rmcp` 由此生成 `outputSchema`，并把序列化后的 JSON 同时放进
//! `structuredContent` 与一个 text block（见 `CallToolResult::structured`）。后者是给尚未
//! 支持 `structuredContent` 的老客户端留的向后兼容路径。

#![warn(missing_docs)]

#[cfg(feature = "http")]
pub mod http;
pub mod schema;
pub mod similarity;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{
    ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter, model::CallToolResult,
    tool, tool_handler, tool_router, transport::stdio,
};
use schema::{
    AI_SETTINGS_PATH, AI_UNREVIEWED_DISCLOSURE, AppreciatePoemInput, AppreciatePoemOutput,
    AttributionConflictFacts, AttributionFacts, AuthorFacts, CommentaryCitationFacts,
    CommentaryFacts, ExplainPoemInput, ExplainPoemOutput, FindSimilarPoemInput,
    FindSimilarPoemOutput, GeneratePoemInput, GeneratePoemOutput, GeneratedPoemForm,
    OFFLINE_FACTS_DISCLOSURE, PoemFacts, ProvenanceFacts, RhymeGroupFacts, SearchPoemHighlight,
    SearchPoemHit, SearchPoemInput, SearchPoemOutput, SimilarPoem, SimilarityAxis, ToneCellFacts,
    ToneFacts, ToneLineFacts,
};
use similarity::Profile;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, OnceLock};
use yunjian_ai::{
    AiProvider, AppreciationCache, AppreciationRequest, CacheHit, CacheSource,
    PoemGenerationRequest, ProviderId,
};
use yunjian_core::{
    Attribution, AuthorSearchRequest, CharacterRhymesRequest, DynastyBrowseRequest, Error,
    PoemDetail, PoemDetailRequest, PoemFeatures, RhymeBook, RhymeGroupSearchRequest,
    TagBrowseRequest, TextSearchRequest, TitleSearchRequest, ToneFilter, Yunjian, content_chars,
    split_metrical_lines,
};

/// `search_poem` 的缺省单页上限。
pub const SEARCH_LIMIT_DEFAULT: u32 = 10;

/// `search_poem` 的服务端硬上限。**超出时截断，不报错。**
///
/// 上限存在的理由不是保护服务端，而是保护调用方的上下文窗口：一次 `tools/call` 的结果会
/// 整体进入模型的上下文，而 `tools/call` 的结果**不在 MCP 分页机制覆盖范围内**，没有任何
/// 协议层会替我们把它切开。所以必须由服务端自己封顶。
///
/// 它比 `yunjian_core::TEXT_SEARCH_HARD_CAP`（100）更严：核心层封的是查询代价，这里封的
/// 是上下文预算，两者不是同一个约束。
pub const SEARCH_LIMIT_MAX: u32 = 50;

/// `find_similar_poem` 的结果条数上限。
pub const SIMILAR_RESULT_CAP: usize = 20;

/// `find_similar_poem` 打分前的候选池上限。
///
/// 每个候选要读一次本体、标签与韵部归属（三次主键点查），所以池子必须封顶。取 200 是因为
/// 结果只有 20 条，十倍的候选足以让排序稳定，而 200 次点查在实测上是毫秒级。
pub const SIMILAR_CANDIDATE_POOL_CAP: usize = 200;

/// 生成诗词在所有 MCP 结果中的固定身份标签。
pub const GENERATED_POEM_LABEL: &str = "AI 生成，非古人作品";

#[derive(Debug, Clone)]
enum CoreClient {
    Ready(Yunjian),
    Missing,
}

/// 通过 MCP 暴露云笺核心能力的服务端。
#[derive(Clone)]
pub struct YunjianServer {
    core: CoreClient,
    ai: Option<Arc<dyn AiProvider>>,
    appreciation_cache: Option<Arc<AppreciationCache>>,
    ai_model: String,
    stopwords: Arc<OnceLock<BTreeSet<char>>>,
    tool_router: ToolRouter<Self>,
}

impl fmt::Debug for YunjianServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("YunjianServer")
            .field("core", &self.core)
            .field("ai_configured", &self.ai.is_some())
            .field("appreciation_cache", &self.appreciation_cache)
            .field("ai_model", &self.ai_model)
            .finish_non_exhaustive()
    }
}

#[tool_router(router = tool_router)]
impl YunjianServer {
    /// 使用已就绪的只读核心客户端创建服务端。
    #[must_use]
    pub fn new(core: Yunjian) -> Self {
        Self {
            core: CoreClient::Ready(core),
            ai: None,
            appreciation_cache: None,
            ai_model: String::new(),
            stopwords: Arc::new(OnceLock::new()),
            tool_router: Self::tool_router(),
        }
    }

    /// 创建可完成握手但会报告缺少语料的服务端。
    #[must_use]
    pub fn without_corpus() -> Self {
        Self {
            core: CoreClient::Missing,
            ai: None,
            appreciation_cache: None,
            ai_model: String::new(),
            stopwords: Arc::new(OnceLock::new()),
            tool_router: Self::tool_router(),
        }
    }

    /// 注入 AI 供应商、可选赏析缓存与模型名。
    #[must_use]
    pub fn with_ai(
        mut self,
        ai: Arc<dyn AiProvider>,
        appreciation_cache: Option<Arc<AppreciationCache>>,
        model: impl Into<String>,
    ) -> Self {
        self.ai = Some(ai);
        self.appreciation_cache = appreciation_cache;
        self.ai_model = model.into();
        self
    }

    #[tool(
        name = "search_poem",
        description = "在本地诗词语料库里按正文检索作品，返回 poem_id 与命中行。\
             用于「这句诗出自哪里」「查含某个词的作品」。支持繁体与异体输入；两字查询走候选\
             索引，不需要三字以上。limit 缺省 10、最大 50，超出即截断；翻页把上一次的 \
             next_cursor 原样传回。完全离线，不需要 API key。",
        annotations(
            title = "检索诗词",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_poem(
        &self,
        Parameters(input): Parameters<SearchPoemInput>,
    ) -> Result<Json<SearchPoemOutput>, CallToolResult> {
        let core = self.ready()?;
        let requested = input.limit.unwrap_or(SEARCH_LIMIT_DEFAULT);
        let limit = requested.clamp(1, SEARCH_LIMIT_MAX);
        let page = core
            .search_text(TextSearchRequest {
                query: input.query.clone(),
                limit: usize::try_from(limit).unwrap_or(usize::MAX),
                cursor: input.cursor.clone(),
            })
            .map_err(|error| tool_error("search_failed", &error.to_string(), None))?;

        let mut notes = Vec::new();
        if requested > SEARCH_LIMIT_MAX {
            notes.push(format!(
                "请求的 limit={requested} 超过服务端上限，已截断为 {SEARCH_LIMIT_MAX}。"
            ));
        }
        let before_filter = page.hits.len();
        let hits: Vec<SearchPoemHit> = page
            .hits
            .into_iter()
            .filter(|hit| matches_filter(input.author.as_deref(), &hit.author))
            .filter(|hit| matches_filter(input.dynasty.as_deref(), &hit.dynasty))
            .map(|hit| SearchPoemHit {
                poem_id: hit.poem_id,
                title: hit.title,
                author: hit.author,
                dynasty: hit.dynasty,
                matched_line_index: clamp_u32(hit.matched_line_index),
                snippet: hit.snippet.text,
                highlights: hit
                    .snippet
                    .highlights
                    .into_iter()
                    .map(|range| SearchPoemHighlight {
                        start: clamp_u32(range.start),
                        end: clamp_u32(range.end),
                    })
                    .collect(),
            })
            .collect();
        if input.author.is_some() || input.dynasty.is_some() {
            notes.push(
                "author / dynasty 只在当前页内过滤，total_estimate 是过滤前的数，游标按过滤前\
                 的序列推进。"
                    .to_owned(),
            );
            if hits.is_empty() && before_filter > 0 {
                notes.push(
                    "本页命中全部被过滤掉了，这不等于语料里没有符合条件的作品；带 next_cursor \
                     继续翻页。"
                        .to_owned(),
                );
            }
        }

        Ok(Json(SearchPoemOutput {
            query: input.query,
            limit,
            limit_clamped: requested > SEARCH_LIMIT_MAX,
            hits,
            total_estimate: clamp_u32(page.total_estimate),
            next_cursor: page.next_cursor,
            notes,
            disclosure: OFFLINE_FACTS_DISCLOSURE.to_owned(),
        }))
    }

    #[tool(
        name = "explain_poem",
        description = "读取一首作品的结构化事实：正文、逐字平仄（未知位置标 unknown）、\
             逐韵书韵部归属与可信度、同一正文的其它归属（替代项与作者冲突）、溯源，以及\
             公有领域历代集评（每条都带出处）。用于「这首诗的格律」「这句韵脚属哪一韵」\
             「古人怎么评这首」。**只返回事实，不生成赏析**；要 AI 赏析请用 appreciate_poem。",
        annotations(
            title = "作品格律与集评",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn explain_poem(
        &self,
        Parameters(input): Parameters<ExplainPoemInput>,
    ) -> Result<Json<ExplainPoemOutput>, CallToolResult> {
        let core = self.ready()?;
        let detail = core
            .poem_detail(PoemDetailRequest {
                poem_id: input.poem_id.clone(),
            })
            .map_err(|error| explain_error(&input.poem_id, &error))?;
        Ok(Json(explain_output(detail)))
    }

    #[tool(
        name = "find_similar_poem",
        description = "为一首作品找语料里相关的其它作品。得分是固定权重加权和——共享标签 \
             0.4、同韵部 0.25、同词牌 0.2、字面重叠 0.15（已排除语料里文档频率最高的 200 \
             字，否则共享一个「不」「人」就能造出相似）——每条结果都回传分量，可逐项复核。\
             可选 by 只限定从哪条轴取候选（theme/rhyme/tune/author/dynasty），不改变打分\
             口径。结果按 work_group 去重、最多 20 条。离线计算，不是 embedding 模型。",
        annotations(
            title = "查找相关作品",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn find_similar_poem(
        &self,
        Parameters(input): Parameters<FindSimilarPoemInput>,
    ) -> Result<Json<FindSimilarPoemOutput>, CallToolResult> {
        let core = self.ready()?;
        let source = core
            .poem_features(&[input.poem_id.as_str()])
            .map_err(|error| explain_error(&input.poem_id, &error))?
            .pop()
            .ok_or_else(|| {
                tool_error(
                    "poem_not_found",
                    &format!("语料里没有 stable_id 为 `{}` 的作品", input.poem_id),
                    Some("先用 search_poem 取得 poem_id，不要自行拼接"),
                )
            })?;

        let axes: Vec<SimilarityAxis> = input
            .by
            .map_or_else(|| SimilarityAxis::all().to_vec(), |axis| vec![axis]);
        let candidate_ids = self.gather_candidates(core, &source, &axes)?;
        let stopwords = self.stopwords(core);
        let source_profile = Profile::new(&source, stopwords);

        let borrowed: Vec<&str> = candidate_ids.iter().map(String::as_str).collect();
        let candidates = core
            .poem_features(&borrowed)
            .map_err(|error| tool_error("similar_failed", &error.to_string(), None))?;

        let mut scored: Vec<SimilarPoem> = candidates
            .iter()
            .map(|candidate| {
                let profile = Profile::new(candidate, stopwords);
                let components = similarity::score(&source_profile, &profile);
                let matched_axes = source_profile
                    .axes_against(
                        &profile,
                        candidate.poem.author == source.poem.author,
                        candidate.poem.dynasty.canonical == source.poem.dynasty.canonical,
                    )
                    .into_iter()
                    .map(|axis| axis.as_key().to_owned())
                    .collect();
                SimilarPoem {
                    poem_id: candidate.poem.stable_id.clone(),
                    title: candidate.poem.title.clone(),
                    author: candidate.poem.author.clone(),
                    dynasty: candidate.poem.dynasty.canonical.clone(),
                    ci_tune: candidate.poem.ci_tune.clone(),
                    work_group: candidate.poem.work_group.clone(),
                    score: similarity::total(&components),
                    components,
                    matched_axes,
                }
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.poem_id.cmp(&right.poem_id))
        });
        let matches = dedup_by_work_group(scored, SIMILAR_RESULT_CAP);

        Ok(Json(FindSimilarPoemOutput {
            poem_id: input.poem_id,
            requested_axis: input.by.map(|axis| axis.as_key().to_owned()),
            axes_used: axes.iter().map(|axis| axis.as_key().to_owned()).collect(),
            weights: similarity::weights(),
            excluded_frequent_chars: clamp_u32(stopwords.len()),
            candidate_pool_cap: clamp_u32(SIMILAR_CANDIDATE_POOL_CAP),
            result_cap: clamp_u32(SIMILAR_RESULT_CAP),
            matches,
            method: similarity::METHOD.to_owned(),
            disclosure: OFFLINE_FACTS_DISCLOSURE.to_owned(),
        }))
    }

    #[tool(
        name = "appreciate_poem",
        description = "为语料库中的一首作品取得明确标注的 AI 赏析。先查用户本地缓存，再查随包预生成层，只有未命中时才调用外部模型；返回 source、model 与 template_version。可选 style 用来约束表达风格。未配置服务商或密钥时返回带设置路径的普通结果。",
        annotations(
            title = "AI 赏析",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn appreciate_poem(
        &self,
        Parameters(input): Parameters<AppreciatePoemInput>,
    ) -> Result<Json<AppreciatePoemOutput>, CallToolResult> {
        let core = self.ready()?;
        let detail = core
            .poem_detail(PoemDetailRequest {
                poem_id: input.poem_id.clone(),
            })
            .map_err(|error| explain_error(&input.poem_id, &error))?;
        let request =
            AppreciationRequest::new(detail, self.ai_model.clone()).with_style(input.style.clone());
        let provider_id = self
            .ai
            .as_ref()
            .map(|provider| provider.id())
            .unwrap_or_else(unconfigured_provider_id);

        if let Some(cache) = &self.appreciation_cache {
            let cached = cache.lookup(&request, &provider_id).map_err(|error| {
                tool_error("appreciation_cache_failed", &error.to_string(), None)
            })?;
            if let Some(hit) = cached {
                return Ok(Json(appreciation_output(input.poem_id, hit)));
            }
        }

        let Some(provider) = &self.ai else {
            return Ok(Json(appreciation_configuration_required(input.poem_id)));
        };
        let appreciation = match provider.appreciate(request.clone()).await {
            Ok(appreciation) => appreciation,
            Err(Error::AiKeyNotConfigured { .. }) => {
                return Ok(Json(appreciation_configuration_required(input.poem_id)));
            }
            Err(error) => {
                return Err(tool_error("appreciation_failed", &error.to_string(), None));
            }
        };
        if let Some(cache) = &self.appreciation_cache {
            cache
                .store_completed(&request, &appreciation)
                .map_err(|error| {
                    tool_error("appreciation_cache_failed", &error.to_string(), None)
                })?;
        }
        Ok(Json(appreciation_output(
            input.poem_id,
            CacheHit {
                appreciation,
                source: CacheSource::Generated,
            },
        )))
    }

    #[tool(
        name = "generate_poem",
        description = "按主题生成五言绝句、七言绝句、五言律诗、七言律诗或词。可约束词牌及韵书韵部；固定句式会校验句数、字数和偶数句韵脚。结果始终标注“AI 生成，非古人作品”，只在内存中返回，绝不写入语料或赏析缓存。未配置服务商或密钥时返回带设置路径的普通结果。",
        annotations(
            title = "AI 作诗",
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn generate_poem(
        &self,
        Parameters(input): Parameters<GeneratePoemInput>,
    ) -> Result<Json<GeneratePoemOutput>, CallToolResult> {
        let core = self.ready()?;
        let constraint = generation_constraint(&input)?;
        let Some(provider) = &self.ai else {
            return Ok(Json(generation_configuration_required(&input)));
        };
        let request = PoemGenerationRequest::new(
            generation_prompt(&input, constraint.as_ref()),
            self.ai_model.clone(),
        );
        let generated = match provider.generate_poem(request).await {
            Ok(generated) => generated,
            Err(Error::AiKeyNotConfigured { .. }) => {
                return Ok(Json(generation_configuration_required(&input)));
            }
            Err(error) => return Err(tool_error("generation_failed", &error.to_string(), None)),
        };
        let validated =
            validate_generated_poem(core, &input, constraint.as_ref(), &generated.text)?;
        Ok(Json(GeneratePoemOutput {
            status: "ready".to_owned(),
            form: input.form.display_name().to_owned(),
            theme: input.theme,
            label: GENERATED_POEM_LABEL.to_owned(),
            text: Some(generated.text),
            lines: validated.lines,
            rhyme_book: constraint
                .as_ref()
                .map(|value| value.book.as_key().to_owned()),
            rhyme_group: constraint.map(|value| value.group),
            rhyme_feet: validated.rhyme_feet,
            model: Some(generated.model),
            message: "生成结果已通过请求中的句式与韵部约束校验，未写入任何数据。".to_owned(),
            settings_path: None,
            disclosure: AI_UNREVIEWED_DISCLOSURE.to_owned(),
        }))
    }
}

#[derive(Debug, Clone)]
struct GenerationConstraint {
    book: RhymeBook,
    group: String,
}

#[derive(Debug, Clone)]
struct ValidatedPoem {
    lines: Vec<String>,
    rhyme_feet: Vec<String>,
}

fn unconfigured_provider_id() -> ProviderId {
    ProviderId::new("unconfigured").unwrap_or_else(|_| unreachable!("内置供应商标识恒为合法 ASCII"))
}

fn appreciation_configuration_required(poem_id: String) -> AppreciatePoemOutput {
    AppreciatePoemOutput {
        status: "configuration_required".to_owned(),
        poem_id,
        text: None,
        source: None,
        model: None,
        template_version: None,
        message: "没有可用的随包赏析，且 AI 服务商或密钥尚未配置。".to_owned(),
        settings_path: Some(AI_SETTINGS_PATH.to_owned()),
        disclosure: AI_UNREVIEWED_DISCLOSURE.to_owned(),
    }
}

fn appreciation_output(poem_id: String, hit: CacheHit) -> AppreciatePoemOutput {
    let source = match hit.source {
        CacheSource::Shipped => "shipped",
        CacheSource::Local => "cache",
        CacheSource::Generated => "generated",
    };
    AppreciatePoemOutput {
        status: "ready".to_owned(),
        poem_id,
        text: Some(hit.appreciation.text),
        source: Some(source.to_owned()),
        model: Some(hit.appreciation.model),
        template_version: Some(hit.appreciation.template_version),
        message: "AI 赏析已返回；请结合原文、格律事实与有出处的历代集评独立核验。".to_owned(),
        settings_path: None,
        disclosure: AI_UNREVIEWED_DISCLOSURE.to_owned(),
    }
}

fn generation_configuration_required(input: &GeneratePoemInput) -> GeneratePoemOutput {
    GeneratePoemOutput {
        status: "configuration_required".to_owned(),
        form: input.form.display_name().to_owned(),
        theme: input.theme.clone(),
        label: GENERATED_POEM_LABEL.to_owned(),
        text: None,
        lines: Vec::new(),
        rhyme_book: input.rhyme_book.clone(),
        rhyme_group: input.rhyme_group.clone(),
        rhyme_feet: Vec::new(),
        model: None,
        message: "生成诗词需要先配置 AI 服务商与密钥。".to_owned(),
        settings_path: Some(AI_SETTINGS_PATH.to_owned()),
        disclosure: AI_UNREVIEWED_DISCLOSURE.to_owned(),
    }
}

fn generation_constraint(
    input: &GeneratePoemInput,
) -> Result<Option<GenerationConstraint>, CallToolResult> {
    if input.theme.trim().is_empty() {
        return Err(tool_error(
            "invalid_generation_request",
            "主题不能为空",
            None,
        ));
    }
    match input.form {
        GeneratedPoemForm::Ci
            if input
                .ci_tune
                .as_deref()
                .is_none_or(|tune| tune.trim().is_empty()) =>
        {
            return Err(tool_error(
                "invalid_generation_request",
                "生成词时必须提供 ci_tune",
                None,
            ));
        }
        GeneratedPoemForm::Ci => {}
        _ if input.ci_tune.is_some() => {
            return Err(tool_error(
                "invalid_generation_request",
                "只有 form=词 时可以提供 ci_tune",
                None,
            ));
        }
        _ => {}
    }

    let inferred_book = match input.form {
        GeneratedPoemForm::Ci => RhymeBook::Cilin,
        _ => RhymeBook::Pingshui,
    };
    match (&input.rhyme_book, &input.rhyme_group) {
        (None, None) => Ok(None),
        (book, Some(group)) => {
            let book = match book.as_deref() {
                Some(key) => RhymeBook::from_key(key).ok_or_else(|| {
                    tool_error(
                        "invalid_generation_request",
                        &format!("未知韵书 `{key}`；可用值为 pingshui 或 cilin"),
                        None,
                    )
                })?,
                None => inferred_book,
            };
            book.ensure_available().map_err(|error| {
                tool_error("invalid_generation_request", &error.to_string(), None)
            })?;
            let group = group.trim();
            if group.is_empty() {
                return Err(tool_error(
                    "invalid_generation_request",
                    "韵部名不能为空",
                    None,
                ));
            }
            Ok(Some(GenerationConstraint {
                book,
                group: group.to_owned(),
            }))
        }
        (Some(_), None) => Err(tool_error(
            "invalid_generation_request",
            "提供 rhyme_book 时必须同时提供 rhyme_group",
            None,
        )),
    }
}

fn generation_prompt(
    input: &GeneratePoemInput,
    constraint: Option<&GenerationConstraint>,
) -> String {
    let mut requirements = vec![
        format!("体式：{}", input.form.display_name()),
        format!("主题：{}", input.theme.trim()),
        "只输出正文，每句单独一行，不要标题、序号、注释或 Markdown。".to_owned(),
    ];
    if let Some((line_count, characters_per_line)) = input.form.fixed_shape() {
        requirements.push(format!(
            "必须恰好 {line_count} 句，每句恰好 {characters_per_line} 个汉字；偶数句押韵。"
        ));
    }
    if let Some(tune) = input.ci_tune.as_deref() {
        requirements.push(format!("词牌：{}", tune.trim()));
    }
    if let Some(constraint) = constraint {
        requirements.push(format!(
            "韵书约束：按{}押{}，所有指定韵脚必须属于该韵部。",
            constraint.book.display_name(),
            constraint.group
        ));
    }
    requirements.push(format!(
        "不得署古人姓名；产品会另行附加“{GENERATED_POEM_LABEL}”标签。"
    ));
    requirements.join("\n")
}

fn validate_generated_poem(
    core: &Yunjian,
    input: &GeneratePoemInput,
    constraint: Option<&GenerationConstraint>,
    text: &str,
) -> Result<ValidatedPoem, CallToolResult> {
    let lines = split_metrical_lines(text)
        .map(|line| content_chars(line).collect::<String>())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return Err(tool_error(
            "generation_invalid",
            "模型返回了空诗词正文",
            Some("请重试；无效生成不会写入任何数据"),
        ));
    }
    if let Some((expected_lines, expected_characters)) = input.form.fixed_shape() {
        if lines.len() != expected_lines {
            return Err(tool_error(
                "generation_invalid",
                &format!(
                    "{}必须为 {expected_lines} 句，模型返回了 {} 句",
                    input.form.display_name(),
                    lines.len()
                ),
                Some("请重试；无效生成不会写入任何数据"),
            ));
        }
        if let Some((index, actual)) = lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index, line.chars().count()))
            .find(|(_, count)| *count != expected_characters)
        {
            return Err(tool_error(
                "generation_invalid",
                &format!(
                    "{}第 {} 句应为 {expected_characters} 字，实为 {actual} 字",
                    input.form.display_name(),
                    index + 1
                ),
                Some("请重试；无效生成不会写入任何数据"),
            ));
        }
    }

    let foot_indexes = if input.form.fixed_shape().is_some() {
        (1..lines.len()).step_by(2).collect::<Vec<_>>()
    } else {
        (0..lines.len()).collect::<Vec<_>>()
    };
    let rhyme_feet = foot_indexes
        .iter()
        .filter_map(|index| lines[*index].chars().last())
        .collect::<Vec<_>>();
    if let Some(constraint) = constraint {
        for foot in &rhyme_feet {
            let groups = core
                .rhyme_groups_of(CharacterRhymesRequest {
                    character: *foot,
                    book: constraint.book,
                })
                .map_err(|error| {
                    tool_error("generation_validation_failed", &error.to_string(), None)
                })?;
            if !groups
                .iter()
                .any(|group| group.rhyme_group == constraint.group)
            {
                return Err(tool_error(
                    "generation_invalid",
                    &format!(
                        "韵脚「{foot}」不属于{}的{}",
                        constraint.book.display_name(),
                        constraint.group
                    ),
                    Some("请重试；无效生成不会写入任何数据"),
                ));
            }
        }
    }
    Ok(ValidatedPoem {
        lines,
        rhyme_feet: rhyme_feet
            .into_iter()
            .map(|foot| foot.to_string())
            .collect(),
    })
}

impl YunjianServer {
    fn ready(&self) -> Result<&Yunjian, CallToolResult> {
        match &self.core {
            CoreClient::Ready(core) => Ok(core),
            CoreClient::Missing => Err(CallToolResult::structured_error(serde_json::json!({
                "code": "corpus_missing",
                "message": "尚无可用的云笺语料库",
                "hint": "运行 `yunjian corpus fetch` 获取语料后重新启动 MCP 服务"
            }))),
        }
    }

    /// 高频字表：整个进程只算一次。
    ///
    /// 缓存在服务端而不是每次调用重算，因为它是一次覆盖索引上的分组扫描。算不出来（派生
    /// 结构缺失）时是空集，字面重叠项照样能算，只是会把常用字计入——退化而不是失败。
    fn stopwords(&self, core: &Yunjian) -> &BTreeSet<char> {
        self.stopwords.get_or_init(|| {
            core.frequent_content_chars(similarity::FREQUENT_CHAR_EXCLUSIONS)
                .map(|characters| characters.into_iter().collect())
                .unwrap_or_default()
        })
    }

    fn gather_candidates(
        &self,
        core: &Yunjian,
        source: &PoemFeatures,
        axes: &[SimilarityAxis],
    ) -> Result<Vec<String>, CallToolResult> {
        let mut pool: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(source.poem.stable_id.clone());
        for axis in axes {
            for id in self.axis_candidates(core, source, *axis)? {
                if pool.len() >= SIMILAR_CANDIDATE_POOL_CAP {
                    return Ok(pool);
                }
                if seen.insert(id.clone()) {
                    pool.push(id);
                }
            }
        }
        Ok(pool)
    }

    fn axis_candidates(
        &self,
        core: &Yunjian,
        source: &PoemFeatures,
        axis: SimilarityAxis,
    ) -> Result<Vec<String>, CallToolResult> {
        let map_error = |error: Error| tool_error("similar_failed", &error.to_string(), None);
        let ids = match axis {
            SimilarityAxis::Theme => {
                let mut ids = Vec::new();
                for tag in &source.tags {
                    let page = core
                        .browse_by_tag(TagBrowseRequest {
                            tag: tag.clone(),
                            cursor: None,
                        })
                        .map_err(map_error)?;
                    ids.extend(page.hits.into_iter().map(|hit| hit.stable_id));
                }
                ids
            }
            SimilarityAxis::Rhyme => {
                let mut ids = Vec::new();
                for membership in &source.rhyme_groups {
                    let matches = core
                        .find_by_rhyme_group(RhymeGroupSearchRequest {
                            book: membership.book,
                            rhyme_group: membership.group.clone(),
                            tone: ToneFilter::Only(membership.tone),
                        })
                        .map_err(map_error)?;
                    ids.extend(matches.hits.into_iter().map(|hit| hit.poem_id));
                }
                ids
            }
            SimilarityAxis::Tune => match source.poem.ci_tune.as_deref() {
                Some(tune) => core
                    .find_by_title(TitleSearchRequest {
                        query: tune.to_owned(),
                        cursor: None,
                    })
                    .map_err(map_error)?
                    .hits
                    .into_iter()
                    .map(|hit| hit.stable_id)
                    .collect(),
                None => Vec::new(),
            },
            SimilarityAxis::Author => core
                .find_by_author(AuthorSearchRequest {
                    query: source.poem.author.clone(),
                    cursor: None,
                })
                .map_err(map_error)?
                .hits
                .into_iter()
                .map(|hit| hit.stable_id)
                .collect(),
            SimilarityAxis::Dynasty => core
                .browse_by_dynasty(DynastyBrowseRequest {
                    dynasty: source.poem.dynasty.canonical.clone(),
                    cursor: None,
                })
                .map_err(map_error)?
                .hits
                .into_iter()
                .map(|hit| hit.stable_id)
                .collect(),
        };
        Ok(ids)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for YunjianServer {}

/// 同一 `work_group` 只留得分最高的一条，然后截到 `cap`。
///
/// 去重在截断**之前**：一首诗在语料里的多个版本会占满整个结果列表，先截后去重会让用户拿到
/// 20 条同一首诗。入参须已按得分降序排好。
fn dedup_by_work_group(scored: Vec<SimilarPoem>, cap: usize) -> Vec<SimilarPoem> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut kept = Vec::with_capacity(cap);
    for candidate in scored {
        if kept.len() >= cap {
            break;
        }
        if seen.insert(candidate.work_group.clone()) {
            kept.push(candidate);
        }
    }
    kept
}

fn matches_filter(wanted: Option<&str>, actual: &str) -> bool {
    wanted.is_none_or(|wanted| wanted.trim() == actual)
}

/// `usize` 转 `u32`，超界饱和。
///
/// 计数字段饱和而不是回绕：一个回绕后的行号或总数会被下游当成真值使用。
fn clamp_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn tool_error(code: &str, message: &str, hint: Option<&str>) -> CallToolResult {
    let mut payload = serde_json::json!({ "code": code, "message": message });
    if let Some(hint) = hint {
        payload["hint"] = serde_json::Value::String(hint.to_owned());
    }
    CallToolResult::structured_error(payload)
}

/// 把核心错误翻成工具级错误。
///
/// 「找不到这首诗」与「这条集评缺出处」必须是两个不同的 `code`：前者是调用方给错了 id，
/// 后者是语料库自身不合法。合并成一个错误码会让后者看起来像用户输入问题，从而不会有人去
/// 重建语料。
fn explain_error(poem_id: &str, error: &Error) -> CallToolResult {
    match error {
        Error::Search(_) => tool_error(
            "poem_not_found",
            &error.to_string(),
            Some("先用 search_poem 取得 poem_id，不要自行拼接"),
        ),
        Error::CommentaryCitationMissing { .. } => tool_error(
            "commentary_citation_missing",
            &error.to_string(),
            Some("这条集评缺出处，语料库需要重建；缺出处的集评不会以空字段返回"),
        ),
        _ => tool_error(
            "explain_failed",
            &format!("读取 `{poem_id}` 失败：{error}"),
            None,
        ),
    }
}

fn explain_output(detail: PoemDetail) -> ExplainPoemOutput {
    let PoemDetail {
        poem,
        author,
        tones,
        rhyme_groups,
        work_group_siblings,
        attribution_conflict,
        provenance,
        tags,
        commentaries,
    } = detail;
    ExplainPoemOutput {
        poem: PoemFacts {
            poem_id: poem.stable_id,
            title: poem.title,
            ci_tune: poem.ci_tune,
            author: poem.author,
            dynasty: poem.dynasty.canonical,
            dynasty_raw: poem.dynasty.raw,
            genre: poem.genre,
            body: poem.body,
            body_original: poem.body_original,
            line_count: poem.line_count,
            char_count: poem.char_count,
            work_group: poem.work_group,
        },
        author: AuthorFacts {
            name: author.name,
            dynasties: author
                .dynasties
                .into_iter()
                .map(|label| label.canonical)
                .collect(),
            poem_count: clamp_u32(author.poem_count),
        },
        tones: ToneFacts {
            book: tones.book.as_key().to_owned(),
            has_unknown: tones.has_unknown(),
            unknown_count: clamp_u32(tones.unknown_count),
            either_count: clamp_u32(tones.either_count),
            lines: tones
                .lines
                .into_iter()
                .map(|line| ToneLineFacts {
                    line_index: line.line_index,
                    text: line.text,
                    cells: line
                        .cells
                        .into_iter()
                        .map(|cell| ToneCellFacts {
                            character: cell.character,
                            tone: cell.tone.as_key().to_owned(),
                            readings: cell.readings,
                        })
                        .collect(),
                })
                .collect(),
        },
        rhyme_groups: rhyme_groups
            .into_iter()
            .map(|membership| RhymeGroupFacts {
                book: membership.book.as_key().to_owned(),
                group: membership.group,
                tone: membership.tone.as_key().to_owned(),
                confidence: membership.confidence.as_key().to_owned(),
            })
            .collect(),
        work_group_alternatives: work_group_siblings
            .into_iter()
            .map(attribution_facts)
            .collect(),
        attribution_conflict: attribution_conflict.map(|conflict| {
            let authors = conflict
                .authors()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            AttributionConflictFacts {
                work_group: conflict.work_group,
                authors,
                attributions: conflict
                    .attributions
                    .into_iter()
                    .map(attribution_facts)
                    .collect(),
            }
        }),
        provenance: ProvenanceFacts {
            source_locator: provenance.source_locator,
            source_locator_kind: provenance.source_locator_kind,
            source: provenance.source,
            revision: provenance.revision,
            kind: provenance.kind,
            license: provenance.license,
            license_class: provenance.license_class,
        },
        tags,
        commentaries: commentaries
            .into_iter()
            .map(|entry| CommentaryFacts {
                id: entry.id,
                text: entry.text,
                citation: CommentaryCitationFacts {
                    work: entry.citation.work,
                    author: entry.citation.author,
                    dynasty: entry.citation.dynasty.canonical,
                    work_completed_by: u32::from(entry.citation.work_completed_by),
                    source_note: entry.citation.source_note,
                },
            })
            .collect(),
        disclosure: OFFLINE_FACTS_DISCLOSURE.to_owned(),
    }
}

fn attribution_facts(attribution: Attribution) -> AttributionFacts {
    AttributionFacts {
        poem_id: attribution.stable_id,
        author: attribution.author,
        dynasty: attribution.dynasty.canonical,
        title: attribution.title,
        source_locator: attribution.source_locator,
        provenance_source: attribution.provenance_source,
        provenance_revision: attribution.provenance_revision,
    }
}

/// 全部工具的规范名单，供服务端自检与文档引用。
///
/// **离线子集**：无需 API key、无需网络。
pub const OFFLINE_TOOL_NAMES: [&str; 3] = ["search_poem", "explain_poem", "find_similar_poem"];

/// 在当前进程的 stdin/stdout 上运行服务，直到客户端关闭输入流。
///
/// # Errors
///
/// 初始化 MCP 会话或等待服务任务结束失败时返回错误。
pub async fn serve_stdio(server: YunjianServer) -> anyhow::Result<()> {
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
