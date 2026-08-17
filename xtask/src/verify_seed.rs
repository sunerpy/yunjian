//! `xtask verify-seed`：发布链路的赏析种子门禁。
//!
//! # 它补的是哪一个真实缺陷
//!
//! `corpus-release.yml` 曾在 CI 里直接跑 `pregenerate` 且**刻意不给 `--endpoint`**——CI 里
//! 没有开放权重推理运行时。那一步如实降级：清单标 `generation_executed=false`、每条正文写
//! 未生成标记。降级本身是对的，错的是**下游照常把它发出去**：`corpus-v0.1.0` 的
//! `appreciations.json` 因此是 16 条全占位，而移动端首启正是从那里取种子。
//!
//! 所以缺的不是「更严的生成门禁」——生成期允许降级是刻意的，本地开发与 CI 的管线校验都
//! 需要它。缺的是**发布这一环**的一道判据。本子命令就是那道判据。
//!
//! # 为什么种子由锁指向，而不是在 CI 里现生成
//!
//! 在 CI 里现拉一个 7B 权重跑推理有两个不可接受的后果：发布时长失控（本机 CPU 实测
//! 32–66 秒/条，且要先下载几个 GiB 权重），以及**发布产物由哪一次推理产生不可追溯**——每次
//! 重跑都会得到一份新的、无人看过的正文。
//!
//! 于是链路改成：种子在一台有运行时的机器上生成一次，上传到一个独立的 `appreciation-seed-v*`
//! Release，并把指针与摘要写进仓库里的 [`LOCK_FILE`]。发布流只做「按锁下载 + 校验」，
//! 时长是一次下载，而「这些字节从哪来」由 `git log dataset/appreciations.lock.toml` 回答。
//!
//! 锁**不是**种子本身，所以 `.gitignore` 里「产物不入库」那条规则没有被动摇：入库的是
//! 一份三十行的指针，发出去的字节仍然只存在于 Release 资产里。
//!
//! # 不可伪造性建立在「重算」而不是「相信」上
//!
//! 锁里的每个字段都可以被手写，`generation_executed: true` 也可以。所以本子命令不采信任何
//! 自述字段，而是把能重算的都重算一遍：
//!
//! 1. 种子与清单的字节摘要必须等于锁里那两行——锁在 git 里，改它要留一次提交；
//! 2. 覆盖集与每首的 `grounding_digest` 由**这次刚建好的语料**经
//!    [`crate::pregenerate::resolve_coverage_requests`]（生成期用的同一个实现）重算，
//!    逐项相等才放行；
//! 3. 其余逐条判据交给 [`ensure_releasable`]，包括占位标记、正文长度下界、正文互不重复、
//!    权重摘要在场，以及生成期那套逐条门禁。
//!
//! 结论如实说清楚：这套东西**不是**密码学证明。没有可信签名方时，「这段文字出自某个 7B
//! 权重」本身无法被证明。它做到的是把最便宜的伪造形态逐个堵死——倒一份占位、把一段话复制
//! 16 份、写一堆存根、换一份对不上语料的旧种子——并让剩下那条路（真的把整条渲染管线跑通，
//! 然后逐首手写赏析）贵到与「老实生成」不是同一个量级。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use yunjian_ai::pregenerate::{
    DatasetManifest, PregeneratedRecord, ReleaseExpectation, ensure_disclosure, ensure_releasable,
    sha256_hex,
};
use yunjian_ai::provider::APPRECIATION_TEMPLATE_VERSION;

use crate::verify_sources::emit;

/// 种子锁的文件名。位于 `dataset/` 下，与披露文件同目录。
pub const LOCK_FILE: &str = "appreciations.lock.toml";

/// 发布链路唯一认得的种子指针。
///
/// `deny_unknown_fields`：多一个键说明写锁的那份代码与读锁的这份不是同一个版本，
/// 而锁的用途正是「两边对同一份字节达成一致」，此时静默忽略比中止危险得多。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedLock {
    /// 承载种子资产的 Release tag，形如 `appreciation-seed-v1`。
    pub seed_tag: String,
    /// `appreciations.json` 的 SHA-256。
    pub seed_sha256: String,
    /// `appreciations.manifest.json` 的 SHA-256。
    pub manifest_sha256: String,
    /// 记录条数。
    pub record_count: usize,
    /// 生成时所对的语料版本。
    pub corpus_version: String,
    /// 生成时的提示词模板版本。
    pub template_version: String,
    /// 权重标识。
    pub model: String,
    /// 权重许可（SPDX）。
    pub model_license: String,
    /// 本地运行时标识。
    pub provider: String,
    /// 运行时自报的权重摘要（十六进制）。
    pub model_digest: String,
    /// 生成完成时的 Unix 秒数。
    pub generated_at: u64,
}

impl SeedLock {
    /// 从 TOML 读出一份锁。
    ///
    /// # Errors
    ///
    /// 文件读不到或不是本结构的形状时返回错误。
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "读取种子锁 {} 失败。发布链路只认锁指向的那份种子，缺锁即无从判断该发什么",
                path.display()
            )
        })?;
        toml::from_str(&text).with_context(|| format!("解析种子锁 {} 失败", path.display()))
    }

    /// 写出一份锁，带上说明它是什么的文件头。
    ///
    /// # Errors
    ///
    /// 序列化或写盘失败时返回错误。
    pub fn write(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("序列化种子锁失败")?;
        let text = format!(
            "# 随包赏析种子锁。**这不是种子本身**，而是指向种子的指针与摘要。\n\
             #\n\
             # 种子（`appreciations.json`）由一台装有开放权重运行时的机器生成一次，\n\
             # 上传到 `seed_tag` 这个 Release；发布流按本文件下载并校验，不在 CI 里跑推理。\n\
             # 因此「发布出去的赏析由哪一次推理产生」由本文件的 git 历史回答。\n\
             #\n\
             # 由 `cargo run -p xtask -- pregenerate --endpoint <URL> --seed-tag <TAG>` 写出，\n\
             # 由 `cargo run -p xtask -- verify-seed` 校验。手改本文件不会让伪造的种子通过：\n\
             # 门禁会拿待发布语料重算覆盖集与每首的事实块摘要。\n\
             \n\
             {body}"
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text).with_context(|| format!("写出 {} 失败", path.display()))
    }
}

/// 校验一份已下载的种子能否随本次发布发出去。
///
/// # Errors
///
/// 锁读不出、摘要对不上、披露不完整、覆盖集或事实块重算不上、或
/// [`ensure_releasable`] 的任一条不成立时返回错误。
pub fn run(
    lock_path: PathBuf,
    seed: PathBuf,
    seed_manifest: PathBuf,
    corpus_db: PathBuf,
    disclosure: PathBuf,
    print_seed_tag: bool,
) -> Result<()> {
    // 发布流必须先知道从哪个 tag 下载，才可能有东西可校验，于是「读锁」这件事在下载之前就
    // 需要一次。它由本子命令承担而不是让工作流 grep 一遍 TOML：那会是锁的第二个解析器，
    // 而两个解析器对同一份文件的分歧正是锁想消除的东西。
    if print_seed_tag {
        emit(&SeedLock::read(&lock_path)?.seed_tag);
        return Ok(());
    }

    emit("== 随包赏析种子发布门禁 ==");

    let lock = SeedLock::read(&lock_path)?;
    emit(&format!(
        "种子锁：tag={} model={} digest={} 语料={} 模板={} {} 条",
        lock.seed_tag,
        lock.model,
        lock.model_digest,
        lock.corpus_version,
        lock.template_version,
        lock.record_count
    ));

    // 披露先校验。发布链路里不再跑 `pregenerate`，那道披露门禁若不在这里接上就整条消失了，
    // 而「缺披露不得分发」是一条法律约束，不是流程偏好。
    let disclosure_text = std::fs::read_to_string(&disclosure)
        .with_context(|| format!("读取披露文件 {} 失败", disclosure.display()))?;
    ensure_disclosure(&disclosure_text).with_context(|| {
        format!(
            "{} 的披露不完整；缺披露的数据集不得发布",
            disclosure.display()
        )
    })?;
    emit(&format!("披露校验通过：{}", disclosure.display()));

    let seed_bytes =
        std::fs::read(&seed).with_context(|| format!("读取种子 {} 失败", seed.display()))?;
    let seed_digest = sha256_hex(&seed_bytes);
    let manifest_bytes = std::fs::read(&seed_manifest)
        .with_context(|| format!("读取种子清单 {} 失败", seed_manifest.display()))?;
    let manifest_digest = sha256_hex(&manifest_bytes);

    for (what, actual, locked) in [
        ("种子", seed_digest.as_str(), lock.seed_sha256.as_str()),
        (
            "种子清单",
            manifest_digest.as_str(),
            lock.manifest_sha256.as_str(),
        ),
    ] {
        if actual != locked {
            bail!(
                "下载到的{what}摘要 {actual} 与锁里声明的 {locked} 不同。\
                 要么上传的字节不是生成时那一份，要么锁被改过而没重新生成——\
                 两种情况都不得继续发布"
            );
        }
    }
    emit("种子与清单的字节摘要都与锁一致");

    let manifest: DatasetManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("解析 {} 失败", seed_manifest.display()))?;
    let records: Vec<PregeneratedRecord> = serde_json::from_slice(&seed_bytes)
        .with_context(|| format!("解析 {} 失败", seed.display()))?;

    // 锁与清单必须逐项同意。它们由同一次运行写出，之后各走各的路（锁进 git、清单进 Release），
    // 因此这一步逮的是「两条搬运路径里有一条被换过」。
    for (field, locked, declared) in [
        ("model", lock.model.as_str(), manifest.model.as_str()),
        (
            "model_license",
            lock.model_license.as_str(),
            manifest.model_license.as_str(),
        ),
        (
            "provider",
            lock.provider.as_str(),
            manifest.provider.as_str(),
        ),
        (
            "model_digest",
            lock.model_digest.as_str(),
            manifest.model_digest.as_deref().unwrap_or(""),
        ),
        (
            "template_version",
            lock.template_version.as_str(),
            manifest.template_version.as_str(),
        ),
        (
            "corpus_version",
            lock.corpus_version.as_str(),
            manifest.corpus_version.as_str(),
        ),
    ] {
        if locked != declared {
            bail!("锁声明 {field}=`{locked}`，种子清单声明 `{declared}`；两者必须一致");
        }
    }
    if lock.record_count != manifest.record_count {
        bail!(
            "锁声明 {} 条，种子清单声明 {} 条",
            lock.record_count,
            manifest.record_count
        );
    }

    // 覆盖集与事实块摘要由**这次要发的语料**重算。工作区留到判定结束后再删：
    // `AppreciationRequest` 借着子集库活着。
    let workspace = corpus_db
        .parent()
        .unwrap_or(Path::new("."))
        .join(".verify-seed");
    let resolved =
        crate::pregenerate::resolve_coverage_requests(&corpus_db, &workspace, &lock.model, None)?;
    let grounding = resolved.grounding();
    emit(&format!(
        "已由待发布语料重算覆盖集：{} 首（筛选途径 {}，语料 {}）",
        grounding.len(),
        resolved.selector.as_str(),
        resolved.corpus_version
    ));

    let verdict = ensure_releasable(
        &manifest,
        &records,
        &ReleaseExpectation {
            corpus_version: &resolved.corpus_version,
            template_version: APPRECIATION_TEMPLATE_VERSION,
            grounding: &grounding,
            seed_sha256: &seed_digest,
        },
    );
    let _ = std::fs::remove_dir_all(&workspace);
    verdict.with_context(|| {
        format!(
            "{} 不得随本次发布发出去。生成期允许如实降级出一份占位，发布期不允许把它发给用户",
            seed.display()
        )
    })?;

    let shortest = records
        .iter()
        .map(|record| record.text.chars().count())
        .min()
        .unwrap_or_default();
    emit(&format!(
        "门禁通过：{} 条真实模型输出（最短正文 {shortest} 字），\
         权重 {}（摘要 {}，许可 {}，运行时 {}）",
        records.len(),
        manifest.model,
        lock.model_digest,
        manifest.model_license,
        manifest.provider
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> SeedLock {
        SeedLock {
            seed_tag: "appreciation-seed-v1".to_owned(),
            seed_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            record_count: 16,
            corpus_version: "0.1.0".to_owned(),
            template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
            model: "deepseek-r1:7b".to_owned(),
            model_license: "MIT".to_owned(),
            provider: "ollama".to_owned(),
            model_digest: "c".repeat(64),
            generated_at: 1_770_000_000,
        }
    }

    #[test]
    fn a_lock_survives_a_write_then_read_round_trip() {
        let dir = std::env::temp_dir().join(format!("yunjian-seed-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(LOCK_FILE);
        lock().write(&path).expect("写锁应当成功");
        assert_eq!(SeedLock::read(&path).expect("读锁应当成功"), lock());
        let text = std::fs::read_to_string(&path).expect("锁应当可读");
        assert!(
            text.contains("这不是种子本身"),
            "锁必须自带说明它是指针而不是种子，否则下一个人会以为产物入库了：{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_key_in_the_lock_is_rejected_rather_than_ignored() {
        let mut text = toml::to_string_pretty(&lock()).expect("序列化应当成功");
        text.push_str("\nsigned_by = \"某人\"\n");
        let error = toml::from_str::<SeedLock>(&text).expect_err("多一个键必须中止");
        assert!(
            error.to_string().contains("signed_by"),
            "拒绝理由应点名那个多出来的键：{error}"
        );
    }

    /// 仓库里那份锁必须是真跑出来的形态。
    ///
    /// 这条守的是「有人把一份没有权重摘要、或指向占位的锁提交进来」。锁在 git 里是它可追溯的
    /// 前提，而可追溯要求它本身先是完整的——摘要缺位时发布门禁会红，但那时已经在发版当天了。
    #[test]
    fn the_committed_lock_points_at_a_real_open_weight_run() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("从 xtask/ 推出仓库根目录")
            .join("dataset")
            .join(LOCK_FILE);
        let lock = SeedLock::read(&path).unwrap_or_else(|error| {
            panic!(
                "仓库里必须有一份可解析的种子锁（{}）：{error:#}",
                path.display()
            )
        });

        yunjian_ai::pregenerate::OpenWeightModel::new(
            &lock.model,
            &lock.model_license,
            &lock.provider,
        )
        .expect("锁里的权重配置必须过开放权重门禁");
        assert_eq!(
            lock.template_version, APPRECIATION_TEMPLATE_VERSION,
            "锁按模板 {} 生成，本代码用模板 {}；改提示词后种子必须重生成并换锁",
            lock.template_version, APPRECIATION_TEMPLATE_VERSION
        );
        for (field, value) in [
            ("model_digest", lock.model_digest.as_str()),
            ("seed_sha256", lock.seed_sha256.as_str()),
            ("manifest_sha256", lock.manifest_sha256.as_str()),
        ] {
            assert_eq!(
                value.len(),
                64,
                "锁的 {field} 必须是 64 位十六进制，实际 `{value}`"
            );
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "锁的 {field} 必须是小写十六进制，实际 `{value}`"
            );
        }
        assert!(
            lock.record_count > 0,
            "锁声明零条记录；空种子发出去等于没有随包赏析"
        );
        assert!(
            lock.seed_tag.starts_with("appreciation-seed-v"),
            "种子 tag 必须在自己的命名空间里，否则它会触发 `corpus-v*` 那条发布流：{}",
            lock.seed_tag
        );
    }
}
