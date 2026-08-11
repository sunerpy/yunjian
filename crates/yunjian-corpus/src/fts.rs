//! 构建期对索引裁决的把关。
//!
//! # 为什么这个模块只剩一条断言
//!
//! 检索结构（`ngram` / `poem_fts` / `poem_last_char`）现在全部由**首启在本机派生**，
//! 实现与校验都在 `yunjian_core::derive`——它们是 `poem.body` 的确定性派生物，属于
//! 运行时而非工件。构建期唯一还要做的事，是把裁决里那条硬要求转成一次失败：
//! 裁决禁用候选表即拒绝构建。
//!
//! 裁决（`corpus/reports/index-mode.json`）描述的是**运行时应有的索引形态**。
//! `chosen_mode` 由 `db.rs` 写进 `corpus_meta.index_detail_mode`，首启按那一列建
//! `poem_fts`；`ngram_aux_enabled` 则在这里把关。两条都仍然有牙齿：改掉任何一条，
//! 37 条契约立刻变红。

use yunjian_core::{Error, Result};

/// 裁决禁用候选表即失败。
///
/// 契约里走候选表路径的条目全是两字查询，而 FTS5 trigram 在三字以下推不出任何约束，
/// 所以「不要候选表」等于放弃最常见的查询形态。
pub(crate) fn reject_disabled_ngram_aux(ngram_aux_enabled: bool) -> Result<()> {
    if ngram_aux_enabled {
        return Ok(());
    }
    Err(Error::Corpus(
        "索引 verdict 禁用了 n-gram 辅助表，但选定的运行时索引形态要求它存在（首启本机构建）"
            .to_owned(),
    ))
}
