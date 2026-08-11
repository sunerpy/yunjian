//! `xtask corpus-build`：产出**待发布**的那一对文件。
//!
//! ```text
//! corpus/build/release/corpus.db         随包库（无诊断表、无候选表）
//! corpus/build/release/corpus-audit.db   审计库（defect + disposition）
//! ```
//!
//! # 为什么不复用 `corpus-measure --keep-databases`
//!
//! `corpus-measure` 会在它建出来的库上跑一次**首启构建**——那是它要测的东西之一。
//! 于是它留下的文件里带着候选表，正好是随包工件不该有的形态，`corpus-package`
//! 的第二条断言会直接拒掉它。两个子命令因此各建各的：测量的库测完即弃，发布的库
//! 干净落盘。
//!
//! # 规模不接受覆盖
//!
//! 随包默认集由实测结论选定，不是命令行选项。能被 `--scale` 改掉的默认集会让
//! 「工件的形态被实测背书」这句话失去意义——那时报告说的是一档、产物是另一档。

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::corpus_measure::{SHIPPED_DEFAULT_SCOPE, Scale, assemble_shipped_input};
use crate::verify_sources::emit;

const SOURCES_TOML: &str = "corpus/sources.toml";
const INDEX_VERDICT: &str = "corpus/reports/index-mode.json";
const DEFAULT_OUT_DIR: &str = "corpus/build/release";

pub fn run(
    chinese_poetry_dir: PathBuf,
    werneror_dir: PathBuf,
    rhyme_dir: PathBuf,
    out_dir: Option<PathBuf>,
) -> Result<()> {
    let root = crate::index_spike::repo_root()?;
    let out_dir = out_dir.unwrap_or_else(|| root.join(DEFAULT_OUT_DIR));
    let scale: Scale = SHIPPED_DEFAULT_SCOPE;

    emit("== 构建待发布语料库 ==");
    emit(&format!(
        "随包默认集：{}（{}）——由实测结论选定，不接受覆盖",
        scale.key(),
        scale.description()
    ));

    let verdict_bytes = std::fs::read(root.join(INDEX_VERDICT))
        .with_context(|| format!("读取 {INDEX_VERDICT} 失败"))?;
    let manifest_bytes = std::fs::read(root.join(SOURCES_TOML))
        .with_context(|| format!("读取 {SOURCES_TOML} 失败"))?;
    let rhymes = yunjian_corpus::rhyme::import(&rhyme_dir)
        .with_context(|| format!("导入韵书失败 {}", rhyme_dir.display()))?;

    let input = assemble_shipped_input(
        scale,
        &chinese_poetry_dir,
        &werneror_dir,
        &rhymes,
        &manifest_bytes,
        &verdict_bytes,
    )?;
    emit(&format!(
        "入库 {} 首（输入 {} 条，缺陷 {} 条）",
        input.records.len(),
        input.quality.input_rows,
        input.quality.findings.len()
    ));

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("创建输出目录失败 {}", out_dir.display()))?;
    let corpus_db = out_dir.join("corpus.db");
    let audit_db = yunjian_corpus::db::audit_path(&corpus_db);
    let stats = yunjian_corpus::db::build_database_with_stats(&corpus_db, &input)
        .with_context(|| format!("构建语料库失败 {}", corpus_db.display()))?;

    emit("");
    emit(&format!(
        "随包库 {}：{} MiB（VACUUM 前 {} MiB）",
        corpus_db.display(),
        stats.bytes_after_vacuum / (1024 * 1024),
        stats.bytes_before_vacuum / (1024 * 1024)
    ));
    emit(&format!(
        "审计库 {}：{} MiB（**不随包**，作为 CI 工件与开发者可选下载）",
        audit_db.display(),
        stats.audit_bytes / (1024 * 1024)
    ));
    emit("跨文件处置守恒已在构建内校验，未通过则两个文件都不会落盘。");
    emit("");
    emit("下一步：cargo run -p xtask -- corpus-package");
    Ok(())
}
