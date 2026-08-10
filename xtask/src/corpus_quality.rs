//! `xtask corpus-quality`：产出重出分组与数据缺陷报告，并守住回归基线。
//!
//! 写出三个文件，两个 JSON 工件加一份人类可读的 Markdown：
//!
//! - `corpus/reports/defects.json` / `.md`——**一行一个 finding**；
//! - `corpus/reports/dispositions.json`——**一行一条输入记录**。
//!
//! 默认跑 **fixture 范围**（随仓提交的 fixture 加
//! `crates/yunjian-corpus/tests/fixtures/quality/` 的补充记录），因为
//! `corpus/reports/baseline.json` 就是在这个范围上钉住的：CI 里没有 835 MB 的上游
//! 检出，一份跑不起来的门禁等于没有门禁。要在真实检出上跑就显式传
//! `--chinese-poetry-dir` 与 `--werneror-dir`，此时基线不适用并会明确说明。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use yunjian_corpus::ingest::werneror::{Bucket, CLASSICAL_BUCKETS};
use yunjian_corpus::quality::{
    BASELINE_JSON, Baseline, DEFECTS_JSON, DEFECTS_MD, DISPOSITIONS_JSON, PipelineOutcome,
    ReasonCode, load_supplement, run_pipeline, write_artifacts,
};

use crate::verify_sources::emit;

const CORPUS_VERSION: &str = "0.1.0";
const FIXTURE_SCOPE: &str = "fixtures";
const CUSTOM_SCOPE: &str = "custom";

const CHINESE_POETRY_FIXTURES: &str = "crates/yunjian-corpus/tests/fixtures/chinese_poetry";
const WERNEROR_FIXTURES: &str = "crates/yunjian-corpus/tests/fixtures/werneror";
const SUPPLEMENT_FIXTURES: &str = "crates/yunjian-corpus/tests/fixtures/quality";

/// fixture 目录里在古典白名单上的分桶。
///
/// `当代.csv` 与 `未来.csv` 刻意不列——前者是已知近现代桶，后者根本不在白名单上，
/// 两者都必须由策略排除而不是由这份名单排除。
const FIXTURE_BUCKETS: [&str; 7] = [
    "先秦.csv",
    "秦.csv",
    "魏晋末南北朝初.csv",
    "隋末唐初.csv",
    "唐.csv",
    "宋末金初.csv",
    "辽.csv",
];

const BASELINE_NOTE: &str = "由 `cargo run -p xtask -- corpus-quality --write-baseline` 生成。\
范围是随仓 fixture 加 tests/fixtures/quality/ 的补充记录，逐 code 容差按 \
ReasonCode::default_tolerance_pct；restricted_license 与 excluded_by_policy 为 0%。\
容差按整数下取整，所以小计数在实践中要求精确相等。";

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .context("无法从 xtask/ 推出仓库根目录")?
        .to_path_buf();
    if !root.join(CHINESE_POETRY_FIXTURES).exists() {
        bail!("在 {} 下找不到 {CHINESE_POETRY_FIXTURES}", root.display());
    }
    Ok(root)
}

fn fixture_buckets() -> Result<Vec<Bucket>> {
    FIXTURE_BUCKETS
        .iter()
        .map(|file| {
            CLASSICAL_BUCKETS
                .iter()
                .find(|bucket| bucket.file == *file)
                .copied()
                .with_context(|| format!("古典白名单里没有 fixture 分桶 {file}"))
        })
        .collect()
}

fn run_scope(
    root: &Path,
    chinese_poetry_dir: Option<PathBuf>,
    werneror_dir: Option<PathBuf>,
) -> Result<(&'static str, PipelineOutcome)> {
    match (chinese_poetry_dir, werneror_dir) {
        (None, None) => {
            let supplement = load_supplement(&root.join(SUPPLEMENT_FIXTURES))
                .context("读取 tests/fixtures/quality/ 的补充记录失败")?;
            emit(&format!(
                "范围：{FIXTURE_SCOPE}（随仓 fixture + {} 条补充记录）",
                supplement.len()
            ));
            let outcome = run_pipeline(
                &root.join(CHINESE_POETRY_FIXTURES),
                &root.join(WERNEROR_FIXTURES),
                &fixture_buckets()?,
                supplement,
                CORPUS_VERSION,
            )
            .context("fixture 范围流水线失败")?;
            Ok((FIXTURE_SCOPE, outcome))
        }
        (Some(cp), Some(wr)) => {
            emit(&format!(
                "范围：{CUSTOM_SCOPE}（{} + {}），全部 28 个古典分桶；基线不适用",
                cp.display(),
                wr.display()
            ));
            let outcome = run_pipeline(&cp, &wr, CLASSICAL_BUCKETS, Vec::new(), CORPUS_VERSION)
                .context("真实检出流水线失败")?;
            Ok((CUSTOM_SCOPE, outcome))
        }
        _ => bail!("--chinese-poetry-dir 与 --werneror-dir 必须同时给出"),
    }
}

pub fn run(
    chinese_poetry_dir: Option<PathBuf>,
    werneror_dir: Option<PathBuf>,
    write_baseline: bool,
) -> Result<()> {
    let root = repo_root()?;
    let (scope, outcome) = run_scope(&root, chinese_poetry_dir, werneror_dir)?;
    let report = &outcome.report;

    write_artifacts(&root, scope, report).context("写出质量工件失败")?;
    emit(&format!(
        "已写出 {DEFECTS_JSON}、{DEFECTS_MD}、{DISPOSITIONS_JSON}"
    ));

    emit("");
    emit("处置台账（守恒只看这三个数）：");
    emit(&format!("  输入记录数      {}", report.input_rows));
    emit(&format!("  shipped         {}", report.counts.shipped));
    emit(&format!("  quarantined     {}", report.counts.quarantined));
    emit(&format!("  excluded        {}", report.counts.excluded));
    emit(&format!(
        "  {} + {} + {} == {} ✓",
        report.counts.shipped, report.counts.quarantined, report.counts.excluded, report.input_rows
    ));
    emit(&format!("  poem_count      {}", report.poem_count));

    emit("");
    emit(&format!(
        "逐原因码 finding 数（合计 {}，与记录数无关，不要相加）：",
        report.findings.len()
    ));
    for reason in ReasonCode::ALL {
        emit(&format!(
            "  {:<24} {}",
            reason.as_str(),
            report.finding_count(reason)
        ));
    }

    emit("");
    let baseline_path = root.join(BASELINE_JSON);
    if write_baseline {
        if scope != FIXTURE_SCOPE {
            bail!("--write-baseline 只在 fixture 范围可用；真实检出的计数不能钉成基线");
        }
        Baseline::from_report(scope, BASELINE_NOTE, report)
            .store(&baseline_path)
            .context("写出基线失败")?;
        emit(&format!("已写出 {BASELINE_JSON}（逐 code 计数与容差）"));
        return Ok(());
    }

    if scope != FIXTURE_SCOPE {
        emit(&format!(
            "跳过基线检查：{BASELINE_JSON} 的范围是 {FIXTURE_SCOPE}，与本次的 {scope} 不可比"
        ));
        return Ok(());
    }
    let baseline = Baseline::load(&baseline_path).context("读取基线失败")?;
    match baseline.check(report) {
        Ok(()) => {
            emit("基线检查通过：逐原因码计数都在容差内");
            Ok(())
        }
        Err(error) => {
            for drift in baseline.drift(report) {
                emit(&format!("  漂移 {drift}"));
            }
            Err(anyhow::Error::msg(error.to_string()))
        }
    }
}
