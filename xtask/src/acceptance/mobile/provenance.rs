//! 真机验收报告的**溯源**：报告里的 `commit_sha` 必须真的绑到产生这份报告的那份源码。
//!
//! # 缺陷的形态（这个模块存在的理由）
//!
//! `docs/reports/mobile-qa-2026-08-17.json` 曾记 `commit_sha=8c7424fa…`，而让 Android 从
//! 9 PASS 变成 10 PASS 的修复提交在它**之后**。报告内部计数是自洽的、截图与测量值都在，
//! 唯独它声称的那份源码不是产生这些数字的那一份。
//!
//! 这不是形式主义：一份真机报告的全部价值就是「**这个版本**的代码在真机上是这个表现」。
//! sha 绑错之后，这句话退化成「某次执行产生过这些数字」，而那对判断当前代码可不可发布毫无用处。
//!
//! # 为什么不能只手改那个数字
//!
//! 手改一次只修这一次，而且把一个**可发现**的缺口（sha 与历史对不上）变成一个更难发现的
//! 伪证据（sha 看起来对，但没人能证明它对）。要修的是机制：
//!
//! 1. **写报告之前**要求被测路径在工作树里干净（`require_clean`），于是 `git rev-parse HEAD`
//!    与真正被构建的字节一致；
//! 2. 把被测源码的**内容摘要**写进报告（`collect`），于是「报告描述的是哪份代码」不再依赖
//!    任何人的记忆；
//! 3. 一条 git-free 的断言（`verify`）在每次 `cargo test` 时重算摘要并比对，源码一变即红，
//!    报告必须在新提交上重跑。
//!
//! # 为什么摘要是内容摘要而不是 git object id
//!
//! `crates/yunjian-mobile/tests/architecture.rs` 已经记过这条：`actions/checkout` 默认浅克隆，
//! 拿不到历史对象，于是任何「拿历史 commit 算对象 id」的断言在 CI 上必然失败。**一条
//! 「报告描述的是不是当前代码」的断言不该因为克隆深度而失效或通过**，所以判据扫真实文件，
//! 与 git、`.git` 目录、克隆深度全部解耦。git 只在**生成**报告时用一次（那时人在真机旁边，
//! 一定有完整仓库）。
//!
//! # 哪些路径被守住，哪些只记录
//!
//! 被守住（`TESTED_PATHS`）的是十条断言真正走过的那些字节：Android 产品源码与 instrumentation、
//! 工程与依赖声明、UniFFI 生成物与移动门面。它们一变，报告里的 PASS 就不再描述当前系统。
//!
//! 只记录不守（`RECORDED_PATHS`）的是 `Cargo.lock`：它的摘要能让审阅者看出依赖图是否同一份，
//! 但工作区里任何一个无关 crate 的依赖调整都会改它。把它纳入门禁会让**别处的正当改动**把
//! 这份报告判红，那种红没有信息量，只会训练人去忽略它。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 十条断言真正被执行的那份源码。摘要发生变化即意味着报告不再描述当前系统。
pub(crate) const TESTED_PATHS: &[&str] = &[
    "mobile/android/app/src/main",
    "mobile/android/app/src/androidTest",
    "mobile/android/app/build.gradle.kts",
    "mobile/android/build.gradle.kts",
    "mobile/android/settings.gradle.kts",
    "mobile/android/gradle.properties",
    "mobile/android/gradle/libs.versions.toml",
    "crates/yunjian-mobile/src",
    "crates/yunjian-mobile/bindings",
    "crates/yunjian-mobile/Cargo.toml",
];

/// 记录但不设门禁的路径。见模块文档最后一节。
pub(crate) const RECORDED_PATHS: &[&str] = &["Cargo.lock"];

/// 一条被测源码的溯源记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestedSource {
    /// 仓库相对路径。
    pub(crate) path: String,
    /// 该路径下的文件数。目录摘要已经覆盖增删文件，这一项只为把「文件集合变了」与
    /// 「文件内容变了」报成两句不同的话——否则新增一个文件只会得到一个无从下手的摘要不符。
    pub(crate) files: usize,
    /// 覆盖「仓库相对路径 + 文件长度 + 内容」的 SHA-256。
    ///
    /// **不等于** `sha256sum` 对单个文件的输出，不要拿后者来核对；这样算是为了让改名与
    /// 增删文件同样被抓到。与 `architecture.rs` 的受保护路径摘要同一算法。
    pub(crate) digest: String,
    /// 这条是否参与门禁。`false` 表示只作审阅材料。
    pub(crate) freshness_enforced: bool,
}

/// 扫描被测路径，产出溯源记录。顺序固定，便于逐行比对。
pub(crate) fn collect(root: &Path) -> Result<Vec<TestedSource>> {
    let mut records = Vec::with_capacity(TESTED_PATHS.len() + RECORDED_PATHS.len());
    for (path, enforced) in TESTED_PATHS
        .iter()
        .map(|path| (*path, true))
        .chain(RECORDED_PATHS.iter().map(|path| (*path, false)))
    {
        let (files, digest) = digest_path(root, path)?;
        records.push(TestedSource {
            path: path.to_owned(),
            files,
            digest,
            freshness_enforced: enforced,
        });
    }
    Ok(records)
}

/// 报告里的记录是否仍然描述当前工作树。
///
/// 只比对 `freshness_enforced` 的那些条目；`false` 的条目仍要求存在，但值可以不同
/// （否则它就不是「只记录」了，而是一条隐式门禁）。
///
/// **只在测试里执行**，因为它守的是**已落盘的那份报告**：`cargo test` 每次都重算摘要与报告
/// 比对，源码一变即红。生成期用的是 [`require_clean`] 与 [`collect`]——那时报告还没写出来，
/// 没有「已记录的值」可比。
#[cfg(test)]
pub(crate) fn verify(root: &Path, recorded: &[TestedSource]) -> Result<()> {
    let current = collect(root)?;
    if recorded.len() != current.len() {
        bail!(
            "报告记录了 {} 条被测源码，当前声明为 {} 条；\
             改动被测路径清单时必须重跑真机验收，否则报告描述的不再是当前系统",
            recorded.len(),
            current.len()
        );
    }
    for (recorded, current) in recorded.iter().zip(&current) {
        if recorded.path != current.path {
            bail!(
                "被测源码清单顺序漂移：报告第 N 条是 `{}`，当前声明是 `{}`",
                recorded.path,
                current.path
            );
        }
        if recorded.freshness_enforced != current.freshness_enforced {
            bail!(
                "`{}` 的门禁开关与声明不符（报告 {}，声明 {}）",
                recorded.path,
                recorded.freshness_enforced,
                current.freshness_enforced
            );
        }
        if !current.freshness_enforced {
            continue;
        }
        if recorded.files != current.files {
            bail!(
                "`{}` 的文件数从 {} 变成 {}：被测源码集合已变，\
                 报告里的 PASS 不再描述当前系统，须在新提交上重跑真机验收",
                recorded.path,
                recorded.files,
                current.files
            );
        }
        if recorded.digest != current.digest {
            bail!(
                "`{}` 的内容摘要与报告记录不符（报告 {}，当前 {}）：\
                 被测源码在报告之后被改过，须在新提交上重跑真机验收；\
                 **不要改报告里的摘要让这条变绿**，那会把溯源缺口变成伪证据",
                recorded.path,
                recorded.digest,
                current.digest
            );
        }
    }
    Ok(())
}

/// 写报告之前要求被测路径在工作树里干净。
///
/// 干净是 `commit_sha` 可信的前提：`git rev-parse HEAD` 报的是**提交**里的字节，而真机上跑的
/// 是**工作树**里的字节。两者不一致时那个 sha 指向的代码根本不是被测的代码——正是
/// 2026-08-17 那次绑错的一般形态。
///
/// 无 git（浅克隆、导出的源码包）时不放行也不硬失败：报不出「干净与否」这件事，就把它如实
/// 写进错误里，让操作者知道该在有完整仓库的地方生成报告。
pub(crate) fn require_clean(root: &Path) -> Result<()> {
    let mut args = vec![
        "status".to_owned(),
        "--porcelain".to_owned(),
        "--".to_owned(),
    ];
    args.extend(TESTED_PATHS.iter().map(|path| (*path).to_owned()));
    let output = Command::new("git")
        .args(&args)
        .current_dir(root)
        .output()
        .context("执行 git status 失败：真机验收报告必须在能读 git 状态的检出里生成")?;
    if !output.status.success() {
        bail!(
            "git status 退出码 {:?}：无法判定被测源码是否干净，因此不能声称报告绑定某个 commit",
            output.status.code()
        );
    }
    let dirty = String::from_utf8_lossy(&output.stdout);
    let dirty: Vec<&str> = dirty
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if !dirty.is_empty() {
        bail!(
            "被测源码在工作树里有未提交改动，报告的 commit_sha 会绑到一份**没有被测**的代码：\n{}\n\
             先提交（或撤销）这些改动，再重跑真机验收",
            dirty.join("\n")
        );
    }
    Ok(())
}

/// 目录或文件的（文件数, 摘要）。
fn digest_path(root: &Path, relative: &str) -> Result<(usize, String)> {
    let absolute = root.join(relative);
    let mut files = collect_files(&absolute)
        .with_context(|| format!("扫描被测路径 {} 失败", absolute.display()))?;
    if files.is_empty() {
        bail!(
            "被测路径 `{relative}` 下没有文件：\
             它是十条真机断言的来源之一，缺失时报告无法声称描述了哪份代码"
        );
    }
    files.sort();
    let mut hasher = Sha256::new();
    for file in &files {
        let contents = fs::read(file).with_context(|| format!("读取 {} 失败", file.display()))?;
        let name = file
            .strip_prefix(root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(contents.len().to_le_bytes());
        hasher.update(&contents);
    }
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(digest, "{byte:02x}");
    }
    Ok((files.len(), digest))
}

fn collect_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)
            .with_context(|| format!("读取目录 {} 失败", current.display()))?
        {
            let entry = entry?;
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
            } else if child.is_file() {
                files.push(child);
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        super::super::super::repo_root()
    }

    #[test]
    fn every_tested_path_exists_and_hashes_to_a_stable_digest() {
        let first = collect(&root()).expect("被测路径必须可扫描");
        let second = collect(&root()).expect("被测路径必须可扫描");
        assert_eq!(
            first, second,
            "同一棵树两次扫描必须得到同一批摘要，否则这条溯源记录不可比对"
        );
        assert_eq!(
            first.len(),
            TESTED_PATHS.len() + RECORDED_PATHS.len(),
            "记录条数必须与声明一致"
        );
        for record in &first {
            assert_eq!(record.digest.len(), 64, "摘要必须是 64 位十六进制");
            assert!(record.files > 0, "`{}` 下必须有文件", record.path);
        }
        assert!(
            first
                .iter()
                .filter(|record| record.freshness_enforced)
                .count()
                == TESTED_PATHS.len(),
            "被守住的条目数必须等于 TESTED_PATHS 的长度"
        );
    }

    #[test]
    fn a_changed_tested_source_fails_verification() {
        let root = root();
        let mut recorded = collect(&root).expect("被测路径必须可扫描");
        verify(&root, &recorded).expect("未改动时必须通过");

        // 注入：把第一条被守住的摘要改掉，等价于「被测源码在报告之后被改过」。
        recorded[0].digest = "0".repeat(64);
        let error = verify(&root, &recorded).expect_err("摘要不符必须失败");
        let message = error.to_string();
        assert!(
            message.contains(&recorded[0].path) && message.contains("重跑真机验收"),
            "判词必须指出是哪条路径、下一步该做什么：{message}"
        );
    }

    #[test]
    fn a_recorded_only_source_may_drift_without_failing() {
        let root = root();
        let mut recorded = collect(&root).expect("被测路径必须可扫描");
        let index = recorded
            .iter()
            .position(|record| !record.freshness_enforced)
            .expect("必须有只记录不守的条目");
        recorded[index].digest = "1".repeat(64);
        recorded[index].files += 7;
        verify(&root, &recorded).expect(
            "只记录的条目漂移不该判红：否则工作区里任何无关依赖调整都会把这份真机报告判红，\
             而那种红没有信息量",
        );
    }

    #[test]
    fn a_dropped_path_from_the_manifest_fails_verification() {
        let root = root();
        let mut recorded = collect(&root).expect("被测路径必须可扫描");
        recorded.pop();
        let error = verify(&root, &recorded).expect_err("清单条数不符必须失败");
        assert!(
            error.to_string().contains("被测源码"),
            "判词必须点明是被测源码清单的问题：{error}"
        );
    }
}
