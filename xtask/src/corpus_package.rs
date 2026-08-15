//! `xtask corpus-package`：把随包语料库打成可发布工件，并在写出任何文件**之前**
//! 跑完全部中止断言。
//!
//! # 产物
//!
//! ```text
//! yunjian-corpus-<corpus_version>.db.gz   随包工件（gzip，非裸文件）
//! yunjian-corpus-<corpus_version>.db.gz.sha256   校验旁文件
//! manifest.json                            兼容范围与实测结论
//! ```
//!
//! 为什么 gzip 而不是裸文件：Android 的资产不是文件（「An asset is not a file. It is an
//! entry in the ZIP archive that makes up an APK」），无论如何都要复制出来才能打开；
//! 压缩既省下 APK 里的空间，也绕开平台对某些已知扩展名的自动变换。
//!
//! # 五条中止断言（写文件之前）
//!
//! 1. `corpus_meta.integrity_check == 'ok'`；
//! 2. 随包库不含 `defect` / `disposition` / `ngram` / `poem_fts` / `poem_last_char`；
//! 3. 跨文件守恒成立（随包库 + 审计库）；
//! 4. 实测结论 `within_budget == true`；
//! 5. 库记录的 `shipped_scope` 与结论选定的随包默认集一致。
//!
//! 第六条只能落盘后才知道：**最终 gzip 是否在预算内**。判为超预算就把已写出的文件
//! 删掉——绝不留下一个「已经生成、只是不该发」的工件，那种文件迟早会被人上传。
//!
//! # 为什么要解压回读
//!
//! `manifest.json` 是另写出来的一份描述，与 `.gz` 之间没有任何机制性联系。不回读的话
//! 完全可能「manifest 描述上一次构建、`.gz` 是这一次」。所以打完包再解压一次，
//! 逐项核对工件里的 `corpus_meta` 与 manifest。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::corpus_measure::{ArtifactShape, MeasuredReport};
use crate::verify_sources::emit;

const REPORT_JSON: &str = "corpus/reports/measurements.json";
const SOURCES_TOML: &str = "corpus/sources.toml";
const DEFAULT_OUT_DIR: &str = "corpus/build/package";

/// 应用能接受的最低版本。与 `schema_version` 一起构成兼容范围：应用按
/// `schema_version` 决定能不能读，工件按 `min_app_version` 声明需要多新的应用。
const MIN_APP_VERSION: &str = "0.1.0";

/// gzip 压缩级别。固定值而非 `default()`：工件摘要要可复现，而压缩级别会改变字节。
const GZIP_LEVEL: Compression = Compression::new(9);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub corpus_version: String,
    pub min_app_version: String,
    pub record_count: i64,
    pub source_manifest_sha256: String,
    pub shipped_scope: String,
    pub derived_indexes: String,
    pub index_detail_mode: String,
    pub built_at: String,
    pub artifact_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub uncompressed_bytes: u64,
    pub measurement: ManifestMeasurement,
}

/// 随工件一起发布的实测结论。
///
/// 为什么工件要自带这些数字：下游（应用的下载器、发布说明、排障的人）拿到的是一个
/// `.gz`，而 `corpus/reports/measurements.json` 在仓库里。把结论刻进 manifest，
/// 「这个工件是在什么结论下被允许发布的」就不依赖于去翻某个 commit。
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestMeasurement {
    pub within_budget: bool,
    pub budget_gzip_bytes: u64,
    pub budget_declared_by: String,
    pub measured_gzip_bytes: u64,
    pub worst_p95_ms: f64,
    /// 首启在本机派生三张检索结构的实测秒数——三者不随包的代价。
    pub first_launch_seconds: f64,
    /// 拆出去的审计库字节。不随包。
    pub audit_bytes: u64,
    pub verdict_summary: String,
}

struct Meta {
    schema_version: u32,
    corpus_version: String,
    built_at: String,
    source_manifest_sha256: String,
    poem_count: i64,
    index_detail_mode: String,
    derived_indexes: String,
    shipped_scope: String,
    integrity_check: String,
}

pub fn run(corpus_db: PathBuf, out_dir: Option<PathBuf>) -> Result<()> {
    let root = crate::index_spike::repo_root()?;
    let out_dir = out_dir.unwrap_or_else(|| root.join(DEFAULT_OUT_DIR));
    let audit_db = yunjian_corpus::db::audit_path(&corpus_db);

    emit("== 语料工件打包 ==");
    emit(&format!("随包库：{}", corpus_db.display()));
    emit(&format!("审计库：{}（不随包）", audit_db.display()));

    crate::prerequisite::require_corpus_db(&corpus_db)?;
    if !audit_db.exists() {
        bail!(
            "审计库不存在 {}；跨文件守恒无法校验，而守恒是打包的前置条件。\
             审计库由同一次 `corpus-build` 产出，不要单独搬动其中一个文件",
            audit_db.display()
        );
    }

    let report = MeasuredReport::load(root.join(REPORT_JSON))?;
    let meta = read_meta(&corpus_db)?;

    // ---- 中止断言 1：完整性
    if meta.integrity_check != "ok" {
        bail!(
            "随包库的 corpus_meta.integrity_check 是 `{}` 而不是 `ok`；\
             一个完整性未通过的语料库不得发布",
            meta.integrity_check
        );
    }
    emit("  [1/5] integrity_check = ok");

    // ---- 中止断言 2：随包库无诊断表与候选表
    {
        let connection = Connection::open(&corpus_db)
            .with_context(|| format!("打开 {} 失败", corpus_db.display()))?;
        yunjian_corpus::db::assert_no_diagnostic_tables(&connection)?;
    }
    emit(&format!(
        "  [2/5] 随包库不含 {}",
        yunjian_corpus::db::NON_SHIPPED_TABLES.join(" / ")
    ));

    // ---- 中止断言 3：跨文件守恒
    yunjian_corpus::db::verify_conservation_across_files(&corpus_db, &audit_db)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    emit("  [3/5] 跨文件处置守恒成立（随包库 + 审计库）");

    // ---- 中止断言 4：实测结论在预算内
    if !report.verdict.within_budget {
        bail!(
            "实测结论 within_budget = false，不得发布工件。结论原文：{}",
            report.verdict.summary
        );
    }
    emit("  [4/5] 实测结论 within_budget = true");

    // ---- 中止断言 5：库的形态与结论选定的一致
    let shipped_row = report
        .scales
        .iter()
        .filter(|row| row.artifact_shape == ArtifactShape::Shipped)
        .filter_map(|row| row.measurement.as_ref().map(|m| (row, m)))
        .find(|(row, _)| row.scale == meta.shipped_scope)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "实测报告里没有随包形态、规模为 `{}` 的实测行；\
                 这个库的形态没有被实测背书，体积与延迟结论对它不成立",
                meta.shipped_scope
            )
        })?;
    if meta.derived_indexes != "first_launch" {
        bail!(
            "随包库记录 derived_indexes = `{}`，而结论选定的是首启本机派生",
            meta.derived_indexes
        );
    }
    if shipped_row.1.poem_count as i64 != meta.poem_count {
        bail!(
            "随包库有 {} 首，而 `{}` 规模的实测行是 {} 首；\
             这个库与被实测的那个不是同一份语料",
            meta.poem_count,
            meta.shipped_scope,
            shipped_row.1.poem_count
        );
    }
    let recorded_manifest_sha = sha256_of(&std::fs::read(root.join(SOURCES_TOML))?);
    if recorded_manifest_sha != meta.source_manifest_sha256 {
        bail!(
            "随包库记录的 source manifest 摘要 {} 与当前 {SOURCES_TOML} 的 {} 不符；\
             这个库是用另一份源清单建的，许可判定与它对不上",
            meta.source_manifest_sha256,
            recorded_manifest_sha
        );
    }
    emit(&format!(
        "  [5/5] 形态与结论一致：scope={} derived_indexes={} {} 首",
        meta.shipped_scope, meta.derived_indexes, meta.poem_count
    ));

    // ---- 落盘
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("创建输出目录失败 {}", out_dir.display()))?;
    let artifact_name = format!("yunjian-corpus-{}.db.gz", meta.corpus_version);
    let artifact_path = out_dir.join(&artifact_name);
    let uncompressed_bytes = std::fs::metadata(&corpus_db)?.len();
    let size_bytes = compress(&corpus_db, &artifact_path)?;
    let digest = sha256_of_file(&artifact_path)?;

    let manifest = Manifest {
        schema_version: meta.schema_version,
        corpus_version: meta.corpus_version.clone(),
        min_app_version: MIN_APP_VERSION.to_owned(),
        record_count: meta.poem_count,
        source_manifest_sha256: meta.source_manifest_sha256.clone(),
        shipped_scope: meta.shipped_scope.clone(),
        derived_indexes: meta.derived_indexes.clone(),
        index_detail_mode: meta.index_detail_mode.clone(),
        built_at: meta.built_at.clone(),
        artifact_name: artifact_name.clone(),
        size_bytes,
        sha256: digest.clone(),
        uncompressed_bytes,
        measurement: ManifestMeasurement {
            within_budget: report.verdict.within_budget,
            budget_gzip_bytes: report.budget.artifact_gzip_bytes,
            budget_declared_by: report.budget.declared_by.clone(),
            measured_gzip_bytes: shipped_row.1.gzip_bytes,
            worst_p95_ms: shipped_row.1.worst_p95_ms,
            first_launch_seconds: shipped_row.1.first_launch_seconds,
            audit_bytes: shipped_row.1.audit_bytes,
            verdict_summary: report.verdict.summary.clone(),
        },
    };

    let checksum_path = out_dir.join(format!("{artifact_name}.sha256"));
    // `sha256sum -c` 的格式：`<摘要>  <文件名>`，两个空格。文件名不带目录，
    // 这样在工件所在目录里直接 `sha256sum -c` 就能过。
    std::fs::write(&checksum_path, format!("{digest}  {artifact_name}\n"))
        .with_context(|| format!("写出 {} 失败", checksum_path.display()))?;
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )
    .with_context(|| format!("写出 {} 失败", manifest_path.display()))?;

    // ---- 第六条：最终工件是否在预算内。只能落盘后才知道。
    if size_bytes > report.budget.artifact_gzip_bytes {
        let _ = std::fs::remove_file(&artifact_path);
        let _ = std::fs::remove_file(&checksum_path);
        let _ = std::fs::remove_file(&manifest_path);
        bail!(
            "最终工件 {} MiB 超出预算 {} MiB，已删除刚写出的文件。\
             一个「已经生成、只是不该发」的工件迟早会被人上传",
            size_bytes / (1024 * 1024),
            report.budget.artifact_gzip_bytes / (1024 * 1024)
        );
    }

    // ---- 解压回读：manifest 与工件必须逐项一致
    verify_round_trip(&artifact_path, &manifest)?;

    emit("");
    emit(&format!(
        "工件 {} （{} MiB，预算 {} MiB）",
        artifact_name,
        size_bytes / (1024 * 1024),
        report.budget.artifact_gzip_bytes / (1024 * 1024)
    ));
    emit(&format!("摘要 {digest}"));
    emit(&format!(
        "解压后 {} MiB，{} 首，schema v{}，corpus {}",
        uncompressed_bytes / (1024 * 1024),
        meta.poem_count,
        meta.schema_version,
        meta.corpus_version
    ));
    emit(&format!(
        "首启派生检索结构实测 {:.1} s；审计库 {} MiB 不随包",
        manifest.measurement.first_launch_seconds,
        manifest.measurement.audit_bytes / (1024 * 1024)
    ));
    emit(&format!("输出目录 {}", out_dir.display()));
    emit(&format!(
        "发布：git tag corpus-v{} && gh release create corpus-v{} {}/{{{},{}.sha256,manifest.json}}",
        meta.corpus_version,
        meta.corpus_version,
        out_dir.display(),
        artifact_name,
        artifact_name
    ));
    Ok(())
}

fn read_meta(path: &Path) -> Result<Meta> {
    let connection = yunjian_corpus::db::open_corpus(path)
        .map_err(|error| anyhow::anyhow!("以只读方式打开随包库失败：{error}"))?;
    connection
        .query_row(
            "SELECT schema_version, corpus_version, built_at, source_manifest_sha256, \
                    poem_count, index_detail_mode, derived_indexes, shipped_scope, integrity_check \
             FROM corpus_meta",
            [],
            |row| {
                Ok(Meta {
                    schema_version: row.get(0)?,
                    corpus_version: row.get(1)?,
                    built_at: row.get(2)?,
                    source_manifest_sha256: row.get(3)?,
                    poem_count: row.get(4)?,
                    index_detail_mode: row.get(5)?,
                    derived_indexes: row.get(6)?,
                    shipped_scope: row.get(7)?,
                    integrity_check: row.get(8)?,
                })
            },
        )
        .context("读取 corpus_meta 失败")
}

fn compress(source: &Path, destination: &Path) -> Result<u64> {
    let mut input =
        std::fs::File::open(source).with_context(|| format!("打开 {} 失败", source.display()))?;
    let output = std::fs::File::create(destination)
        .with_context(|| format!("创建 {} 失败", destination.display()))?;
    let mut encoder = GzEncoder::new(output, GZIP_LEVEL);
    std::io::copy(&mut input, &mut encoder)?;
    encoder.finish()?.flush()?;
    Ok(std::fs::metadata(destination)?.len())
}

/// 解压工件、读回 `corpus_meta`，逐项与 manifest 核对。
///
/// 核的是身份与计数五项：`schema_version` / `corpus_version` / `source_manifest_sha256`
/// / `poem_count` / `shipped_scope`。任一不符都说明 manifest 描述的不是这个 `.gz`。
fn verify_round_trip(artifact: &Path, manifest: &Manifest) -> Result<()> {
    let restored = artifact.with_extension("roundtrip.db");
    {
        let file = std::fs::File::open(artifact)
            .with_context(|| format!("打开 {} 失败", artifact.display()))?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut bytes = Vec::new();
        decoder
            .read_to_end(&mut bytes)
            .with_context(|| format!("解压 {} 失败", artifact.display()))?;
        if bytes.len() as u64 != manifest.uncompressed_bytes {
            let _ = std::fs::remove_file(&restored);
            bail!(
                "解压后 {} 字节，manifest 记录 {} 字节",
                bytes.len(),
                manifest.uncompressed_bytes
            );
        }
        std::fs::write(&restored, &bytes)?;
    }

    let outcome = (|| -> Result<()> {
        let meta = read_meta(&restored)?;
        let mismatches = [
            (
                "schema_version",
                meta.schema_version.to_string(),
                manifest.schema_version.to_string(),
            ),
            (
                "corpus_version",
                meta.corpus_version.clone(),
                manifest.corpus_version.clone(),
            ),
            (
                "source_manifest_sha256",
                meta.source_manifest_sha256.clone(),
                manifest.source_manifest_sha256.clone(),
            ),
            (
                "poem_count",
                meta.poem_count.to_string(),
                manifest.record_count.to_string(),
            ),
            (
                "shipped_scope",
                meta.shipped_scope.clone(),
                manifest.shipped_scope.clone(),
            ),
            (
                "derived_indexes",
                meta.derived_indexes.clone(),
                manifest.derived_indexes.clone(),
            ),
        ]
        .into_iter()
        .filter(|(_, actual, expected)| actual != expected)
        .map(|(field, actual, expected)| {
            format!("{field}：工件 `{actual}` vs manifest `{expected}`")
        })
        .collect::<Vec<_>>();
        if !mismatches.is_empty() {
            bail!(
                "manifest 与工件不一致（manifest 可能描述的是上一次构建）：{}",
                mismatches.join("；")
            );
        }
        let connection = Connection::open(&restored)?;
        yunjian_corpus::db::assert_no_diagnostic_tables(&connection)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&restored);
    outcome?;
    emit("  解压回读：工件内 corpus_meta 与 manifest 逐项一致，且无诊断表");
    Ok(())
}

fn sha256_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests;
