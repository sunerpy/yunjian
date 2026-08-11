//! 元数据检索：题目、作者、朝代、首句与尾字。
//!
//! # 为什么这个模块与全文检索是两条路
//!
//! 题目、作者、朝代、首句、尾字都有确定的边界，用户输入的就是完整的字段值或它的前缀，
//! 因此它们是**等值或范围查询**，由 `poem` 上的普通 B-tree 索引直接服务。全文检索那条
//! 路（`search::text`）要处理的是「正文里含这个子串」，形态完全不同。
//!
//! 两条路必须分开的具体理由：FTS5 trigram 分词器下长度不足 3 的子串匹配不到任何行
//! （见 [`crate::search::query`]）。「春望」是两个字，「霜」是一个字——把它们送进 FTS
//! 会稳定返回零行，而**这种坏法不会报错**。所以本模块的每一条 SQL 都不引用 `poem_fts`，
//! 并由模块测试里的 `EXPLAIN QUERY PLAN` 断言逐条钉住这一点。
//!
//! # 三种上游题目约定
//!
//! 上游语料的 `title` 有三种互不兼容的形态（见 `corpus/` 的入库实现
//! `ingest::werneror::resolve_title`），本模块用四条互斥的索引分支覆盖它们：
//!
//! | 形态 | 例 | 命中分支 |
//! |---|---|---|
//! | 裸题目 / 裸词牌 | `静夜思`、`望海潮` | [`MetaMatch::Title`]、[`MetaMatch::CiTune`] |
//! | 词牌·题目 | `念奴娇·赤壁怀古` | 查词牌走 [`MetaMatch::CiTune`] / [`MetaMatch::TitleHead`]，查题目走 [`MetaMatch::TitleTail`] |
//! | 题目 + 独立体裁/序号标记 | `菩萨蛮 五`、`古风 其一` | [`MetaMatch::TitleHead`] |
//!
//! **词牌那一半要对着词牌白名单校验。** 运行期的白名单就是语料自己的 `poem.ci_tune`
//! 列：入库时 `resolve_title` 只在题首命中上游 `cipai_2.txt`（1667 个词牌）时才写入
//! `ci_tune` 并把分隔符统一成 `·`，所以「`ci_tune` 里出现过的值」正是那份白名单在语料上
//! 的投影。据此校验既不需要运行期读上游文件，又天然只认真实存在的词牌，而且是一次
//! 走 `poem_ci_tune_idx` 的索引查找。
//!
//! # 两处刻意的「不归一」
//!
//! - **`dynasty_raw` 不被归一化掉。** 结果里同时给出规范键与上游原串
//!   （[`DynastyLabel`]）。上游有 28 个古典分桶，其中带跨朝代标签的（`唐末宋初`、
//!   `宋末元初`、`金末元初`……）都会归一到一个键，只留规范键就等于把上游的信息删掉。
//! - **归属冲突不被隐藏。** `work_group` 刻意不含作者（`blake3(去标点正文)`），正是为了
//!   让「同一正文挂在两个作者名下」可被检出。本模块把两个归属连同各自出处一起返回
//!   （[`AttributionConflict`]），而不是静默选一个。上游 `chinese-poetry` issue #232 的
//!   《赤壁》（杜牧 / 李商隐）就是真实案例。

use crate::search::query::plan_metadata_query;
use crate::{CorpusHandle, Error, QueryPlan, Result};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params_from_iter};
use serde::{Deserialize, Serialize};

/// 单次调用返回的命中上限。
///
/// 元数据查询同样会撞上大结果集：一个多产作者名下可能有上万首，`browse_by_dynasty("宋")`
/// 在发布规模上是几十万首。不设上限就会把调用方（尤其是 MCP 客户端的上下文）淹掉。
pub const META_PAGE_LIMIT: usize = 50;

/// 合成题目的分隔符。
///
/// `·`（U+00B7）是入库后的规范分隔符；半角与全角空格保留，因为上游有
/// `菩萨蛮 五` 这类**未**命中词牌白名单、因此没有被改写成 `·` 的空格形态。
pub const TITLE_SEPARATORS: [char; 3] = ['·', ' ', '\u{3000}'];

/// 词牌·题目的规范分隔符。
const CANONICAL_TITLE_SEPARATOR: char = '·';

/// 朝代的规范键与上游原串。
///
/// 两者都保留：规范键用于检索与分组，原串用于展示与溯源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynastyLabel {
    /// 归一后的朝代键，如 `唐`。
    pub canonical: String,
    /// 上游原样的朝代串，如 `唐末宋初`。
    pub raw: String,
}

/// 命中来自哪一列，以及是等值还是前缀。
///
/// 调用方靠它区分「精确命中」与「前缀命中」，从而能在界面上分开呈现，而不是把两种
/// 强度不同的命中混成一堆。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaMatch {
    /// `poem.title` 等值。
    Title,
    /// `poem.ci_tune` 等值：查询是一个词牌。
    CiTune,
    /// 合成题目的**前半段**，如查 `念奴娇` 命中 `念奴娇·赤壁怀古`。
    TitleHead,
    /// 合成题目的**后半段**，如查 `赤壁怀古` 命中 `念奴娇·赤壁怀古`。
    TitleTail,
    /// `poem.author` 等值。
    Author,
    /// `poem.author` 前缀：上游作者字段并不保证是干净姓名。
    AuthorPrefix,
    /// `poem.dynasty` 等值。
    Dynasty,
    /// `poem.first_line` 等值（整句首句）。
    FirstLine,
    /// `poem.first_line` 前缀。
    FirstLinePrefix,
    /// `poem_last_char.ch` 等值（逐句末字）。
    LastChar,
    /// `poem_tag.tag` 等值（策展主题标签）。
    Tag,
}

/// 一条元数据命中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaHit {
    /// 用户可见的唯一键。
    pub stable_id: String,
    /// 规范题目。
    pub title: String,
    /// 上游原样题目。
    pub title_raw: String,
    /// 词牌；诗为 `None`。
    pub ci_tune: Option<String>,
    /// 作者。**不保证是干净姓名**，见 [`find_by_author`]。
    pub author: String,
    /// 朝代的规范键与原串。
    pub dynasty: DynastyLabel,
    /// 首句（不含句末标点）。
    pub first_line: String,
    /// 不含作者的作品分组键，供调用方检出归属冲突。
    pub work_group: String,
    /// 体裁：`shi` / `ci` / `qu` / `fu` / `wen`。
    pub genre: String,
    /// 句数。
    pub line_count: u32,
    /// 正文字数。
    pub char_count: u32,
    /// 命中的是哪一列。
    pub matched_on: MetaMatch,
    /// 尾字检索命中的句序号（自 0 起）；其余检索为 `None`。
    pub matched_line_index: Option<u32>,
}

/// 一页元数据命中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaPage {
    /// 本页命中，条数不超过 [`META_PAGE_LIMIT`]。
    pub hits: Vec<MetaHit>,
    /// 续页游标；`None` 表示已到末页。
    pub next_cursor: Option<String>,
    /// 归一化之后真正用于查询的串，供调用方高亮与回显。
    pub normalized: String,
}

impl MetaPage {
    /// 归一化后为空时的空页。
    pub(super) fn empty(normalized: String) -> Self {
        Self {
            hits: Vec::new(),
            next_cursor: None,
            normalized,
        }
    }
}

/// 一个 `work_group` 内的一条归属。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// 该归属对应的记录。
    pub stable_id: String,
    /// 该记录声称的作者。
    pub author: String,
    /// 该记录声称的朝代。
    pub dynasty: DynastyLabel,
    /// 该记录的题目。
    pub title: String,
    /// 上游定位符——冲突要能落回到具体的上游位置才有意义。
    pub source_locator: String,
    /// 上游数据源名。
    pub provenance_source: String,
    /// 上游锁定的 revision。
    pub provenance_revision: String,
}

/// 同一正文挂在多个作者名下的归属冲突。
///
/// **它的存在本身就是需求。** 静默选一个作者会把上游的已知缺陷（`chinese-poetry`
/// issue #232 的《赤壁》双挂杜牧与李商隐）伪装成确定答案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributionConflict {
    /// 冲突所在的分组键。
    pub work_group: String,
    /// 全部归属，至少两条，作者互不相同。按 `stable_id` 排序，跨运行稳定。
    pub attributions: Vec<Attribution>,
}

impl AttributionConflict {
    /// 冲突涉及的作者名，去重且有序。
    #[must_use]
    pub fn authors(&self) -> Vec<&str> {
        let mut authors: Vec<&str> = self
            .attributions
            .iter()
            .map(|attribution| attribution.author.as_str())
            .collect();
        authors.sort_unstable();
        authors.dedup();
        authors
    }
}

/// 作者详情：作者记录 + 诗数 + 分页诗列表 + 归属冲突。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorDetail {
    /// 查询归一化后的作者名。
    pub name: String,
    /// 语料里实际命中的 `author` 值。
    ///
    /// 与 [`Self::name`] 分开是必要的：上游有作者字段混进现代小传的情形，入库时已截断到
    /// 姓名，但**不保证一条不漏**，所以前缀命中到的值可能仍带残留。把它原样列出来，
    /// 调用方才看得见自己拿到的是哪些串。
    pub matched_names: Vec<String>,
    /// 该作者名下出现过的朝代标签，按规范键 + 原串去重。
    pub dynasties: Vec<DynastyLabel>,
    /// 该作者名下的诗总数（不受分页影响）。
    pub poem_count: usize,
    /// 分页诗列表。
    pub page: MetaPage,
    /// 本页诗涉及的归属冲突。
    pub attribution_conflicts: Vec<AttributionConflict>,
}

// ---------------------------------------------------------------- 公开检索入口

/// 按题目检索，覆盖三种上游题目约定。
///
/// 四条**互斥**分支按可信度排序，越靠前越精确：
///
/// 1. [`MetaMatch::Title`]：`title` 等值。
/// 2. [`MetaMatch::CiTune`]：`ci_tune` 等值——这一条就是词牌白名单校验。
/// 3. [`MetaMatch::TitleHead`]：`title` 以「查询 + 分隔符」开头。
/// 4. [`MetaMatch::TitleTail`]：`title` 形如「白名单词牌 · 查询」。
///
/// 分支互斥是分页正确性的前提：若两条分支能命中同一首诗，它就可能在两页里各出现一次。
pub fn find_by_title(handle: &CorpusHandle, query: &str, cursor: Option<&str>) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, query)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    let connection = handle.connect()?;
    let mut branches = vec![
        Branch {
            rank: 0,
            sql: title_exact_sql(),
            binds: vec![text(&normalized)],
            matched_on: MetaMatch::Title,
        },
        Branch {
            rank: 1,
            sql: ci_tune_exact_sql(),
            binds: vec![text(&normalized), text(&normalized)],
            matched_on: MetaMatch::CiTune,
        },
    ];
    for separator in TITLE_SEPARATORS {
        let lower = format!("{normalized}{separator}");
        // 上界取不到才说明前缀区间是开区间：`念奴娇·` 的上界是 `念奴娇` 后接
        // `·` 的下一个码位，于是区间恰好覆盖「以 `念奴娇·` 开头」的全部题目。
        let Some(upper) = prefix_upper_bound(&lower) else {
            continue;
        };
        branches.push(Branch {
            rank: 2,
            sql: title_head_sql(),
            binds: vec![text(&lower), text(&upper), text(&normalized)],
            matched_on: MetaMatch::TitleHead,
        });
    }
    branches.push(Branch {
        rank: 3,
        sql: title_tail_sql(),
        binds: vec![text(&normalized)],
        matched_on: MetaMatch::TitleTail,
    });
    run_branches(&connection, branches, cursor, normalized)
}

/// 按作者检索。
///
/// 两条互斥分支：等值优先，其后是前缀。
///
/// **前缀分支不是便利功能，而是对上游脏数据的容错。** 上游有些 `author` 字段装的是
/// 现代小传（`王筠（1784-1854），字貫山……`），入库时按首个全角左括号截断到姓名，
/// 但判据是「文本是否前现代」而不是字段名，因此**不能假定库里的 `author` 一定是纯
/// 姓名**。前缀区间让 `王筠` 仍能命中残留了后缀的记录，且照样走 `poem_author_idx`。
pub fn find_by_author(
    handle: &CorpusHandle,
    query: &str,
    cursor: Option<&str>,
) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, query)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    let connection = handle.connect()?;
    run_branches(
        &connection,
        author_branches(&normalized),
        cursor,
        normalized,
    )
}

/// 作者详情：作者记录、诗数、分页诗列表，以及本页涉及的归属冲突。
pub fn author_detail(
    handle: &CorpusHandle,
    query: &str,
    cursor: Option<&str>,
) -> Result<AuthorDetail> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, query)? else {
        return Ok(AuthorDetail {
            name: String::new(),
            matched_names: Vec::new(),
            dynasties: Vec::new(),
            poem_count: 0,
            page: MetaPage::empty(String::new()),
            attribution_conflicts: Vec::new(),
        });
    };
    let connection = handle.connect()?;
    let page = run_branches(
        &connection,
        author_branches(&normalized),
        cursor,
        normalized.clone(),
    )?;

    let upper = prefix_upper_bound(&normalized);
    let poem_count = author_poem_count(&connection, &normalized, upper.as_deref())?;
    let (matched_names, dynasties) = author_facets(&connection, &normalized, upper.as_deref())?;
    let attribution_conflicts = conflicts_for(&connection, &page)?;

    Ok(AuthorDetail {
        name: normalized,
        matched_names,
        dynasties,
        poem_count,
        page,
        attribution_conflicts,
    })
}

/// 按朝代的规范键浏览。
///
/// 键不在语料里时返回 [`Error::Search`] 并列出现有键，而不是一个空列表——空列表分不清
/// 「这个朝代没有诗」与「根本没有这个键」，缺数据会被当成否定答案呈现。
pub fn browse_by_dynasty(
    handle: &CorpusHandle,
    dynasty: &str,
    cursor: Option<&str>,
) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, dynasty)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    let connection = handle.connect()?;
    let available = dynasty_keys(&connection)?;
    if !available.iter().any(|key| key == &normalized) {
        return Err(Error::Search(format!(
            "朝代键 {normalized} 不在语料中；语料现有 {}",
            available.join("、")
        )));
    }
    run_branches(
        &connection,
        vec![Branch {
            rank: 0,
            sql: dynasty_sql(),
            binds: vec![text(&normalized)],
            matched_on: MetaMatch::Dynasty,
        }],
        cursor,
        normalized,
    )
}

/// 按首句前缀检索预计算的 `first_line` 列。
///
/// 用半开区间 `first_line >= 前缀 AND first_line < 上界` 而**不是** `LIKE '前缀%'`：
/// 后者在某些 collation 下会被优化器放弃索引而退化成整列扫描，而半开区间是纯 B-tree
/// 范围查找，与 collation 无关。
pub fn find_by_first_line(
    handle: &CorpusHandle,
    prefix: &str,
    cursor: Option<&str>,
) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, prefix)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    let connection = handle.connect()?;
    let mut branches = vec![Branch {
        rank: 0,
        sql: first_line_exact_sql(),
        binds: vec![text(&normalized)],
        matched_on: MetaMatch::FirstLine,
    }];
    if let Some(upper) = prefix_upper_bound(&normalized) {
        branches.push(Branch {
            rank: 1,
            sql: first_line_prefix_sql(),
            binds: vec![text(&normalized), text(&upper), text(&normalized)],
            matched_on: MetaMatch::FirstLinePrefix,
        });
    }
    run_branches(&connection, branches, cursor, normalized)
}

/// 按句末字检索。
///
/// 走首启派生的 `poem_last_char(poem_id, line_index, ch)` 表及其 `(ch, poem_id)` 索引，
/// **不解 `poem.last_chars` 的 JSON**：那是一个 JSON 文本列，建不了索引，用
/// `LIKE '%字%'` 查它实测会 `SCAN poem`（1 万首上 p95 1.6 ms，外推到发布规模约
/// 137 ms，逼近预算），而规范化多对多表加覆盖索引后降到 0.1 ms 以下。
///
/// 结果单位是「诗」而不是「句」：律诗多句可以押同一个字，因此按 `poem_id` 聚合并取
/// 最小句序号。
pub fn find_by_last_char(
    handle: &CorpusHandle,
    character: &str,
    cursor: Option<&str>,
) -> Result<MetaPage> {
    let QueryPlan::Meta { normalized } = plan_metadata_query(handle, character)? else {
        return Ok(MetaPage::empty(String::new()));
    };
    if normalized.chars().count() != 1 {
        return Err(Error::Search(format!(
            "尾字检索只接受单个字，实际收到 {} 个字（{normalized}）",
            normalized.chars().count()
        )));
    }
    let connection = handle.connect()?;
    run_branches(
        &connection,
        vec![Branch {
            rank: 0,
            sql: last_char_sql(),
            binds: vec![text(&normalized)],
            matched_on: MetaMatch::LastChar,
        }],
        cursor,
        normalized,
    )
}

/// 列出一个 `work_group` 内的全部归属。
///
/// 供调用方在拿到 [`MetaHit::work_group`] 之后展开冲突；[`author_detail`] 内部用的
/// 也是这一条。
pub fn find_work_group_attributions(
    handle: &CorpusHandle,
    work_group: &str,
) -> Result<Vec<Attribution>> {
    let connection = handle.connect()?;
    attributions_of(&connection, work_group)
}

// ---------------------------------------------------------------- SQL

/// 全部题目/作者/朝代/首句分支共用的投影。
///
/// 十二列的顺序被 [`map_hit`] 依赖；改动要两处同步。
pub(super) const POEM_COLUMNS: &str = "p.stable_id, p.title, p.title_raw, p.ci_tune, p.author, \
p.dynasty, p.dynasty_raw, p.first_line, p.work_group, p.genre, p.line_count, p.char_count";

/// `title` 等值。
fn title_exact_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.title = ?1 AND p.stable_id > ?2 ORDER BY p.stable_id LIMIT ?3"
    )
}

/// `ci_tune` 等值。`title <> ?2` 把它与 [`title_exact_sql`] 的结果集切开。
fn ci_tune_exact_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.ci_tune = ?1 AND p.title <> ?2 AND p.stable_id > ?3 \
         ORDER BY p.stable_id LIMIT ?4"
    )
}

/// 合成题目的前半段：`title` 落在 `[查询+分隔符, 上界)` 区间内。
///
/// `ci_tune <> ?3` 把已经由 [`ci_tune_exact_sql`] 覆盖的词作排除，保持分支互斥。
fn title_head_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.title >= ?1 AND p.title < ?2 \
         AND (p.ci_tune IS NULL OR p.ci_tune <> ?3) \
         AND p.stable_id > ?4 ORDER BY p.stable_id LIMIT ?5"
    )
}

/// 合成题目的后半段：`title` 等于「白名单词牌 · 查询」。
///
/// 子查询就是词牌白名单在语料上的投影，走 `poem_ci_tune_idx` 覆盖索引；外层对拼出的
/// 完整题目做 `poem_title_idx` 等值查找。两步都是 B-tree，没有一步扫基表。
fn title_tail_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM \
         (SELECT DISTINCT ci_tune AS t FROM poem \
          WHERE ci_tune IS NOT NULL AND ci_tune <> '') AS tunes \
         JOIN poem AS p ON p.title = tunes.t || '{CANONICAL_TITLE_SEPARATOR}' || ?1 \
         WHERE (p.ci_tune IS NULL OR p.ci_tune <> ?1) AND p.stable_id > ?2 \
         ORDER BY p.stable_id LIMIT ?3"
    )
}

/// `author` 等值。
fn author_exact_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.author = ?1 AND p.stable_id > ?2 ORDER BY p.stable_id LIMIT ?3"
    )
}

/// `author` 前缀区间，排除等值部分。
fn author_prefix_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.author >= ?1 AND p.author < ?2 AND p.author <> ?3 \
         AND p.stable_id > ?4 ORDER BY p.stable_id LIMIT ?5"
    )
}

/// `dynasty` 等值。
fn dynasty_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.dynasty = ?1 AND p.stable_id > ?2 ORDER BY p.stable_id LIMIT ?3"
    )
}

/// `first_line` 等值。
fn first_line_exact_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.first_line = ?1 AND p.stable_id > ?2 ORDER BY p.stable_id LIMIT ?3"
    )
}

/// `first_line` 前缀区间，排除等值部分。
fn first_line_prefix_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS} FROM poem AS p \
         WHERE p.first_line >= ?1 AND p.first_line < ?2 AND p.first_line <> ?3 \
         AND p.stable_id > ?4 ORDER BY p.stable_id LIMIT ?5"
    )
}

/// 尾字：驱动表是 `poem_last_char`，按 `poem_id` 聚合成「诗」。
fn last_char_sql() -> String {
    format!(
        "SELECT {POEM_COLUMNS}, MIN(l.line_index) AS line_index \
         FROM poem_last_char AS l JOIN poem AS p ON p.stable_id = l.poem_id \
         WHERE l.ch = ?1 AND l.poem_id > ?2 \
         GROUP BY l.poem_id ORDER BY l.poem_id LIMIT ?3"
    )
}

/// 一个 `work_group` 内的全部归属。
fn work_group_sql() -> String {
    "SELECT p.stable_id, p.author, p.dynasty, p.dynasty_raw, p.title, p.source_locator, \
     p.provenance_source, p.provenance_revision \
     FROM poem AS p WHERE p.work_group = ?1 ORDER BY p.stable_id"
        .to_owned()
}

/// 作者名下的诗数。
fn author_count_sql() -> String {
    "SELECT COUNT(*) FROM poem AS p WHERE p.author >= ?1 AND p.author < ?2".to_owned()
}

/// 作者名下的 `author` / `dynasty` / `dynasty_raw` 取值。
fn author_facets_sql() -> String {
    "SELECT DISTINCT p.author, p.dynasty, p.dynasty_raw FROM poem AS p \
     WHERE p.author >= ?1 AND p.author < ?2 ORDER BY p.author, p.dynasty, p.dynasty_raw"
        .to_owned()
}

/// 语料里出现过的朝代规范键。
fn dynasty_keys_sql() -> String {
    "SELECT DISTINCT dynasty FROM poem ORDER BY dynasty".to_owned()
}

// ---------------------------------------------------------------- 分支执行

/// 一条物理分支：一段 SQL、它自己的绑定值，以及命中语义。
pub(super) struct Branch {
    pub(super) rank: u8,
    pub(super) sql: String,
    pub(super) binds: Vec<Value>,
    pub(super) matched_on: MetaMatch,
}

/// 分页游标：`rank:stable_id`。
///
/// 必须带 `rank`：题目与作者检索有多条分支，只记 `stable_id` 无法表达「第 2 条分支
/// 读到哪儿了」，续页会从错误的分支重新开始。
#[derive(Debug, Clone, PartialEq, Eq)]
struct MetaCursor {
    rank: u8,
    stable_id: String,
}

impl MetaCursor {
    fn parse(raw: &str) -> Result<Self> {
        let (rank, stable_id) = raw
            .split_once(':')
            .ok_or_else(|| Error::Search(format!("游标格式非法：{raw}")))?;
        let rank = rank
            .parse::<u8>()
            .map_err(|_| Error::Search(format!("游标的分支序号非法：{raw}")))?;
        if stable_id.is_empty() {
            return Err(Error::Search(format!("游标缺少 stable_id：{raw}")));
        }
        Ok(Self {
            rank,
            stable_id: stable_id.to_owned(),
        })
    }

    fn encode(&self) -> String {
        format!("{}:{}", self.rank, self.stable_id)
    }
}

fn author_branches(normalized: &str) -> Vec<Branch> {
    let mut branches = vec![Branch {
        rank: 0,
        sql: author_exact_sql(),
        binds: vec![text(normalized)],
        matched_on: MetaMatch::Author,
    }];
    if let Some(upper) = prefix_upper_bound(normalized) {
        branches.push(Branch {
            rank: 1,
            sql: author_prefix_sql(),
            binds: vec![text(normalized), text(&upper), text(normalized)],
            matched_on: MetaMatch::AuthorPrefix,
        });
    }
    branches
}

/// 依次执行分支，收满一页即停，并给出续页游标。
pub(super) fn run_branches(
    connection: &Connection,
    branches: Vec<Branch>,
    cursor: Option<&str>,
    normalized: String,
) -> Result<MetaPage> {
    let cursor = cursor.map(MetaCursor::parse).transpose()?;
    let mut collected: Vec<(u8, MetaHit)> = Vec::new();
    for branch in branches {
        if collected.len() > META_PAGE_LIMIT {
            break;
        }
        if cursor.as_ref().is_some_and(|c| branch.rank < c.rank) {
            continue;
        }
        let resume = cursor
            .as_ref()
            .filter(|c| c.rank == branch.rank)
            .map_or(String::new(), |c| c.stable_id.clone());
        // 多取一条用来判断「还有下一页」，而不是再发一次 COUNT。
        let remaining = META_PAGE_LIMIT + 1 - collected.len();
        let mut binds = branch.binds;
        binds.push(Value::Text(resume));
        binds.push(Value::Integer(
            i64::try_from(remaining).unwrap_or(i64::from(u16::MAX)),
        ));
        let mut statement = connection.prepare_cached(&branch.sql)?;
        let rows = statement.query_map(params_from_iter(binds), |row| {
            map_hit(row, branch.matched_on)
        })?;
        for hit in rows {
            collected.push((branch.rank, hit?));
        }
    }

    let next_cursor = if collected.len() > META_PAGE_LIMIT {
        collected.truncate(META_PAGE_LIMIT);
        collected.last().map(|(rank, hit)| {
            MetaCursor {
                rank: *rank,
                stable_id: hit.stable_id.clone(),
            }
            .encode()
        })
    } else {
        None
    };

    Ok(MetaPage {
        hits: collected.into_iter().map(|(_, hit)| hit).collect(),
        next_cursor,
        normalized,
    })
}

fn map_hit(row: &Row<'_>, matched_on: MetaMatch) -> rusqlite::Result<MetaHit> {
    Ok(MetaHit {
        stable_id: row.get(0)?,
        title: row.get(1)?,
        title_raw: row.get(2)?,
        ci_tune: row.get::<_, Option<String>>(3)?.filter(|s| !s.is_empty()),
        author: row.get(4)?,
        dynasty: DynastyLabel {
            canonical: row.get(5)?,
            raw: row.get(6)?,
        },
        first_line: row.get(7)?,
        work_group: row.get(8)?,
        genre: row.get(9)?,
        line_count: row.get::<_, i64>(10)?.try_into().unwrap_or(u32::MAX),
        char_count: row.get::<_, i64>(11)?.try_into().unwrap_or(u32::MAX),
        matched_on,
        matched_line_index: match matched_on {
            MetaMatch::LastChar => row.get::<_, i64>(12)?.try_into().ok(),
            _ => None,
        },
    })
}

// ---------------------------------------------------------------- 辅助查询

fn attributions_of(connection: &Connection, work_group: &str) -> Result<Vec<Attribution>> {
    let mut statement = connection.prepare_cached(&work_group_sql())?;
    let rows = statement.query_map([work_group], |row| {
        Ok(Attribution {
            stable_id: row.get(0)?,
            author: row.get(1)?,
            dynasty: DynastyLabel {
                canonical: row.get(2)?,
                raw: row.get(3)?,
            },
            title: row.get(4)?,
            source_locator: row.get(5)?,
            provenance_source: row.get(6)?,
            provenance_revision: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 本页诗涉及的归属冲突，按 `work_group` 去重且有序。
fn conflicts_for(connection: &Connection, page: &MetaPage) -> Result<Vec<AttributionConflict>> {
    let mut groups: Vec<&str> = page
        .hits
        .iter()
        .map(|hit| hit.work_group.as_str())
        .collect();
    groups.sort_unstable();
    groups.dedup();
    let mut conflicts = Vec::new();
    for group in groups {
        let attributions = attributions_of(connection, group)?;
        let mut authors: Vec<&str> = attributions
            .iter()
            .map(|attribution| attribution.author.as_str())
            .collect();
        authors.sort_unstable();
        authors.dedup();
        if authors.len() > 1 {
            conflicts.push(AttributionConflict {
                work_group: group.to_owned(),
                attributions,
            });
        }
    }
    Ok(conflicts)
}

fn author_poem_count(
    connection: &Connection,
    normalized: &str,
    upper: Option<&str>,
) -> Result<usize> {
    let Some(upper) = upper else {
        // 没有上界说明前缀已到码位顶端，只能退回等值计数；这不是退化，是那种前缀
        // 本身就只可能等于自己。
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM poem AS p WHERE p.author = ?1",
            [normalized],
            |row| row.get(0),
        )?;
        return Ok(usize::try_from(count).unwrap_or(0));
    };
    let count: i64 =
        connection.query_row(&author_count_sql(), [normalized, upper], |row| row.get(0))?;
    Ok(usize::try_from(count).unwrap_or(0))
}

type AuthorFacets = (Vec<String>, Vec<DynastyLabel>);

fn author_facets(
    connection: &Connection,
    normalized: &str,
    upper: Option<&str>,
) -> Result<AuthorFacets> {
    let upper = match upper {
        Some(upper) => upper.to_owned(),
        None => return Ok((vec![normalized.to_owned()], Vec::new())),
    };
    let mut statement = connection.prepare_cached(&author_facets_sql())?;
    let rows = statement.query_map([normalized, upper.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            DynastyLabel {
                canonical: row.get(1)?,
                raw: row.get(2)?,
            },
        ))
    })?;
    let mut names = Vec::new();
    let mut dynasties = Vec::new();
    for row in rows {
        let (name, dynasty) = row?;
        if !names.contains(&name) {
            names.push(name);
        }
        if !dynasties.contains(&dynasty) {
            dynasties.push(dynasty);
        }
    }
    Ok((names, dynasties))
}

fn dynasty_keys(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare_cached(&dynasty_keys_sql())?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// 前缀区间的开上界。
///
/// 把末字换成「下一个码位」，于是 `[前缀, 上界)` 恰好等于「以该前缀开头」。跳过 UTF-16
/// 代理区（`char` 不允许那个区间），末字已是 `char::MAX` 时退一位继续抬——整串都到顶
/// 才返回 `None`，此时调用方只用等值分支。
fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut characters: Vec<char> = prefix.chars().collect();
    while let Some(last) = characters.pop() {
        if let Some(next) = next_char(last) {
            characters.push(next);
            return Some(characters.into_iter().collect());
        }
    }
    None
}

fn next_char(character: char) -> Option<char> {
    let mut code = u32::from(character).checked_add(1)?;
    if code == 0xD800 {
        code = 0xE000;
    }
    char::from_u32(code)
}

#[cfg(test)]
mod tests;
