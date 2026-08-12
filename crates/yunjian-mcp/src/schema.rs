//! MCP 工具的线上数据形状。
//!
//! # 为什么不直接序列化 `yunjian_core` 的类型
//!
//! 两个原因，缺一不可：
//!
//! 1. **技术上**：`outputSchema` 由 `schemars::JsonSchema` 生成，而 core 的类型只派生
//!    `Serialize` / `Deserialize`。让 core 依赖 `schemars` 会把一个「给 MCP 客户端看的」
//!    约束加到一个不知道 MCP 存在的层上。
//! 2. **契约上**：这一层是**对外承诺**。core 的内部字段重命名不该变成 MCP 客户端的破坏性
//!    变更，反之亦然；两侧各有自己的演进节奏，中间这层映射就是那道缝。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 每次工具调用都附带的性质声明。
///
/// **这三个工具都不产生 AI 文本。** 声明写进结果而不是只写在 `description` 里，是因为
/// 描述只有挑工具时会被读到，而结果会被贴进对话——用户看到的是后者。
pub const OFFLINE_FACTS_DISCLOSURE: &str =
    "本结果全部来自本地语料库的结构化事实与公有领域历代集评，不含 AI 生成内容，且未写入任何数据。";

/// 每个 AI 工具结果都携带的准确性声明。
pub const AI_UNREVIEWED_DISCLOSURE: &str =
    "本结果包含 AI 生成内容，未经人工审校，可能存在事实、典故或格律错误，请独立核验。";

/// AI 凭据的产品内设置路径。
pub const AI_SETTINGS_PATH: &str = "云笺 → 设置 → AI 服务商与密钥";

// ---------------------------------------------------------------- search_poem

/// `search_poem` 的入参。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchPoemInput {
    /// 检索词，可以是一个字、一个词、一句或残句；支持繁体与异体输入。
    pub query: String,
    /// 只保留该作者的命中；在**当前页内**过滤。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// 只保留该朝代的命中；在**当前页内**过滤。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynasty: Option<String>,
    /// 单页命中上限；缺省 10，超出 50 时截断为 50 而不是报错。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// 上一次结果里的 `next_cursor`；不透明串，不要构造或解析它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 一条检索命中。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchPoemHit {
    /// 作品稳定标识，可直接传给 `explain_poem` 与 `find_similar_poem`。
    pub poem_id: String,
    /// 题目；词作为「词牌·题目」的合成形式。
    pub title: String,
    /// 作者名。
    pub author: String,
    /// 朝代。
    pub dynasty: String,
    /// 命中所在的行序号，从 0 起。
    pub matched_line_index: u32,
    /// 命中所在的行，未截断。
    pub snippet: String,
    /// 命中在 `snippet` 里的字符区间，按 Unicode 字符计数。
    pub highlights: Vec<SearchPoemHighlight>,
}

/// 命中在片段里的一段字符区间。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchPoemHighlight {
    /// 起始字符下标，含。
    pub start: u32,
    /// 结束字符下标，不含。
    pub end: u32,
}

/// `search_poem` 的返回。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchPoemOutput {
    /// 归一化后实际执行的检索词。
    pub query: String,
    /// 服务端最终采用的单页上限。
    pub limit: u32,
    /// 请求的 `limit` 是否被服务端截断到 [`super::SEARCH_LIMIT_MAX`]。
    pub limit_clamped: bool,
    /// 本页命中。
    pub hits: Vec<SearchPoemHit>,
    /// 过滤前的命中总数估计。
    pub total_estimate: u32,
    /// 下一页游标；为 `null` 表示已到末页。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// 调用方必须读到的限定说明，例如「作者过滤只作用于本页」。
    pub notes: Vec<String>,
    /// 结果性质声明，恒为 [`OFFLINE_FACTS_DISCLOSURE`]。
    pub disclosure: String,
}

// ---------------------------------------------------------------- explain_poem

/// `explain_poem` 的入参。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainPoemInput {
    /// 作品稳定标识，来自 `search_poem` 的 `poem_id`。
    pub poem_id: String,
}

/// 作品本体。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct PoemFacts {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 题目。
    pub title: String,
    /// 词牌；诗为 `null`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_tune: Option<String>,
    /// 作者名。
    pub author: String,
    /// 规范化朝代。
    pub dynasty: String,
    /// 上游原始朝代写法。
    pub dynasty_raw: String,
    /// 体裁。
    pub genre: String,
    /// 规范简体正文，含标点。
    pub body: String,
    /// 上游原始字形正文。
    pub body_original: String,
    /// 行数。
    pub line_count: u32,
    /// 正文字数，不含标点与空白。
    pub char_count: u32,
    /// 同一正文的分组键，用于识别重出与异文。
    pub work_group: String,
}

/// 一个字的平仄。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ToneCellFacts {
    /// 该字。
    pub character: String,
    /// 平仄判定：`level` 平、`oblique` 仄、`either` 平仄两读、`unknown` 韵书未收。
    pub tone: String,
    /// 该字在韵书里的全部读音归属，可能为空。
    pub readings: Vec<String>,
}

/// 一行的平仄。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ToneLineFacts {
    /// 行序号，从 0 起。
    pub line_index: u32,
    /// 该行正文。
    pub text: String,
    /// 逐字平仄。
    pub cells: Vec<ToneCellFacts>,
}

/// 全篇平仄标注。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ToneFacts {
    /// 反查所依据的韵书。
    pub book: String,
    /// 逐行平仄；已挂平仄时行数必与正文一致。
    pub lines: Vec<ToneLineFacts>,
    /// `unknown` 的字数。**不要把它当成平声**：韵书只收韵字，未收即未知。
    pub unknown_count: u32,
    /// `either`（平仄两读）的字数。
    pub either_count: u32,
    /// 是否存在未知平仄。
    pub has_unknown: bool,
}

/// 一条韵部归属。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct RhymeGroupFacts {
    /// 韵书。
    pub book: String,
    /// 韵部名。
    pub group: String,
    /// 声调。
    pub tone: String,
    /// 可信度：`unambiguous` 本无歧义、`resolved_by_vote` 由韵脚投票解出、
    /// `unresolved` 未能唯一消歧。后者**不是**结论。
    pub confidence: String,
}

/// 同一 `work_group` 下的一条归属。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AttributionFacts {
    /// 该归属对应的作品标识。
    pub poem_id: String,
    /// 该记录声称的作者。
    pub author: String,
    /// 该记录声称的朝代。
    pub dynasty: String,
    /// 该记录的题目。
    pub title: String,
    /// 上游定位符。
    pub source_locator: String,
    /// 上游数据源名。
    pub provenance_source: String,
    /// 上游锁定的 revision。
    pub provenance_revision: String,
}

/// 同一正文挂在多个作者名下的冲突。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AttributionConflictFacts {
    /// 冲突所在的分组键。
    pub work_group: String,
    /// 涉及的作者名，去重且有序。
    pub authors: Vec<String>,
    /// 全部冲突归属。
    pub attributions: Vec<AttributionFacts>,
}

/// 溯源。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ProvenanceFacts {
    /// 上游定位符。
    pub source_locator: String,
    /// 定位符类型。
    pub source_locator_kind: String,
    /// 上游数据源名。
    pub source: String,
    /// 上游锁定的 revision。
    pub revision: String,
    /// 内容类型。
    pub kind: String,
    /// 许可标识。
    pub license: String,
    /// 许可类别。
    pub license_class: String,
}

/// 一条集评的出处。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct CommentaryCitationFacts {
    /// 出处书名。
    pub work: String,
    /// 评者。
    pub author: String,
    /// 评者朝代。
    pub dynasty: String,
    /// 成书年份下限。
    pub work_completed_by: u32,
    /// 卷次与底本说明。
    pub source_note: String,
}

/// 一条公有领域历代集评。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct CommentaryFacts {
    /// 集评标识。
    pub id: String,
    /// 集评正文。
    pub text: String,
    /// 出处。**永远存在**：缺出处的集评在读取时即为类型化错误，不会以空字段抵达这里。
    pub citation: CommentaryCitationFacts,
}

/// `explain_poem` 的返回。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ExplainPoemOutput {
    /// 作品本体。
    pub poem: PoemFacts,
    /// 作者在语料里的收录情况。
    pub author: AuthorFacts,
    /// 平仄标注，未知位置以 `unknown` 保留。
    pub tones: ToneFacts,
    /// 逐韵书的韵部归属，含可信度。
    pub rhyme_groups: Vec<RhymeGroupFacts>,
    /// 同一 `work_group` 下的其它记录，即本篇的替代项。
    pub work_group_alternatives: Vec<AttributionFacts>,
    /// 归属冲突；只挂一个作者时为 `null`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution_conflict: Option<AttributionConflictFacts>,
    /// 溯源。
    pub provenance: ProvenanceFacts,
    /// 策展标签。
    pub tags: Vec<String>,
    /// 公有领域历代集评，逐条带出处。
    pub commentaries: Vec<CommentaryFacts>,
    /// 结果性质声明，恒为 [`OFFLINE_FACTS_DISCLOSURE`]。
    pub disclosure: String,
}

/// 作者在语料里的收录情况。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AuthorFacts {
    /// 规范化作者名。
    pub name: String,
    /// 该作者名下的朝代写法，可能多于一个。
    pub dynasties: Vec<String>,
    /// 语料里该作者的作品数。
    pub poem_count: u32,
}

// ---------------------------------------------------------------- find_similar_poem

/// 候选来源轴。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SimilarityAxis {
    /// 共享至少一个策展标签。
    Theme,
    /// 与本篇的某个韵部归属同书、同韵部、同声调。
    Rhyme,
    /// 同词牌。
    Tune,
    /// 同作者。
    Author,
    /// 同朝代。
    Dynasty,
}

impl SimilarityAxis {
    /// 稳定键，与 JSON 表示逐字一致。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Rhyme => "rhyme",
            Self::Tune => "tune",
            Self::Author => "author",
            Self::Dynasty => "dynasty",
        }
    }

    /// 全部轴，次序固定。
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::Theme,
            Self::Rhyme,
            Self::Tune,
            Self::Author,
            Self::Dynasty,
        ]
    }
}

/// `find_similar_poem` 的入参。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindSimilarPoemInput {
    /// 作为基准的作品标识。
    pub poem_id: String,
    /// 只从这一条轴上取候选；缺省时取全部轴的并集。**它不改变打分口径**，
    /// 四项权重恒定，只影响哪些作品进入候选池。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<SimilarityAxis>,
}

/// 一条相似度得分的构成。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SimilarityComponents {
    /// 共享标签项：`0.4 × |交集| / |并集|`。
    pub shared_tags: f64,
    /// 同韵部项：命中记 `0.25`，否则 `0`。
    pub same_rhyme_group: f64,
    /// 同词牌项：命中记 `0.2`，否则 `0`。
    pub same_ci_tune: f64,
    /// 字面重叠项：`0.15 × Jaccard(正文字集, 已排除高频字)`。
    pub character_overlap: f64,
}

/// 一条相似作品。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SimilarPoem {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 题目。
    pub title: String,
    /// 作者名。
    pub author: String,
    /// 朝代。
    pub dynasty: String,
    /// 词牌；诗为 `null`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_tune: Option<String>,
    /// 同一正文的分组键；同组只保留得分最高的一条。
    pub work_group: String,
    /// 四项之和，落在 `[0, 1]`。
    pub score: f64,
    /// 得分构成，逐项可核。
    pub components: SimilarityComponents,
    /// 该候选是从哪些轴上取到的，次序固定。
    pub matched_axes: Vec<String>,
}

/// 打分口径，随每次结果回传以便审计。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SimilarityWeights {
    /// 共享标签项权重。
    pub shared_tags: f64,
    /// 同韵部项权重。
    pub same_rhyme_group: f64,
    /// 同词牌项权重。
    pub same_ci_tune: f64,
    /// 字面重叠项权重。
    pub character_overlap: f64,
}

/// `find_similar_poem` 的返回。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct FindSimilarPoemOutput {
    /// 基准作品标识。
    pub poem_id: String,
    /// 请求指定的轴；缺省时为 `null`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_axis: Option<String>,
    /// 本次实际取过候选的轴，次序固定。
    pub axes_used: Vec<String>,
    /// 打分口径。
    pub weights: SimilarityWeights,
    /// 字面重叠项排除掉的高频字个数。
    pub excluded_frequent_chars: u32,
    /// 候选池在打分前被截断到的上限。
    pub candidate_pool_cap: u32,
    /// 结果条数上限。
    pub result_cap: u32,
    /// 相似作品，按得分降序、同分按 `poem_id` 升序。
    pub matches: Vec<SimilarPoem>,
    /// 打分方法的一句话说明。
    pub method: String,
    /// 结果性质声明，恒为 [`OFFLINE_FACTS_DISCLOSURE`]。
    pub disclosure: String,
}

// ---------------------------------------------------------------- appreciate_poem

/// `appreciate_poem` 的入参。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppreciatePoemInput {
    /// 作品稳定标识，来自 `search_poem` 的 `poem_id`。
    pub poem_id: String,
    /// 可选的赏析风格，例如「简明」「学术」或「面向初学者」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// `appreciate_poem` 的返回。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AppreciatePoemOutput {
    /// `ready` 表示已有赏析；`configuration_required` 表示需要配置服务商或密钥。
    pub status: String,
    /// 请求的作品稳定标识。
    pub poem_id: String,
    /// 赏析正文；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 结果来源：`shipped`、`cache` 或 `generated`；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 生成模型；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 提示词模板版本；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
    /// 面向用户的状态说明。
    pub message: String,
    /// 需要配置时给出的产品内路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
    /// AI 内容准确性声明，恒为 [`AI_UNREVIEWED_DISCLOSURE`]。
    pub disclosure: String,
}

// ---------------------------------------------------------------- generate_poem

/// MCP 支持生成的诗词体式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum GeneratedPoemForm {
    /// 五言绝句，四句、每句五字。
    #[serde(rename = "五言绝句")]
    FiveCharacterQuatrain,
    /// 七言绝句，四句、每句七字。
    #[serde(rename = "七言绝句")]
    SevenCharacterQuatrain,
    /// 五言律诗，八句、每句五字。
    #[serde(rename = "五言律诗")]
    FiveCharacterRegulatedVerse,
    /// 七言律诗，八句、每句七字。
    #[serde(rename = "七言律诗")]
    SevenCharacterRegulatedVerse,
    /// 词；必须同时给出 `ci_tune`。
    #[serde(rename = "词")]
    Ci,
}

impl GeneratedPoemForm {
    /// 与 JSON 表示一致的中文体式名。
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::FiveCharacterQuatrain => "五言绝句",
            Self::SevenCharacterQuatrain => "七言绝句",
            Self::FiveCharacterRegulatedVerse => "五言律诗",
            Self::SevenCharacterRegulatedVerse => "七言律诗",
            Self::Ci => "词",
        }
    }

    /// 固定句式的 `(句数, 每句字数)`；词没有固定值。
    #[must_use]
    pub const fn fixed_shape(self) -> Option<(usize, usize)> {
        match self {
            Self::FiveCharacterQuatrain => Some((4, 5)),
            Self::SevenCharacterQuatrain => Some((4, 7)),
            Self::FiveCharacterRegulatedVerse => Some((8, 5)),
            Self::SevenCharacterRegulatedVerse => Some((8, 7)),
            Self::Ci => None,
        }
    }
}

/// `generate_poem` 的入参。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratePoemInput {
    /// 目标体式。
    pub form: GeneratedPoemForm,
    /// 创作主题。
    pub theme: String,
    /// 词牌；`form=词` 时必填，其余体式不得填写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci_tune: Option<String>,
    /// 韵书稳定键：诗用 `pingshui`，词用 `cilin`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhyme_book: Option<String>,
    /// 目标韵部名，例如平水韵「七阳」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhyme_group: Option<String>,
}

/// `generate_poem` 的返回。
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GeneratePoemOutput {
    /// `ready` 表示生成并校验成功；`configuration_required` 表示需要配置服务商或密钥。
    pub status: String,
    /// 请求的中文体式名。
    pub form: String,
    /// 请求主题。
    pub theme: String,
    /// 生成结果的强制身份标签。
    pub label: String,
    /// 生成正文；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 去除标点后的逐句正文；需要配置时为空。
    pub lines: Vec<String>,
    /// 本次校验使用的韵书稳定键。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhyme_book: Option<String>,
    /// 本次约束的韵部名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhyme_group: Option<String>,
    /// 参与押韵校验的句末字。
    pub rhyme_feet: Vec<String>,
    /// 生成模型；需要配置时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 面向用户的状态说明。
    pub message: String,
    /// 需要配置时给出的产品内路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
    /// AI 内容准确性声明，恒为 [`AI_UNREVIEWED_DISCLOSURE`]。
    pub disclosure: String,
}
