//! `cargo xtask commentary-index`：由 `corpus/commentary/sources/` 重新生成聚合索引。
//!
//! 索引是**生成物**，不是手写数据。它存在的理由只有一个：让下游（todo 17 的
//! SQLite 写入、todo 27/36 的展示层）读一个已排序、已去重的单文件，而不必
//! 自己枚举 `sources/` 并复现排序规则。因此它必须可逐字节重建——
//! `require_index_matches` 就是那条门禁。

use anyhow::{Context, bail};
use std::path::PathBuf;
use yunjian_corpus::commentary;

fn commentary_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../corpus/commentary")
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let dir = commentary_dir();
    let seeds = commentary::load_seeds(&dir).context("读取集评种子集")?;

    let outcome = commentary::validate_all(&seeds);
    let rejected = outcome.rejections.len();
    let accepted = outcome
        .require_all_accepted()
        .context("集评种子集未全部通过出处校验")?;
    tracing::info!(
        entries = accepted.len(),
        rejected,
        "集评种子集通过逐条出处校验"
    );

    let rendered = commentary::render_index(&seeds).context("生成集评索引")?;
    let index_path = dir.join(commentary::INDEX_FILE);

    if check {
        let count = commentary::require_index_matches(&dir).context("集评索引与种子集不一致")?;
        tracing::info!(entries = count, path = %index_path.display(), "集评索引无漂移");
        return Ok(());
    }

    if accepted.len() < 100 {
        bail!(
            "集评种子集只有 {} 条，方案要求至少 100 条已核实条目",
            accepted.len()
        );
    }

    std::fs::write(&index_path, rendered)
        .with_context(|| format!("写入 {}", index_path.display()))?;
    tracing::info!(
        entries = accepted.len(),
        path = %index_path.display(),
        "已重新生成集评索引"
    );
    Ok(())
}
