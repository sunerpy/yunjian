//! `xtask assets-manifest`：把两件独立工件的清单合成一份 `assets_manifest.json`。
//!
//! # 为什么需要一个专门的子命令
//!
//! 语料工件的清单由 `corpus-package` 产出，随包赏析种子的清单由 `pregenerate` 产出。
//! 两者互不相识，而应用侧（`yunjian_core::assets::AssetsManifest`）读的是**第三种**
//! 形状：一份同时声明两件工件的 URL、摘要与版本的清单。这个转换必须有代码承担，
//! 否则它就会变成发布流程里一段手写 JSON——而手写 JSON 的失败模式是**发布成功、
//! 用户侧 `corpus fetch` 失败**，代价全部落在用户那边。
//!
//! # 门禁在写盘之前
//!
//! 产出物先经 [`AssetsManifest::parse`] ——也就是应用运行期真正用的那个解析器——
//! 解析一遍才写盘。这条顺序是本模块存在的主要理由：应用会拒绝的清单在这里就发不出去，
//! 而不是等到用户执行 `corpus fetch` 时才发现。
//!
//! # 为什么还要单独核对 `corpus_version`
//!
//! todo 76 的兼容矩阵在**导入时**拒绝 `corpus_version` 与语料不一致的种子。那道检查是
//! 用户侧的最后一道防线，不是发布侧的第一道：一份两个版本对不上的清单在发布时就是错的，
//! 发出去只会让每一个用户各自撞一次同一个错误。因此这里提前比一次，对不上直接中止发布。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use yunjian_core::assets::{
    ASSETS_MANIFEST_FILE_NAME, AppreciationSeedManifest, AssetsManifest, CorpusAssetManifest,
};

use crate::verify_sources::emit;

/// `corpus-package` 产出的语料清单里本模块要用的字段。
///
/// 刻意不 `deny_unknown_fields`：语料清单还带体积实测与预算依据等一大片元数据，
/// 那些与本转换无关，多出来的键不该让发布中止。
#[derive(Debug, Deserialize)]
struct CorpusPackageManifest {
    schema_version: u32,
    corpus_version: String,
    artifact_name: String,
    sha256: String,
}

/// `pregenerate` 产出的数据集清单里本模块要用的字段。
#[derive(Debug, Deserialize)]
struct DatasetManifestFields {
    template_version: String,
    record_count: usize,
    appreciations_sha256: String,
    corpus_version: String,
    generation_executed: bool,
    not_executed_reason: Option<String>,
}

/// 合成并校验统一资产清单。
pub fn run(
    corpus_manifest: PathBuf,
    seed_manifest: PathBuf,
    seed: PathBuf,
    base_url: String,
    out: Option<PathBuf>,
) -> Result<()> {
    emit("== 统一资产清单合成 ==");

    let corpus: CorpusPackageManifest = read_json(&corpus_manifest)?;
    let dataset: DatasetManifestFields = read_json(&seed_manifest)?;

    // 种子文件的摘要**当场重算**，不信清单里那一行。清单与文件由同一次运行写出，
    // 但发布流程会把它们分别搬运——只信清单等于放过「搬错了一个文件」这类事故。
    let seed_bytes = std::fs::read(&seed).with_context(|| format!("读 {} 失败", seed.display()))?;
    let seed_digest = sha256_hex(&seed_bytes);
    if seed_digest != dataset.appreciations_sha256 {
        bail!(
            "{} 的实际摘要 {seed_digest} 与清单声明 {} 不同；发布中止",
            seed.display(),
            dataset.appreciations_sha256
        );
    }

    if corpus.corpus_version != dataset.corpus_version {
        bail!(
            "语料工件声明 corpus_version {}，赏析种子声明 {}；两者必须一致，\
             否则导入时会被兼容矩阵拒绝",
            corpus.corpus_version,
            dataset.corpus_version
        );
    }

    let seed_name = seed
        .file_name()
        .and_then(|name| name.to_str())
        .context("种子文件名不是合法 UTF-8")?
        .to_owned();
    let base = base_url.trim_end_matches('/').to_owned();

    let manifest = AssetsManifest {
        corpus: CorpusAssetManifest {
            url: format!("{base}/{}", corpus.artifact_name),
            sha256: corpus.sha256.clone(),
            corpus_version: corpus.corpus_version.clone(),
            schema_version: corpus.schema_version,
        },
        appreciation_seed: AppreciationSeedManifest {
            url: format!("{base}/{seed_name}"),
            sha256: seed_digest,
            template_version: dataset.template_version.clone(),
            corpus_version: dataset.corpus_version.clone(),
            record_count: dataset.record_count,
        },
    };

    let mut json = serde_json::to_string_pretty(&manifest)?;
    json.push('\n');

    // 用应用运行期那个解析器验一遍再写盘。这里失败意味着这份清单发出去必然让
    // 每一个用户的 `corpus fetch` 失败，所以它是发布门禁而不是自检。
    AssetsManifest::parse(json.as_bytes())
        .map_err(|error| anyhow::anyhow!("应用侧解析器拒绝本清单：{error}"))?;

    let out = out.unwrap_or_else(|| {
        corpus_manifest
            .parent()
            .unwrap_or(Path::new("."))
            .join(ASSETS_MANIFEST_FILE_NAME)
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &json).with_context(|| format!("写 {} 失败", out.display()))?;

    emit(&format!(
        "语料：{}（schema {}，corpus {}）",
        manifest.corpus.url, manifest.corpus.schema_version, manifest.corpus.corpus_version
    ));
    emit(&format!(
        "种子：{}（模板 {}，{} 条）",
        manifest.appreciation_seed.url,
        manifest.appreciation_seed.template_version,
        manifest.appreciation_seed.record_count
    ));
    emit(&format!("已写出 {}", out.display()));

    // 未执行推理的数据集照常发布——管线、门禁与溯源字段都是真的——但必须在发布日志里
    // 说清楚正文是未生成标记，否则下游会把一份占位当成随包赏析。
    if dataset.generation_executed {
        emit("数据集：generation_executed=true（正文是模型输出）");
    } else {
        emit(&format!(
            "数据集：generation_executed=false，正文是未生成标记；原因：{}",
            dataset
                .not_executed_reason
                .as_deref()
                .unwrap_or("清单未给出原因")
        ));
        emit("NOT EXECUTED：本清单声明的种子不是模型输出，不得当成随包赏析对外承诺");
    }

    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("读 {} 失败", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("解析 {} 失败", path.display()))
}

fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}
