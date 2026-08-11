//! 主题标签检索与作品详情（含历代集评）。
//!
//! # 两个入口，一个共同前提
//!
//! [`browse_by_tag`] 与 [`poem_detail`] 都只做等值连接，一行 SQL 也不引用 `poem_fts`
//! ——理由与 [`crate::search::meta`] 相同：标签名（「思乡」）与 `stable_id` 都是完整的
//! 字段值，FTS5 trigram 分词器在长度不足 3 的输入上匹配不到任何行，而**这种坏法不会
//! 报错**。测试里的 `EXPLAIN QUERY PLAN` 断言逐条钉住这一点。
//!
//! # 标签从哪里来
//!
//! **构建期，由签入的策展词表按显式规则与评审名单产出**（`yunjian-corpus` 的
//! `tag` 模块与 `tags.toml`）。运行期只读 `poem_tag`，绝不在此处推断标签：一个在运行期
//! 生成的标签既无法复核，也会随实现改动而变，而标签直接决定用户看到哪些诗。
//!
//! 未登记的标签名返回 [`Error::Search`] 并列出现有标签，而不是空页——空页分不清
//! 「这个标签下没有诗」与「根本没有这个标签」，与 [`crate::search::meta::browse_by_dynasty`]
//! 和 [`Error::RhymeBookUnavailable`] 是同一条规则：缺数据不得呈现为否定答案。
//!
//! # 平仄从哪里来
//!
//! **运行期由随包的 `rhyme` 表按字反查**，而不是随包一张逐字平仄表。两条理由：
//!
//! 1. **上游平仄表只覆盖 `全唐诗`。** `chinese-poetry` 的 `strains/json/*` 与
//!    `全唐诗/poet.*` 一一对应，宋词与宋诗一条都没有。随包它只能给不到十分之一的
//!    作品标注平仄，而按韵书反查对每一首都成立。
//! 2. **本项目已裁定以韵书的反向索引为准。** 上游平仄表把「空」标成仄，而它在平水韵
//!    里同时见于上平一东（平）、上声一董与去声一送（仄）——采信上游会把「空山不见人」
//!    判为出律（见 `yunjian-corpus` 的 `rhyme` 模块）。既然分歧时以韵书为准，
//!    就没有理由再随包一份要被覆盖的数据。
//!
//! 代价是韵书未收的字得不到平仄。那正是 [`Tone::Unknown`] 的用途：它**不是**平声的
//! 默认值，[`Tone::is_level`] 对它返回 `false`，序列化成 `"unknown"` 而不是省略。
//! 把未知渲染成平会让「格律是否合规」这个判断建立在编造的数据上。
//!
//! # 集评必须与出处一同呈现
//!
//! 集评这条通道能存在的全部理由是「前现代评语已过保护期，且逐条可复核」。一条缺出处
//! 的评语两个条件都不满足，因此 [`poem_detail`] 遇到它返回
//! [`Error::CommentaryCitationMissing`] 并点名那条集评的 id，**而不是渲染一个空字段**
//! ——后者会让界面上出现一段来路不明的文言，与被排除掉的第三方现代赏析在呈现上毫无
//! 区别。构建期已有校验（`commentary::validate_seed`），运行期这一道是第二重：
//! 语料库文件可以来自任何一次构建，运行期不该假定它必然通过过构建期的门禁。

#[cfg(test)]
mod tests;

use crate::search::meta::{
    Attribution, AttributionConflict, DynastyLabel, META_PAGE_LIMIT, MetaMatch, MetaPage,
    find_work_group_attributions,
};
use crate::search::query::plan_metadata_query;
use crate::{CorpusHandle, Error, QueryPlan, Result, RhymeBook, RhymeConfidence, RhymeTone};
use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 报错时最多列出多少个现有标签。
///
/// 全量列出会在语料变大后把一条错误信息变成一屏噪声；截断后附总数，用户仍能判断
/// 自己是拼错了还是这个标签确实不存在。
const MAX_LISTED_TAGS: usize = 40;

/// 一个字的平仄。
///
/// **没有 `Default` 实现，这是刻意的。** 平仄有四种状态而其中一种是「不知道」，
/// 给它一个默认值就等于替韵书作答。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// 平。
    Level,
    /// 仄（上、去、入，以及词林正韵不再细分的仄声）。
    Oblique,
    /// 平仄两读。多音字在同一本韵书里既入平声部又入仄声部时是这一档。
    Either,
    /// 不知道：韵书未收此字，或上游标注为 `？` / `○`。
    Unknown,
}

impl Tone {
    /// 写进报告与界面的稳定键。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Oblique => "oblique",
            Self::Either => "either",
            Self::Unknown => "unknown",
        }
    }

    /// 用于展示的单字标记。未知用 `？`，与上游的写法一致。
    #[must_use]
    pub const fn marker(self) -> char {
        match self {
            Self::Level => '平',
            Self::Oblique => '仄',
            Self::Either => '多',
            Self::Unknown => '？',
        }
    }

    /// 该位置是否确定为平声。
    ///
    /// [`Self::Unknown`] 与 [`Self::Either`] 都返回 `false`：格律判断只能建立在确定
    /// 的平声上，把不确定算成平会让判断结果比数据更肯定。
    #[must_use]
    pub const fn is_level(self) -> bool {
        matches!(self, Self::Level)
    }

    /// 解析上游平仄表的标记。
    ///
    /// `？` 与 `○` 是上游明确的「未定」写法，必须落到 [`Self::Unknown`]。非平仄字符
    /// （标点）返回 `None`，由调用方跳过。
    #[must_use]
    pub const fn from_marker(marker: char) -> Option<Self> {
        match marker {
            '平' => Some(Self::Level),
            '仄' => Some(Self::Oblique),
            '多' => Some(Self::Either),
            '？' | '○' | '?' => Some(Self::Unknown),
            _ => None,
        }
    }

    /// 由某个字在一本韵书里的全部声调归并出平仄。
    ///
    /// 空集合是「韵书未收此字」，返回 [`Self::Unknown`]。
    fn from_tones(tones: &BTreeSet<RhymeTone>) -> Self {
        if tones.is_empty() {
            return Self::Unknown;
        }
        let level = tones.iter().any(|tone| tone.is_level());
        let oblique = tones.iter().any(|tone| !tone.is_level());
        match (level, oblique) {
            (true, true) => Self::Either,
            (true, false) => Self::Level,
            (false, true) => Self::Oblique,
            (false, false) => Self::Unknown,
        }
    }
}

/// 一个字连同它的平仄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToneCell {
    /// 正文里的那个字。
    pub character: String,
    /// 平仄。
    pub tone: Tone,
    /// 该字在这本韵书里的全部声调，按稳定键排序。[`Tone::Unknown`] 时为空。
    ///
    /// 透出它是因为 [`Tone::Either`] 只说「两读」，不说读哪两个；而「空」在一东（平）、
    /// 一董（上）、一送（去）三处出现这件事，是用户判断该按哪个读音的唯一依据。
    pub readings: Vec<String>,
}

/// 一行正文的平仄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToneLine {
    /// 句序号，自 0 起。
    pub line_index: u32,
    /// 该句正文（含原有标点）。
    pub text: String,
    /// 逐字平仄，已跳过标点与空白。
    pub cells: Vec<ToneCell>,
}

/// 一首作品的平仄标注。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToneAnnotation {
    /// 反查所用的韵书。
    pub book: RhymeBook,
    /// 逐句标注。
    pub lines: Vec<ToneLine>,
    /// 未知位置的个数。非零时界面必须显式呈现「未知」，不得按平声渲染。
    pub unknown_count: usize,
    /// 平仄两读位置的个数。
    pub either_count: usize,
}

impl ToneAnnotation {
    /// 有没有未知位置。
    #[must_use]
    pub const fn has_unknown(&self) -> bool {
        self.unknown_count > 0
    }

    /// 逐字平仄的展示串，逐句用 `/` 分隔。未知位置写 `？`。
    #[must_use]
    pub fn display(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.cells
                    .iter()
                    .map(|cell| cell.tone.marker())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// 一首作品在某本韵书里的韵部归属。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RhymeGroupMembership {
    /// 韵书。
    pub book: RhymeBook,
    /// 韵部名，如 `七阳`。**不带声部前缀**，声调由 [`Self::tone`] 承载。
    pub group: String,
    /// 声调。
    pub tone: RhymeTone,
    /// todo 18 韵脚投票给出的可信度。
    pub confidence: RhymeConfidence,
}

impl RhymeGroupMembership {
    /// 这条归属能否作为肯定的押韵判断。
    ///
    /// [`RhymeConfidence::Unresolved`] 不能：未消歧意味着我们不知道它押哪个韵部，
    /// 把候选当结论就是把猜测报成判断。
    #[must_use]
    pub const fn is_positive_claim(&self) -> bool {
        self.confidence.is_positive_claim()
    }
}

/// 一条集评的出处。每个字段都非空——空值在 [`poem_detail`] 里是类型化错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentaryCitation {
    /// 所引著作，如《沧浪诗话》。
    pub work: String,
    /// 评者。
    pub author: String,
    /// 评者朝代的规范键与上游原串。
    pub dynasty: DynastyLabel,
    /// 成书年上界。必然早于 1912，由随包 schema 的 CHECK 与构建期校验共同保证。
    pub work_completed_by: u16,
    /// 卷次/章节定位符与所据版本。引用要能被复核，这一条是复核的入口。
    pub source_note: String,
}

/// 一条历代集评。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentaryEntry {
    /// 集评 id。出错时错误信息点名的就是它。
    pub id: String,
    /// 评语正文，前现代文言，公有领域。
    pub text: String,
    /// 出处。**不是 `Option`**：缺出处的集评不构造成这个类型，而是让 [`poem_detail`]
    /// 返回 [`Error::CommentaryCitationMissing`]。
    pub citation: CommentaryCitation,
}

/// 作品的溯源信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// 上游定位符。`stable_id` 就是从它铸造的。
    pub source_locator: String,
    /// 定位符类型：`native`（上游自带 id）或 `positional`（文件 + 序号）。
    pub source_locator_kind: String,
    /// 上游数据源名。
    pub source: String,
    /// 上游锁定的 revision。
    pub revision: String,
    /// 内容类别：`原文` / `集评-PD` / `AI`。
    pub kind: String,
    /// 上游许可。
    pub license: String,
    /// 许可类别：`public_domain` 或 `permissive`。
    pub license_class: String,
}

/// 作品本体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoemRecord {
    /// 用户可见的唯一键。
    pub stable_id: String,
    /// 正文内容摘要。与 [`Self::stable_id`] 分离，故文本修正不会改身份。
    pub content_hash: String,
    /// 规范题目。
    pub title: String,
    /// 上游原样题目。
    pub title_raw: String,
    /// 词牌；诗为 `None`。
    pub ci_tune: Option<String>,
    /// 作者。
    pub author: String,
    /// 朝代的规范键与原串。
    pub dynasty: DynastyLabel,
    /// 体裁。
    pub genre: String,
    /// 规范简体正文。
    pub body: String,
    /// 源字形正文，与上游逐字节相同。
    pub body_original: String,
    /// 描述 [`Self::body_original`] 的书写系统。
    pub script: String,
    /// 首句。
    pub first_line: String,
    /// 逐句末字。
    pub last_chars: Vec<String>,
    /// 句数。
    pub line_count: u32,
    /// 正文字数。
    pub char_count: u32,
    /// 不含作者的作品分组键。
    pub work_group: String,
    /// 含作者的版本分组键。
    pub edition_group: String,
}

/// 作者记录。
///
/// 随包 `author` 表只有 `name` 一列——上游的作者小传虽然是公有领域文言，但当前随包
/// schema 未收它，所以这里如实只给名字，另附由 `poem` 聚合出的朝代与作品数。
/// 编一个空的小传字段会让调用方以为将来填上就够了，而实际要动的是 schema。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorRecord {
    /// 作者名。它是 `poem.author` 的外键目标，故必然存在。
    pub name: String,
    /// 该作者名下出现过的朝代标签。
    pub dynasties: Vec<DynastyLabel>,
    /// 该作者名下的作品数。
    pub poem_count: usize,
}

/// 一个标签及其作品数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSummary {
    /// 标签名。
    pub name: String,
    /// 带该标签的作品数。
    pub poem_count: usize,
}

/// 作品详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoemDetail {
    /// 作品本体。
    pub poem: PoemRecord,
    /// 作者记录。
    pub author: AuthorRecord,
    /// 平仄标注。
    pub tones: ToneAnnotation,
    /// 逐韵书的韵部归属，含可信度。
    pub rhyme_groups: Vec<RhymeGroupMembership>,
    /// 同一 `work_group` 下的其它记录（不含本篇）。
    pub work_group_siblings: Vec<Attribution>,
    /// 归属冲突；同一正文只挂一个作者时为 `None`。
    pub attribution_conflict: Option<AttributionConflict>,
    /// 溯源。
    pub provenance: Provenance,
    /// 构建期打上的策展标签，有序。
    pub tags: Vec<String>,
    /// 公有领域历代集评，每条都带出处。
    pub commentaries: Vec<CommentaryEntry>,
}

// ---------------------------------------------------------------- 公开入口

/// 列出语料里的全部标签及其作品数。
///
/// 走 `poem_tag_idx (tag, poem_id)` 覆盖索引；`tag` 表里已声明但一首诗都没命中的标签
/// 也会出现，`poem_count` 为 0。**不过滤掉它们**：一个计数为零的标签说明词表声明了它
/// 而语料里没有对应作品，那是需要被看见的事实，静默隐藏会让人以为它不存在。
pub fn list_tags(handle: &CorpusHandle) -> Result<Vec<TagSummary>> {
    let connection = handle.connect()?;
    let mut statement = connection.prepare_cached(TAG_SUMMARY_SQL)?;
    let rows = statement.query_map([], |row| {
        Ok(TagSummary {
            name: row.get(0)?,
            poem_count: usize::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 按策展标签浏览。
///
/// 未登记的标签名返回 [`Error::Search`] 并列出现有标签，而不是空页。
pub fn browse_by_tag(handle: &CorpusHandle, tag: &str, cursor: Option<&str>) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, tag)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    let connection = handle.connect()?;
    ensure_tag_declared(&connection, &normalized)?;
    crate::search::meta::run_branches(
        &connection,
        vec![crate::search::meta::Branch {
            rank: 0,
            sql: TAG_POEMS_SQL.to_owned(),
            binds: vec![crate::search::meta::text(&normalized)],
            matched_on: MetaMatch::Tag,
        }],
        cursor,
        normalized,
    )
}

/// 作品详情：本体、作者、平仄、韵部、`work_group` 兄弟项、溯源、标签与历代集评。
///
/// 集评必与出处一同返回；任一必填出处字段为空即返回
/// [`Error::CommentaryCitationMissing`] 并点名那条集评的 id。
pub fn poem_detail(handle: &CorpusHandle, poem_id: &str) -> Result<PoemDetail> {
    if poem_id.trim().is_empty() {
        return Err(Error::Search("作品详情需要一个 stable_id".to_owned()));
    }
    let connection = handle.connect()?;
    let poem = load_poem(&connection, poem_id)?;
    let author = load_author(&connection, &poem.author)?;
    let tones = annotate_tones(&connection, &poem.body, RhymeBook::Pingshui)?;
    let rhyme_groups = load_rhyme_groups(&connection, poem_id)?;
    let tags = load_tags(&connection, poem_id)?;
    let commentaries = load_commentaries(&connection, poem_id)?;

    let all_attributions = find_work_group_attributions(handle, &poem.work_group)?;
    let work_group_siblings: Vec<Attribution> = all_attributions
        .iter()
        .filter(|attribution| attribution.stable_id != poem.stable_id)
        .cloned()
        .collect();
    let mut authors: Vec<&str> = all_attributions
        .iter()
        .map(|attribution| attribution.author.as_str())
        .collect();
    authors.sort_unstable();
    authors.dedup();
    let attribution_conflict = (authors.len() > 1).then(|| AttributionConflict {
        work_group: poem.work_group.clone(),
        attributions: all_attributions,
    });

    let provenance = load_provenance(&connection, poem_id)?;
    Ok(PoemDetail {
        poem,
        author,
        tones,
        rhyme_groups,
        work_group_siblings,
        attribution_conflict,
        provenance,
        tags,
        commentaries,
    })
}

// ---------------------------------------------------------------- SQL

/// 标签作品：驱动表是 `poem_tag`，`poem` 只被按主键点查。
const TAG_POEMS_SQL: &str = concat!(
    "SELECT p.stable_id, p.title, p.title_raw, p.ci_tune, p.author, p.dynasty, p.dynasty_raw, ",
    "p.first_line, p.work_group, p.genre, p.line_count, p.char_count ",
    "FROM poem_tag AS t JOIN poem AS p ON p.stable_id = t.poem_id ",
    "WHERE t.tag = ?1 AND t.poem_id > ?2 ORDER BY t.poem_id LIMIT ?3"
);

/// 全部标签及其作品数。`tag` 是驱动表，故计数为零的标签也会出现。
const TAG_SUMMARY_SQL: &str = "SELECT g.name, COUNT(t.poem_id) FROM tag AS g \
     LEFT JOIN poem_tag AS t ON t.tag = g.name GROUP BY g.name ORDER BY g.name";

/// 标签是否已在 `tag` 表里登记。
const TAG_EXISTS_SQL: &str = "SELECT 1 FROM tag WHERE name = ?1";

/// 现有标签名，供报错时列出。
const TAG_NAMES_SQL: &str = "SELECT name FROM tag ORDER BY name";

const POEM_DETAIL_SQL: &str = "SELECT stable_id, content_hash, title, title_raw, ci_tune, author, \
     dynasty, dynasty_raw, genre, body, body_original, script, first_line, last_chars, \
     line_count, char_count, work_group, edition_group FROM poem WHERE stable_id = ?1";

const PROVENANCE_SQL: &str = "SELECT source_locator, source_locator_kind, provenance_source, \
     provenance_revision, provenance_kind, provenance_license, provenance_license_class \
     FROM poem WHERE stable_id = ?1";

const AUTHOR_EXISTS_SQL: &str = "SELECT name FROM author WHERE name = ?1";

const AUTHOR_FACETS_SQL: &str = "SELECT DISTINCT dynasty, dynasty_raw FROM poem \
     WHERE author = ?1 ORDER BY dynasty, dynasty_raw";

const AUTHOR_COUNT_SQL: &str = "SELECT COUNT(*) FROM poem WHERE author = ?1";

const POEM_TAGS_SQL: &str = "SELECT tag FROM poem_tag WHERE poem_id = ?1 ORDER BY tag";

const POEM_RHYME_GROUPS_SQL: &str = "SELECT rhyme_book, rhyme_group, tone, confidence \
     FROM poem_rhyme_group WHERE poem_id = ?1 ORDER BY rhyme_book, rhyme_group, tone";

/// 一个字在一本韵书里的全部声调。走 `rhyme_character_idx` 覆盖索引。
const CHARACTER_TONES_SQL: &str =
    "SELECT DISTINCT tone FROM rhyme WHERE rhyme_book = ?1 AND character = ?2 ORDER BY tone";

const COMMENTARY_SQL: &str = "SELECT id, text, citation_work, citation_author, citation_dynasty, \
     citation_dynasty_raw, citation_work_completed_by, citation_source_note \
     FROM commentary WHERE poem_id = ?1 ORDER BY id";

// ---------------------------------------------------------------- 装载

fn ensure_tag_declared(connection: &Connection, tag: &str) -> Result<()> {
    let mut statement = connection.prepare_cached(TAG_EXISTS_SQL)?;
    let declared = statement
        .query_row([tag], |row| row.get::<_, i64>(0))
        .map(|_| true)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other),
        })?;
    if declared {
        return Ok(());
    }
    let mut names_statement = connection.prepare_cached(TAG_NAMES_SQL)?;
    let names = names_statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total = names.len();
    let listed = names
        .iter()
        .take(MAX_LISTED_TAGS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("、");
    let suffix = if total > MAX_LISTED_TAGS {
        format!("（共 {total} 个，此处只列前 {MAX_LISTED_TAGS} 个）")
    } else {
        String::new()
    };
    Err(Error::Search(format!(
        "标签 `{tag}` 不在语料的策展词表里；现有标签：{listed}{suffix}。\
         标签在构建期由签入的词表产出，运行期不生成新标签"
    )))
}

fn load_poem(connection: &Connection, poem_id: &str) -> Result<PoemRecord> {
    let mut statement = connection.prepare_cached(POEM_DETAIL_SQL)?;
    statement
        .query_row([poem_id], map_poem)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                Error::Search(format!("语料里没有 stable_id 为 `{poem_id}` 的作品"))
            }
            other => Error::from(other),
        })
}

fn map_poem(row: &Row<'_>) -> rusqlite::Result<PoemRecord> {
    let last_chars_json: String = row.get(13)?;
    Ok(PoemRecord {
        stable_id: row.get(0)?,
        content_hash: row.get(1)?,
        title: row.get(2)?,
        title_raw: row.get(3)?,
        ci_tune: row.get::<_, Option<String>>(4)?.filter(|s| !s.is_empty()),
        author: row.get(5)?,
        dynasty: DynastyLabel {
            canonical: row.get(6)?,
            raw: row.get(7)?,
        },
        genre: row.get(8)?,
        body: row.get(9)?,
        body_original: row.get(10)?,
        script: row.get(11)?,
        first_line: row.get(12)?,
        last_chars: serde_json::from_str(&last_chars_json).unwrap_or_default(),
        line_count: row.get::<_, i64>(14)?.try_into().unwrap_or(u32::MAX),
        char_count: row.get::<_, i64>(15)?.try_into().unwrap_or(u32::MAX),
        work_group: row.get(16)?,
        edition_group: row.get(17)?,
    })
}

fn load_provenance(connection: &Connection, poem_id: &str) -> Result<Provenance> {
    let mut statement = connection.prepare_cached(PROVENANCE_SQL)?;
    Ok(statement.query_row([poem_id], |row| {
        Ok(Provenance {
            source_locator: row.get(0)?,
            source_locator_kind: row.get(1)?,
            source: row.get(2)?,
            revision: row.get(3)?,
            kind: row.get(4)?,
            license: row.get(5)?,
            license_class: row.get(6)?,
        })
    })?)
}

fn load_author(connection: &Connection, name: &str) -> Result<AuthorRecord> {
    let mut exists = connection.prepare_cached(AUTHOR_EXISTS_SQL)?;
    let canonical = exists
        .query_row([name], |row| row.get::<_, String>(0))
        .map_err(|error| match error {
            // `poem.author` 是 `author(name)` 的外键，所以缺行只可能是语料库被改坏。
            // 报出来而不是回填一个名字：静默回填会让一条已损坏的外键看起来正常。
            rusqlite::Error::QueryReturnedNoRows => Error::Corpus(format!(
                "作者 `{name}` 在 poem 表里被引用，但 author 表里没有对应行；语料库外键已损坏"
            )),
            other => Error::from(other),
        })?;
    let mut facets = connection.prepare_cached(AUTHOR_FACETS_SQL)?;
    let dynasties = facets
        .query_map([name], |row| {
            Ok(DynastyLabel {
                canonical: row.get(0)?,
                raw: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut count = connection.prepare_cached(AUTHOR_COUNT_SQL)?;
    let poem_count = count.query_row([name], |row| row.get::<_, i64>(0))?;
    Ok(AuthorRecord {
        name: canonical,
        dynasties,
        poem_count: usize::try_from(poem_count).unwrap_or(0),
    })
}

fn load_tags(connection: &Connection, poem_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare_cached(POEM_TAGS_SQL)?;
    let rows = statement.query_map([poem_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_rhyme_groups(connection: &Connection, poem_id: &str) -> Result<Vec<RhymeGroupMembership>> {
    let mut statement = connection.prepare_cached(POEM_RHYME_GROUPS_SQL)?;
    let rows = statement.query_map([poem_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut memberships = Vec::new();
    for row in rows {
        let (book_key, group, tone_key, confidence_key) = row?;
        // 三个键都由随包 schema 的 CHECK 约束限定取值。解析失败说明库里出现了 schema
        // 不允许的值，那是语料库损坏而不是一条可跳过的脏数据——跳过会让韵部凭空少一条。
        let book = RhymeBook::from_key(&book_key).ok_or_else(|| {
            Error::Corpus(format!("poem_rhyme_group 里出现未知韵书键 `{book_key}`"))
        })?;
        let tone = RhymeTone::from_key(&tone_key).ok_or_else(|| {
            Error::Corpus(format!("poem_rhyme_group 里出现未知声调键 `{tone_key}`"))
        })?;
        let confidence = RhymeConfidence::from_key(&confidence_key).ok_or_else(|| {
            Error::Corpus(format!(
                "poem_rhyme_group 里出现未知可信度键 `{confidence_key}`"
            ))
        })?;
        memberships.push(RhymeGroupMembership {
            book,
            group,
            tone,
            confidence,
        });
    }
    Ok(memberships)
}

fn load_commentaries(connection: &Connection, poem_id: &str) -> Result<Vec<CommentaryEntry>> {
    let mut statement = connection.prepare_cached(COMMENTARY_SQL)?;
    let rows = statement.query_map([poem_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, text, work, author, dynasty, dynasty_raw, completed_by, source_note) = row?;
        for (field, value) in [
            ("citation_work", work.as_str()),
            ("citation_author", author.as_str()),
            ("citation_dynasty", dynasty.as_str()),
            ("citation_source_note", source_note.as_str()),
            ("text", text.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(Error::CommentaryCitationMissing {
                    commentary_id: id,
                    poem_id: poem_id.to_owned(),
                    missing_field: field,
                });
            }
        }
        let work_completed_by =
            u16::try_from(completed_by).map_err(|_| Error::CommentaryCitationMissing {
                commentary_id: id.clone(),
                poem_id: poem_id.to_owned(),
                missing_field: "citation_work_completed_by",
            })?;
        entries.push(CommentaryEntry {
            id,
            text,
            citation: CommentaryCitation {
                work,
                author,
                dynasty: DynastyLabel {
                    canonical: dynasty,
                    raw: dynasty_raw,
                },
                work_completed_by,
                source_note,
            },
        });
    }
    Ok(entries)
}

/// 按韵书逐字反查平仄。
///
/// 逐字一次索引查找而不是一次大 `IN`：一首诗的正文是几十个字量级，而 `rhyme` 上的
/// `(rhyme_book, character)` 覆盖索引让每次查找都是常数级；换成拼 `IN` 列表会让 SQL
/// 长度随正文变化，那种语句进不了 `prepare_cached`，反而更慢。
fn annotate_tones(connection: &Connection, body: &str, book: RhymeBook) -> Result<ToneAnnotation> {
    book.ensure_available()?;
    let mut statement = connection.prepare_cached(CHARACTER_TONES_SQL)?;
    let mut lines = Vec::new();
    let mut unknown_count = 0;
    let mut either_count = 0;
    for (index, raw) in split_lines(body).into_iter().enumerate() {
        let mut cells = Vec::new();
        for character in crate::text::content_chars(&raw) {
            let buffer = character.to_string();
            let tones = statement
                .query_map(rusqlite::params![book.as_key(), &buffer], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut parsed = BTreeSet::new();
            let mut readings = Vec::new();
            for key in tones {
                let tone = RhymeTone::from_key(&key)
                    .ok_or_else(|| Error::Corpus(format!("rhyme 表里出现未知声调键 `{key}`")))?;
                parsed.insert(tone);
                readings.push(key);
            }
            readings.sort_unstable();
            readings.dedup();
            let tone = Tone::from_tones(&parsed);
            match tone {
                Tone::Unknown => unknown_count += 1,
                Tone::Either => either_count += 1,
                Tone::Level | Tone::Oblique => {}
            }
            cells.push(ToneCell {
                character: buffer,
                tone,
                readings,
            });
        }
        lines.push(ToneLine {
            line_index: u32::try_from(index).unwrap_or(u32::MAX),
            text: raw,
            cells,
        });
    }
    Ok(ToneAnnotation {
        book,
        lines,
        unknown_count,
        either_count,
    })
}

fn split_lines(body: &str) -> Vec<String> {
    crate::split_metrical_lines(body)
        .map(str::to_owned)
        .collect()
}

/// 单页上限，与元数据检索共用同一个值，避免调用方对两条路记两个数。
pub const TAG_PAGE_LIMIT: usize = META_PAGE_LIMIT;
