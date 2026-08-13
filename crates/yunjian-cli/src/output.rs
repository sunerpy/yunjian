//! 两种输出形态的唯一数据源。
//!
//! 人类可读文本与 `--json` 信封里的载荷由**同一个**结构体产出：前者是它的
//! [`Renderable::render`]，后者是它的 `Serialize`。两条路各写一遍的话，改了一处忘了另一处
//! 的结果是「界面上有的字段 JSON 里没有」，而这种漂移只有用户会发现。
//!
//! 已经在 `yunjian-core` 里定义好且可序列化的类型（[`PoemDetail`]、[`AuthorDetail`]、
//! [`RhymeGroupMatches`]）不在这里重新包一层：JSON 直接序列化核心类型，本模块只补它们的
//! 人类可读渲染。信封因此与核心 API 同形，MCP 工具（todo 32）与桌面端能复用同一份契约。

use serde::Serialize;
use yunjian_core::{
    AuthorDetail, CorpusHandle, CorpusMeta, CorpusOrigin, DerivedState, GradingConfig,
    HighlightedSnippet, MetaHit, PoemDetail, RhymeGroupMatches, RhymeGroupMembership, SearchPage,
    TextSearchHit,
};
use yunjian_recite::{
    AlignOp, FsrsGrade, OpsSummary, ReviewState, SubstitutionClass, TypedScore,
    classify_substitution,
};

/// 能渲染成若干行人类可读文本的输出。
pub trait Renderable {
    /// 渲染成逐行文本。每一行由 [`crate::present::line`] 写往 stdout。
    fn render(&self) -> Vec<String>;
}

/// `ai cache purge` 的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AiCachePurgeOut {
    /// 被清理的范围。
    pub scope: String,
    /// 删除的用户缓存行数。
    pub removed: usize,
    /// 缓存数据库路径。
    pub database: String,
}

impl Renderable for AiCachePurgeOut {
    fn render(&self) -> Vec<String> {
        vec![format!(
            "已从 {} 清理 {} 条用户赏析缓存（范围：{}）；内置赏析未改动",
            self.database, self.removed, self.scope
        )]
    }
}

/// `search` 的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchOut {
    /// 用户给出的原始查询。
    pub query: String,
    /// 请求的单页命中上限。
    pub limit: usize,
    /// 实际执行的物理查询计划名，取值与黄金查询契约一致。
    pub plan: String,
    /// 当前查询的总命中估计数。**过滤前**的数，因为过滤只作用于本页。
    pub total_estimate: usize,
    /// 续页游标；`None` 表示已到末页。
    pub next_cursor: Option<String>,
    /// 本次施加的过滤与标注选项。
    pub filters: SearchFilters,
    /// 过滤后的本页命中。
    pub hits: Vec<SearchHit>,
}

/// `search` 的过滤与标注选项，原样回显以便调用方核对自己发的是什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchFilters {
    /// `--author`。
    pub author: Option<String>,
    /// `--dynasty`。
    pub dynasty: Option<String>,
    /// `--rhyme-book`。
    pub rhyme_book: Option<String>,
}

impl SearchFilters {
    /// 有没有任何过滤条件（标注不算过滤）。
    #[must_use]
    pub const fn filters_hits(&self) -> bool {
        self.author.is_some() || self.dynasty.is_some()
    }
}

/// 一条命中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 题目。
    pub title: String,
    /// 作者。
    pub author: String,
    /// 朝代规范键。
    pub dynasty: String,
    /// 最佳命中句的零基序号。
    pub matched_line_index: usize,
    /// 命中句及字符级高亮范围。
    pub snippet: HighlightedSnippet,
    /// `--rhyme-book` 给出时该书下的韵部归属。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhyme_groups: Option<Vec<RhymeGroupMembership>>,
}

impl SearchHit {
    /// 由核心命中与可选韵部标注组装。
    #[must_use]
    pub fn new(hit: TextSearchHit, rhyme_groups: Option<Vec<RhymeGroupMembership>>) -> Self {
        Self {
            poem_id: hit.poem_id,
            title: hit.title,
            author: hit.author,
            dynasty: hit.dynasty,
            matched_line_index: hit.matched_line_index,
            snippet: hit.snippet,
            rhyme_groups,
        }
    }
}

impl SearchOut {
    /// 由一页检索结果与选项组装。
    #[must_use]
    pub fn new(
        query: String,
        limit: usize,
        page: &SearchPage,
        filters: SearchFilters,
        hits: Vec<SearchHit>,
    ) -> Self {
        Self {
            query,
            limit,
            plan: page.plan_used.contract_name().to_owned(),
            total_estimate: page.total_estimate,
            next_cursor: page.next_cursor.clone(),
            filters,
            hits,
        }
    }
}

impl Renderable for SearchOut {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "「{}」· 计划 {} · 估计命中 {} · 本页 {}",
            self.query,
            self.plan,
            self.total_estimate,
            self.hits.len()
        )];
        for (index, hit) in self.hits.iter().enumerate() {
            lines.push(format!(
                "{:>2}. {} — {}（{}）  [{}]",
                index + 1,
                hit.title,
                hit.author,
                hit.dynasty,
                hit.poem_id
            ));
            lines.push(format!(
                "    第 {} 句：{}",
                hit.matched_line_index + 1,
                hit.snippet.text
            ));
            if let Some(groups) = &hit.rhyme_groups {
                lines.push(format!("    韵部：{}", render_memberships(groups)));
            }
        }
        if let Some(cursor) = &self.next_cursor {
            lines.push(format!("续页：--cursor {cursor}"));
        }
        lines
    }
}

/// `corpus status` 与 `corpus fetch` 共用的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorpusOut {
    /// 语料库文件路径。
    pub path: String,
    /// 语料数据版本。
    pub corpus_version: String,
    /// schema 版本。
    pub schema_version: u32,
    /// 构建时间。
    pub built_at: String,
    /// 作品总数。
    pub poem_count: i64,
    /// FTS5 `detail` 模式。
    pub index_detail_mode: String,
    /// 派生索引的分发策略。
    pub derived_indexes: String,
    /// 随包语料范围。
    pub shipped_scope: String,
    /// 本次是从哪一级解析出来的。
    pub origin: String,
    /// 本次运行是否真的落地了一份语料。
    pub materialized: bool,
    /// 派生结构是否就绪。
    pub derived_ready: bool,
    /// 派生结构不可用的原因；就绪时为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_reason: Option<String>,
}

impl CorpusOut {
    /// 由已就绪的语料库句柄读出全部状态。
    #[must_use]
    pub fn new(handle: &CorpusHandle, materialized: bool) -> Self {
        let CorpusMeta {
            schema_version,
            corpus_version,
            built_at,
            poem_count,
            index_detail_mode,
            derived_indexes,
            shipped_scope,
        } = handle.meta().clone();
        let (derived_ready, derived_reason) = match handle.derived() {
            DerivedState::Ready { .. } => (true, None),
            DerivedState::Unavailable { reason } => (false, Some(reason.clone())),
        };
        Self {
            path: handle.path().display().to_string(),
            corpus_version,
            schema_version,
            built_at,
            poem_count,
            index_detail_mode,
            derived_indexes,
            shipped_scope,
            origin: origin_key(handle.origin()).to_owned(),
            materialized,
            derived_ready,
            derived_reason,
        }
    }
}

impl Renderable for CorpusOut {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("语料库：{}", self.path),
            format!(
                "数据版本：{} · schema {} · {} 首 · 构建于 {}",
                self.corpus_version, self.schema_version, self.poem_count, self.built_at
            ),
            format!(
                "索引形态：detail={} · 派生分发={} · 随包范围={}",
                self.index_detail_mode, self.derived_indexes, self.shipped_scope
            ),
            format!(
                "来源：{}{}",
                origin_label(&self.origin),
                if self.materialized {
                    "（本次运行落地）"
                } else {
                    ""
                }
            ),
        ];
        lines.push(match &self.derived_reason {
            None => "派生结构：就绪，一至两字查询走候选表".to_owned(),
            Some(reason) => format!("派生结构：不可用（{reason}）；一至两字查询本次退化为全表扫描"),
        });
        lines
    }
}

impl Renderable for PoemDetail {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "{} — {}（{}）  [{}]",
                self.poem.title, self.poem.author, self.poem.dynasty.raw, self.poem.stable_id
            ),
            self.poem.body.clone(),
        ];
        if let Some(tune) = &self.poem.ci_tune {
            lines.push(format!("词牌：{tune}"));
        }
        lines.push(format!(
            "平仄（{}）：{}",
            self.tones.book.display_name(),
            self.tones.display()
        ));
        if self.tones.has_unknown() {
            lines.push(format!(
                "    其中 {} 字韵书未收，以 ？ 标出——未知不是平声",
                self.tones.unknown_count
            ));
        }
        if !self.rhyme_groups.is_empty() {
            lines.push(format!("韵部：{}", render_memberships(&self.rhyme_groups)));
        }
        if !self.tags.is_empty() {
            lines.push(format!("标签：{}", self.tags.join("、")));
        }
        lines.push(format!(
            "出处：{}@{} · {} · {}（{}）",
            self.provenance.source,
            self.provenance.revision,
            self.provenance.kind,
            self.provenance.license,
            self.provenance.license_class
        ));
        if let Some(conflict) = &self.attribution_conflict {
            lines.push(format!(
                "归属冲突：同一正文（{}）另挂 {}",
                conflict.work_group,
                conflict.authors().join(" / ")
            ));
        }
        if self.commentaries.is_empty() {
            lines.push("历代集评：无".to_owned());
        } else {
            lines.push(format!("历代集评 {} 条：", self.commentaries.len()));
            for (index, entry) in self.commentaries.iter().enumerate() {
                lines.push(format!("{:>2}. {}", index + 1, entry.text));
                lines.push(format!(
                    "    —— {} {}《{}》（{}）{}",
                    entry.citation.dynasty.raw,
                    entry.citation.author,
                    entry.citation.work,
                    entry.citation.work_completed_by,
                    entry.citation.source_note
                ));
            }
        }
        lines
    }
}

impl Renderable for AuthorDetail {
    fn render(&self) -> Vec<String> {
        let dynasties = self
            .dynasties
            .iter()
            .map(|label| label.raw.as_str())
            .collect::<Vec<_>>()
            .join("、");
        let mut lines = vec![format!(
            "{}（{}）· 共 {} 首 · 本页 {}",
            self.name,
            if dynasties.is_empty() {
                "朝代未知"
            } else {
                dynasties.as_str()
            },
            self.poem_count,
            self.page.hits.len()
        )];
        if self.matched_names.len() > 1
            || self
                .matched_names
                .first()
                .is_some_and(|name| name != &self.name)
        {
            lines.push(format!("命中的作者串：{}", self.matched_names.join(" / ")));
        }
        lines.extend(render_meta_hits(&self.page.hits));
        for conflict in &self.attribution_conflicts {
            lines.push(format!(
                "归属冲突：{} → {}",
                conflict.work_group,
                conflict.authors().join(" / ")
            ));
        }
        if let Some(cursor) = &self.page.next_cursor {
            lines.push(format!("续页：--cursor {cursor}"));
        }
        lines
    }
}

impl Renderable for RhymeGroupMatches {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} {} · 命中 {} 首 · 未消歧 {} 首",
            self.book.display_name(),
            self.rhyme_group,
            self.hits.len(),
            self.unresolved.len()
        )];
        for (index, hit) in self.hits.iter().enumerate() {
            lines.push(format!(
                "{:>2}. {} — {}（{} {}，{}）  [{}]",
                index + 1,
                hit.title,
                hit.author,
                hit.rhyme_group,
                hit.tone.display_name(),
                hit.confidence.display_name(),
                hit.poem_id
            ));
        }
        if !self.unresolved.is_empty() {
            // 未消歧刻意单列：并进命中就是把猜测报成判断。
            lines.push("以下作品的韵脚未能唯一消歧，不计入命中：".to_owned());
            for hit in &self.unresolved {
                lines.push(format!(
                    "    {} — {}  [{}]",
                    hit.title, hit.author, hit.poem_id
                ));
            }
        }
        lines
    }
}

/// 按 id 查不到作品时的载荷。
///
/// 它是一条**空结果**而不是一条错误：`show` 一个不存在的 `stable_id` 与 `search` 一个查不到
/// 的词是同一类结局（退出 1），而「语料坏了」是另一类（退出 3）。把两者混成一个码会让脚本
/// 无法区分「这首诗不在库里」和「库打不开」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotFound {
    /// 被查询的稳定标识。
    pub poem_id: String,
    /// 恒为 `false`，供调用方不必解析中文即可判断。
    pub found: bool,
    /// 中文说明。
    pub message: String,
}

impl NotFound {
    /// 组一条「查不到」。
    #[must_use]
    pub fn new(poem_id: impl Into<String>) -> Self {
        let poem_id = poem_id.into();
        Self {
            message: format!("语料里没有 stable_id 为 `{poem_id}` 的作品"),
            poem_id,
            found: false,
        }
    }
}

impl Renderable for NotFound {
    fn render(&self) -> Vec<String> {
        vec![self.message.clone()]
    }
}

fn render_meta_hits(hits: &[MetaHit]) -> Vec<String> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            format!(
                "{:>2}. {} — {}（{}）  [{}]",
                index + 1,
                hit.title,
                hit.author,
                hit.dynasty.raw,
                hit.stable_id
            )
        })
        .collect()
}

fn render_memberships(groups: &[RhymeGroupMembership]) -> String {
    if groups.is_empty() {
        return "无归属".to_owned();
    }
    groups
        .iter()
        .map(|group| {
            format!(
                "{} {}（{}，{}）",
                group.book.display_name(),
                group.group,
                group.tone.display_name(),
                group.confidence.display_name()
            )
        })
        .collect::<Vec<_>>()
        .join("；")
}

fn origin_key(origin: &CorpusOrigin) -> &'static str {
    match origin {
        CorpusOrigin::Configured => "configured",
        CorpusOrigin::Materialized => "materialized",
        CorpusOrigin::JustMaterialized { .. } => "just_materialized",
    }
}

fn origin_label(key: &str) -> &str {
    match key {
        "configured" => "配置指定",
        "materialized" => "已落地副本",
        "just_materialized" => "本次从归档落地",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{CorpusOut, Renderable, SearchFilters, SearchHit, SearchOut};
    use yunjian_core::{HighlightRange, HighlightedSnippet, QueryPlan, SearchPage, TextSearchHit};

    fn page(plan: QueryPlan, next_cursor: Option<&str>) -> SearchPage {
        SearchPage {
            hits: Vec::new(),
            total_estimate: 7,
            next_cursor: next_cursor.map(str::to_owned),
            plan_used: plan,
        }
    }

    fn hit() -> SearchHit {
        SearchHit::new(
            TextSearchHit {
                poem_id: "fixture:jingyesi".to_owned(),
                title: "静夜思".to_owned(),
                author: "李白".to_owned(),
                dynasty: "唐".to_owned(),
                matched_line_index: 0,
                snippet: HighlightedSnippet {
                    text: "床前明月光".to_owned(),
                    highlights: vec![HighlightRange { start: 2, end: 4 }],
                },
            },
            None,
        )
    }

    #[test]
    fn search_json_and_human_output_agree_on_the_hit_count() {
        let out = SearchOut::new(
            "明月".to_owned(),
            10,
            &page(
                QueryPlan::NgramCandidates {
                    gram: "明月".to_owned(),
                    like_pattern: "%明月%".to_owned(),
                },
                None,
            ),
            SearchFilters {
                author: None,
                dynasty: None,
                rhyme_book: None,
            },
            vec![hit()],
        );
        let value = serde_json::to_value(&out).expect("序列化 search 载荷");
        assert_eq!(value["hits"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["plan"], serde_json::json!("Ngram"));
        assert_eq!(value["total_estimate"], serde_json::json!(7));
        let rendered = out.render().join("\n");
        assert!(rendered.contains("静夜思"), "{rendered}");
        assert!(rendered.contains("床前明月光"), "{rendered}");
        assert!(rendered.contains("估计命中 7"), "{rendered}");
    }

    #[test]
    fn a_next_cursor_is_rendered_as_a_ready_to_paste_flag() {
        let out = SearchOut::new(
            "明月".to_owned(),
            10,
            &page(
                QueryPlan::Match {
                    expression: "\"明月\"".to_owned(),
                },
                Some("eyJhIjoxfQ"),
            ),
            SearchFilters {
                author: None,
                dynasty: None,
                rhyme_book: None,
            },
            vec![hit()],
        );
        let rendered = out.render().join("\n");
        assert!(rendered.contains("--cursor eyJhIjoxfQ"), "{rendered}");
    }

    #[test]
    fn filters_hits_ignores_the_annotation_only_option() {
        let annotation_only = SearchFilters {
            author: None,
            dynasty: None,
            rhyme_book: Some("pingshui".to_owned()),
        };
        assert!(
            !annotation_only.filters_hits(),
            "--rhyme-book 只做标注，不该被当成过滤"
        );
        let filtering = SearchFilters {
            author: Some("李白".to_owned()),
            dynasty: None,
            rhyme_book: None,
        };
        assert!(filtering.filters_hits());
    }

    #[test]
    fn a_degraded_derived_state_is_spelled_out_in_both_forms() {
        let out = CorpusOut {
            path: "/tmp/corpus.db".to_owned(),
            corpus_version: "v1".to_owned(),
            schema_version: 2,
            built_at: "2026-08-11T00:00:00Z".to_owned(),
            poem_count: 474_162,
            index_detail_mode: "full".to_owned(),
            derived_indexes: "first_launch".to_owned(),
            shipped_scope: "tang-song".to_owned(),
            origin: "materialized".to_owned(),
            materialized: false,
            derived_ready: false,
            derived_reason: Some("磁盘只读".to_owned()),
        };
        let value = serde_json::to_value(&out).expect("序列化 corpus 载荷");
        assert_eq!(value["derived_ready"], serde_json::json!(false));
        assert_eq!(value["derived_reason"], serde_json::json!("磁盘只读"));
        let rendered = out.render().join("\n");
        assert!(
            rendered.contains("磁盘只读"),
            "原因必须透给用户：{rendered}"
        );
        assert!(rendered.contains("退化"), "后果必须说清：{rendered}");
    }

    #[test]
    fn a_ready_derived_state_omits_the_reason_field() {
        let out = CorpusOut {
            path: "/tmp/corpus.db".to_owned(),
            corpus_version: "v1".to_owned(),
            schema_version: 2,
            built_at: "2026-08-11T00:00:00Z".to_owned(),
            poem_count: 1,
            index_detail_mode: "full".to_owned(),
            derived_indexes: "first_launch".to_owned(),
            shipped_scope: "10k".to_owned(),
            origin: "configured".to_owned(),
            materialized: false,
            derived_ready: true,
            derived_reason: None,
        };
        let value = serde_json::to_value(&out).expect("序列化 corpus 载荷");
        assert!(value.get("derived_reason").is_none());
        assert!(out.render().join("\n").contains("就绪"));
    }
}

/// `models list` 与 `models verify` 的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelListOut {
    /// 本次动作名，`list` 或 `verify`。
    pub action: &'static str,
    /// 模型缓存根目录。
    pub cache_root: String,
    /// 清单里的每个模型。
    pub models: Vec<ModelRow>,
}

/// 清单里一个模型的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRow {
    /// 发布包名。
    pub name: String,
    /// `asr` 或 `tts`。
    pub kind: &'static str,
    /// `production` 或 `smoke`。
    pub role: &'static str,
    /// SPDX。
    pub license: String,
    /// 归档字节数。
    pub size_bytes: u64,
    /// 解包后的模型目录是否就位。
    pub unpacked: bool,
    /// 已校验归档是否还在本地。
    pub archived: bool,
    /// 随仓许可原文的文件名，在 `licenses/` 下。
    pub attribution: String,
    /// 许可门禁的拒绝原因；通过时不出现在 JSON 里。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// `verify` 实测通过的摘要；`list` 或本地无归档时不出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_sha256: Option<String>,
}

impl Renderable for ModelListOut {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("模型缓存：{}", self.cache_root)];
        for model in &self.models {
            let state = match (model.unpacked, model.archived) {
                (true, _) => "已就位",
                (false, true) => "仅有归档",
                (false, false) => "未下载",
            };
            lines.push(format!(
                "{}  {}/{}  {}  {:.1} MiB  {state}",
                model.name,
                model.kind,
                model.role,
                model.license,
                bytes_to_mib(model.size_bytes),
            ));
            if let Some(sha) = &model.verified_sha256 {
                lines.push(format!("  摘要已核对：{sha}"));
            }
            if let Some(refused) = &model.refused {
                lines.push(format!("  已拒绝加载：{refused}"));
            }
        }
        lines.push(
            "许可原文见 licenses/；权重不随安装包分发，只接受 MIT 与 Apache-2.0。".to_owned(),
        );
        lines
    }
}

/// `models fetch` 的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelFetchOut {
    /// 发布包名。
    pub name: String,
    /// 就位后的模型目录。
    pub path: String,
    /// SPDX。
    pub license: String,
    /// 随仓许可原文的文件名。
    pub attribution: String,
}

impl Renderable for ModelFetchOut {
    fn render(&self) -> Vec<String> {
        vec![
            format!("模型 {} 已就位：{}", self.name, self.path),
            format!(
                "许可 {}，原文见 licenses/{}",
                self.license, self.attribution
            ),
        ]
    }
}

/// `models remove` 的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRemoveOut {
    /// 发布包名。
    pub name: String,
    /// 解包目录被删了。
    pub removed_dir: bool,
    /// 归档被删了。
    pub removed_archive: bool,
}

impl Renderable for ModelRemoveOut {
    fn render(&self) -> Vec<String> {
        if !self.removed_dir && !self.removed_archive {
            return vec![format!("模型 {} 本地没有缓存，无需删除", self.name)];
        }
        let mut what = Vec::new();
        if self.removed_dir {
            what.push("模型目录");
        }
        if self.removed_archive {
            what.push("归档");
        }
        vec![format!("已删除 {} 的{}", self.name, what.join("与"))]
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "只用于给人看的体积，MiB 的小数位精度足够"
)]
fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

// ---------------------------------------------------------------- 背诵

/// 复习库的文件名，在 `app.data_dir` 下。
pub const RECITE_DATABASE_FILE: &str = "recite.db";

/// FSRS 等级写进载荷的稳定标识。
#[must_use]
pub const fn grade_key(grade: FsrsGrade) -> &'static str {
    match grade {
        FsrsGrade::Again => "again",
        FsrsGrade::Hard => "hard",
        FsrsGrade::Good => "good",
        FsrsGrade::Easy => "easy",
    }
}

/// FSRS 等级给人看的中文名。
#[must_use]
pub const fn grade_label(grade: FsrsGrade) -> &'static str {
    match grade {
        FsrsGrade::Again => "重来",
        FsrsGrade::Hard => "困难",
        FsrsGrade::Good => "良好",
        FsrsGrade::Easy => "轻松",
    }
}

/// 一次打字练习的分数。
///
/// 逐字段镜像 [`TypedScore`]（它没有 `Serialize`），**不做任何再计算**：比例原样以
/// `[0, 1]` 的小数呈现，不换算百分比。乘一个 100 也是在分数上做算术，而一旦这里开始
/// 算术，「评分只在内核」这条边界就没有可执行的判据了。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ScoreOut {
    /// 未漏读字符所占比例。
    pub completeness: f32,
    /// 严格字准。
    pub accuracy_strict: f32,
    /// 近音宽容后的字准。
    pub accuracy_lenient: f32,
    /// 打字路径无时序信号，内核给中性满值；**不表示发音质量**。
    pub fluency: f32,
    /// 是否被内核判为拒绝识别。
    pub is_rejected: bool,
    /// 各类对齐操作的计数。
    pub ops_summary: OpsSummaryOut,
}

impl From<&TypedScore> for ScoreOut {
    fn from(score: &TypedScore) -> Self {
        Self {
            completeness: score.completeness,
            accuracy_strict: score.accuracy_strict,
            accuracy_lenient: score.accuracy_lenient,
            fluency: score.fluency,
            is_rejected: score.is_rejected,
            ops_summary: OpsSummaryOut::from(&score.ops_summary),
        }
    }
}

/// 各类对齐操作的计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OpsSummaryOut {
    /// 相符字数。
    pub normal_count: usize,
    /// 漏读字数。
    pub deletion_count: usize,
    /// 增读字数。
    pub insertion_count: usize,
    /// 回读片段数。
    pub rerecitation_count: usize,
    /// 替换字数。
    pub substitution_count: usize,
}

impl From<&OpsSummary> for OpsSummaryOut {
    fn from(summary: &OpsSummary) -> Self {
        Self {
            normal_count: summary.normal_count,
            deletion_count: summary.deletion_count,
            insertion_count: summary.insertion_count,
            rerecitation_count: summary.rerecitation_count,
            substitution_count: summary.substitution_count,
        }
    }
}

/// 一项对齐操作。
///
/// 下标都是**归一化文本**的字符位置，与提示文本的下标不通用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpOut {
    /// 相符。
    Normal {
        /// 参考文本中的位置。
        reference_index: usize,
        /// 作答文本中的位置。
        attempt_index: usize,
        /// 相符的字。
        character: char,
    },
    /// 漏读。
    Deletion {
        /// 参考文本中的位置。
        reference_index: usize,
        /// 未读出的字。
        reference: char,
    },
    /// 增读。
    Insertion {
        /// 插入点对应的参考位置。
        reference_index: usize,
        /// 作答文本中的位置。
        attempt_index: usize,
        /// 多读出的字。
        attempt: char,
    },
    /// 回读。
    ReRecitation {
        /// 被重读区间在参考文本中的起点。
        reference_start: usize,
        /// 开区间终点。
        reference_end: usize,
        /// 被重读区间在作答文本中的起点。
        attempt_start: usize,
        /// 开区间终点。
        attempt_end: usize,
        /// 重读的文本。
        text: String,
    },
    /// 替换。
    Substitution {
        /// 参考文本中的位置。
        reference_index: usize,
        /// 作答文本中的位置。
        attempt_index: usize,
        /// 应读的字。
        reference: char,
        /// 实读的字。
        attempt: char,
        /// 是否为近音替换。由内核的 [`classify_substitution`] 判定。
        near_homophone: bool,
    },
}

impl From<&AlignOp> for OpOut {
    fn from(op: &AlignOp) -> Self {
        match op {
            AlignOp::Normal {
                reference_index,
                attempt_index,
                character,
            } => Self::Normal {
                reference_index: *reference_index,
                attempt_index: *attempt_index,
                character: *character,
            },
            AlignOp::Deletion {
                reference_index,
                reference,
            } => Self::Deletion {
                reference_index: *reference_index,
                reference: *reference,
            },
            AlignOp::Insertion {
                reference_index,
                attempt_index,
                attempt,
            } => Self::Insertion {
                reference_index: *reference_index,
                attempt_index: *attempt_index,
                attempt: *attempt,
            },
            AlignOp::ReRecitation {
                reference_start,
                reference_end,
                attempt_start,
                attempt_end,
                text,
            } => Self::ReRecitation {
                reference_start: *reference_start,
                reference_end: *reference_end,
                attempt_start: *attempt_start,
                attempt_end: *attempt_end,
                text: text.clone(),
            },
            AlignOp::Substitution {
                reference_index,
                attempt_index,
                reference,
                attempt,
            } => Self::Substitution {
                reference_index: *reference_index,
                attempt_index: *attempt_index,
                reference: *reference,
                attempt: *attempt,
                near_homophone: classify_substitution(*reference, *attempt)
                    == SubstitutionClass::NearHomophone,
            },
        }
    }
}

impl OpOut {
    /// 逐字标记里代表本项的记号。
    #[must_use]
    pub const fn mark(&self) -> char {
        match self {
            Self::Normal { .. } => '✓',
            Self::Deletion { .. } => '✗',
            Self::Insertion { .. } => '＋',
            Self::ReRecitation { .. } => '↻',
            Self::Substitution {
                near_homophone: true,
                ..
            } => '≈',
            Self::Substitution { .. } => '≠',
        }
    }

    /// 非相符项的一行说明；相符项返回 `None`。
    #[must_use]
    pub fn explain(&self) -> Option<String> {
        match self {
            Self::Normal { .. } => None,
            Self::Deletion {
                reference_index,
                reference,
            } => Some(format!(
                "第 {} 字 漏读：应读「{reference}」",
                reference_index + 1
            )),
            Self::Insertion {
                attempt_index,
                attempt,
                ..
            } => Some(format!(
                "作答第 {} 字 增读：多读了「{attempt}」",
                attempt_index + 1
            )),
            Self::ReRecitation {
                reference_start,
                reference_end,
                text,
                ..
            } => Some(format!(
                "第 {}–{} 字 回读：重读了「{text}」",
                reference_start + 1,
                reference_end
            )),
            Self::Substitution {
                reference_index,
                reference,
                attempt,
                near_homophone,
                ..
            } => Some(format!(
                "第 {} 字 {}：应读「{reference}」，实读「{attempt}」",
                reference_index + 1,
                if *near_homophone {
                    "近音替换"
                } else {
                    "替换"
                }
            )),
        }
    }
}

/// 一条排程项。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewItemOut {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 下次到期的 Unix 日序号。
    pub due_day: i64,
    /// 最近复习的 Unix 日序号。
    pub last_review_day: i64,
    /// 最近一次排出的间隔天数。
    pub scheduled_days: u32,
    /// FSRS 记忆稳定度。
    pub stability: f32,
    /// FSRS 记忆难度。
    pub difficulty: f32,
    /// 最近一次提交的等级。
    pub last_grade: &'static str,
}

impl From<&ReviewState> for ReviewItemOut {
    fn from(state: &ReviewState) -> Self {
        Self {
            poem_id: state.stable_id.clone(),
            due_day: state.due_day,
            last_review_day: state.last_review_day,
            scheduled_days: state.scheduled_days,
            stability: state.stability,
            difficulty: state.difficulty,
            last_grade: grade_key(state.last_grade),
        }
    }
}

impl ReviewItemOut {
    fn render_row(&self, index: usize) -> String {
        format!(
            "{:>2}. {}  等级 {} · 间隔 {} 天 · 到期日序 {} · 稳定度 {:.2} · 难度 {:.2}",
            index + 1,
            self.poem_id,
            self.last_grade,
            self.scheduled_days,
            self.due_day,
            self.stability,
            self.difficulty
        )
    }
}

/// `recite` 的载荷。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReciteOut {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 题目。
    pub title: String,
    /// 作者。
    pub author: String,
    /// 朝代原串。
    pub dynasty: String,
    /// 实际执行的形态。
    pub mode: &'static str,
    /// 用户请求的形态；仅在退化时出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_mode: Option<&'static str>,
    /// 退化原因；仅在退化时出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    /// 生效的挖空比例；非挖空形态不出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f32>,
    /// 生效的随机种子；非挖空形态不出现。**给出它就能复现同一份挖空。**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// 实际遮住的句数；非遮挡形态不出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub masked_lines: Option<usize>,
    /// 展示给用户的提示文本，被遮处为全角下划线。
    pub prompt: String,
    /// 被遮位置，原文正文字符序列的下标。
    pub hidden_indices: Vec<usize>,
    /// 归一化后的参考诗文。
    pub reference: String,
    /// 归一化后的作答。
    pub answer: String,
    /// 内核给出的分数。
    pub score: ScoreOut,
    /// 内核给出的逐项对齐操作。
    pub ops: Vec<OpOut>,
    /// 本次提交的 FSRS 等级。
    pub grade: &'static str,
    /// 等级来自打字映射还是用户直接指定。
    pub grade_source: &'static str,
    /// 本次是否为该作品的首次作答。
    pub first_attempt: bool,
    /// 复习库路径。
    pub database: String,
    /// 提交后的排程状态。
    pub review: ReviewItemOut,
}

impl Renderable for ReciteOut {
    fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "{} — {}（{}）  [{}]",
                self.title, self.author, self.dynasty, self.poem_id
            ),
            format!("形态：{}", self.render_mode()),
        ];
        if let Some(reason) = &self.fallback_reason {
            lines.push(format!("退化：{reason}"));
        }
        lines.push(format!("提示：{}", self.prompt));
        lines.push(format!("作答：{}", self.answer));
        lines.push(format!(
            "逐字：{}",
            self.ops.iter().map(OpOut::mark).collect::<String>()
        ));
        lines.push("记号：✓ 相符 · ✗ 漏读 · ＋ 增读 · ↻ 回读 · ≠ 替换 · ≈ 近音替换".to_owned());
        let differences = self
            .ops
            .iter()
            .filter_map(OpOut::explain)
            .collect::<Vec<_>>();
        if differences.is_empty() {
            lines.push("全篇相符，没有差异。".to_owned());
        } else {
            lines.push(format!("差异 {} 处：", differences.len()));
            lines.extend(differences.into_iter().map(|text| format!("    {text}")));
        }
        lines.push(format!(
            "完整度 {:.3} · 严格字准 {:.3} · 宽容字准 {:.3}（均为 0 至 1 的比例）",
            self.score.completeness, self.score.accuracy_strict, self.score.accuracy_lenient
        ));
        if self.score.is_rejected {
            lines.push("内核判为拒绝识别：作答与本篇相去过远，本次记为最低档。".to_owned());
        }
        lines.push("不评估发音标准度；打字路径的节奏连贯度为中性值。".to_owned());
        lines.push(format!(
            "评级 {}（{}）· 来源 {} · {}",
            self.grade,
            grade_label_of(self.grade),
            self.grade_source,
            if self.first_attempt {
                "首次作答"
            } else {
                "非首次作答"
            }
        ));
        lines.push(format!(
            "下次复习：{} 天后（到期日序 {}）· 复习库 {}",
            self.review.scheduled_days, self.review.due_day, self.database
        ));
        lines
    }
}

impl ReciteOut {
    fn render_mode(&self) -> String {
        let mut text = match self.mode {
            "cloze" => "挖空".to_owned(),
            "first-char" => "首字提示".to_owned(),
            "masked" => "遮挡".to_owned(),
            other => other.to_owned(),
        };
        if let Some(ratio) = self.ratio {
            text.push_str(&format!("（比例 {ratio}",));
            if let Some(seed) = self.seed {
                text.push_str(&format!("，种子 {seed}，用 `--seed {seed}` 复现"));
            }
            text.push('）');
        }
        if let Some(lines) = self.masked_lines {
            text.push_str(&format!("（遮 {lines} 句）"));
        }
        text
    }
}

/// `recite due` 的载荷。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReciteDueOut {
    /// 复习库路径。
    pub database: String,
    /// `due_today` 只列今天到期，`all` 列整份排程。
    pub scope: &'static str,
    /// 排程项，按到期日升序。
    pub items: Vec<ReviewItemOut>,
}

impl Renderable for ReciteDueOut {
    fn render(&self) -> Vec<String> {
        let scope = if self.scope == "all" {
            "整份排程"
        } else {
            "今天到期"
        };
        let mut lines = vec![format!(
            "{scope} {} 项 · 复习库 {}",
            self.items.len(),
            self.database
        )];
        if self.items.is_empty() {
            lines.push(
                "没有到期项。练一轮（`yunjian recite <poem-id>`）即可建立排程；\
                 用 `--all` 可看尚未到期的部分。"
                    .to_owned(),
            );
        } else {
            lines.extend(
                self.items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| item.render_row(index)),
            );
        }
        lines
    }
}

/// `recite stats` 的载荷。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReciteStatsOut {
    /// 复习库路径。
    pub database: String,
    /// 已排程的作品数。
    pub scheduled_total: usize,
    /// 今天到期的作品数。
    pub due_today: usize,
    /// 按最近一次等级的分布。
    pub by_last_grade: GradeCountsOut,
    /// 本机生效的评级阈值，原样取自 `[recite.grading]`。
    ///
    /// 报阈值而不是自己重算等级：等级由内核的 `grade_typed` 按严格优先级得出，
    /// 这里再算一遍就会多出第二套规则。
    pub grading: GradingConfig,
}

/// 四档等级各自的计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct GradeCountsOut {
    /// 最近一次评为重来的作品数。
    pub again: usize,
    /// 最近一次评为困难的作品数。
    pub hard: usize,
    /// 最近一次评为良好的作品数。
    pub good: usize,
    /// 最近一次评为轻松的作品数。
    pub easy: usize,
}

impl GradeCountsOut {
    /// 按每首作品最近一次的等级计数。
    #[must_use]
    pub fn tally(states: &[ReviewState]) -> Self {
        let mut counts = Self::default();
        for state in states {
            match state.last_grade {
                FsrsGrade::Again => counts.again += 1,
                FsrsGrade::Hard => counts.hard += 1,
                FsrsGrade::Good => counts.good += 1,
                FsrsGrade::Easy => counts.easy += 1,
            }
        }
        counts
    }
}

impl Renderable for ReciteStatsOut {
    fn render(&self) -> Vec<String> {
        vec![
            format!("复习库：{}", self.database),
            format!(
                "已排程 {} 首 · 今天到期 {} 首",
                self.scheduled_total, self.due_today
            ),
            format!(
                "最近等级分布：重来 {} · 困难 {} · 良好 {} · 轻松 {}",
                self.by_last_grade.again,
                self.by_last_grade.hard,
                self.by_last_grade.good,
                self.by_last_grade.easy
            ),
            format!(
                "评级阈值（config.toml 的 [recite.grading]）：完整度低于 {} 记重来 · \
                 宽容字准低于 {} 记困难 · 回读多于 {} 次记困难 · \
                 首次作答严格字准达到 {} 才记轻松",
                self.grading.again_completeness_below,
                self.grading.hard_accuracy_lenient_below,
                self.grading.hard_rerecitation_above,
                self.grading.easy_accuracy_strict_at_least
            ),
            "等级由背诵内核按严格优先级判定，本命令只报阈值与分布。".to_owned(),
        ]
    }
}

fn grade_label_of(key: &str) -> &'static str {
    FsrsGrade::ALL
        .into_iter()
        .find(|grade| grade_key(*grade) == key)
        .map_or("未知", grade_label)
}
