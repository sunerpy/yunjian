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
    AuthorDetail, CorpusHandle, CorpusMeta, CorpusOrigin, DerivedState, HighlightedSnippet,
    MetaHit, PoemDetail, RhymeGroupMatches, RhymeGroupMembership, SearchPage, TextSearchHit,
};

/// 能渲染成若干行人类可读文本的输出。
pub trait Renderable {
    /// 渲染成逐行文本。每一行由 [`crate::present::line`] 写往 stdout。
    fn render(&self) -> Vec<String>;
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
