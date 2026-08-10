//! `xtask verify-sources`：上游数据源许可校验闸门。
//!
//! 校验粒度是**逐资产**而非逐仓库。一份仓库级 LICENSE 只授予该仓库自身整理工作的
//! 权利，无法为它抓取或转录来的内容授权，所以「仓库是 MIT」不能推出「里面的文件都能用」。

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MANIFEST: &str = "corpus/sources.toml";
const DENYLIST: &str = "corpus/DENYLIST.md";
const DENYLIST_SECTION: &str = "## 拒绝清单";

const SPDX_ALLOW: [&str; 5] = ["MIT", "Apache-2.0", "BSD-3-Clause", "CC0-1.0", "Unlicense"];

/// 必须出现在 `DENYLIST.md` 里的标识符。删掉任何一条以便放行某个源，构建立刻失败——
/// 否则「维护一份拒绝清单」就只是自觉行为，而不是可执行的约束。
const REQUIRED_DENYLIST: [&str; 14] = [
    "huajianji",
    "VMIJUNV",
    "xcc3641/chinese-gushiwen",
    "Provinm/chinese-poetry-simplified",
    "THUNLP-AIPoet",
    "byj233/ChinesePoetryLibrary",
    "StewartXiang/poetry_with_labels",
    "sheepzh/poetry",
    "Poetry_CN",
    "yht050511/gushiwen",
    "MCGA",
    "jkak/pingShuiYun",
    "caoxingyu/chinese-gushiwen",
    "javayhu/poetry",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(rename = "source")]
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    name: String,
    url: String,
    git_rev: String,
    license: String,
    license_url: String,
    license_sha256: String,
    license_file: String,
    retrieved_at: String,
    usage: Usage,
    #[serde(default)]
    note: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    path: String,
    underlying_work: String,
    license_class: LicenseClass,
    shippable: bool,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Usage {
    Text,
    Rhyme,
    Commentary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LicenseClass {
    PublicDomain,
    Permissive,
    Unverified,
}

impl LicenseClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::PublicDomain => "public_domain",
            Self::Permissive => "permissive",
            Self::Unverified => "unverified",
        }
    }
}

impl Usage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Rhyme => "rhyme",
            Self::Commentary => "commentary",
        }
    }
}

/// 本子命令唯一的 stdout 出口。
///
/// `xtask` 是开发工具（`publish = false`，永不进入分发产物），终端报告就是它的产品，
/// 所以这里的 stdout 是正当的。但豁免仍然收在**一个函数**上而不是整个 crate：
/// crate 级豁免会让后续每个新增子命令都能随手打印，收敛不回来。
///
#[allow(clippy::print_stdout)]
fn emit(line: &str) {
    println!("{line}");
}

#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    fn push(&mut self, what: &str, why: String) {
        self.0.push(format!("{what}: {why}"));
    }

    fn into_result(self) -> Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let mut msg = format!("许可校验失败，共 {} 项：\n", self.0.len());
        for f in &self.0 {
            let _ = writeln!(msg, "  FAIL  {f}");
        }
        bail!(msg)
    }
}

pub fn run(offline: bool) -> Result<()> {
    let root = repo_root()?;
    let manifest_path = root.join(MANIFEST);
    let denylist_path = root.join(DENYLIST);

    let manifest: Manifest = toml::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("读取 {} 失败", manifest_path.display()))?,
    )
    .with_context(|| format!("解析 {} 失败", manifest_path.display()))?;

    let denylist = parse_denylist(&denylist_path)?;

    let mut fails = Failures::default();

    for id in REQUIRED_DENYLIST {
        if !denylist.iter().any(|d| d == id) {
            fails.push(
                DENYLIST,
                format!("必须列入拒绝清单的 `{id}` 不见了，不允许从清单里删条目"),
            );
        }
    }

    emit(&format!(
        "清单 {}：{} 个源，拒绝清单 {} 条{}",
        MANIFEST,
        manifest.sources.len(),
        denylist.len(),
        if offline { "（离线模式）" } else { "" }
    ));

    let mut seen_names = BTreeSet::new();
    let mut shippable_assets = 0usize;
    let mut withheld_assets = 0usize;

    for src in &manifest.sources {
        if !seen_names.insert(src.name.clone()) {
            fails.push(&src.name, "清单里出现重名的 source".to_owned());
        }

        check_source_identity(src, &mut fails);
        check_denylist(src, &denylist, &mut fails);

        match license_bytes(src, &root, offline) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                if actual == src.license_sha256 {
                    emit(&format!(
                        "  OK   源 {}  rev {}  {}  LICENSE {} 字节 sha256 {}  用途 {}",
                        src.name,
                        src.git_rev,
                        src.license,
                        bytes.len(),
                        &actual[..16],
                        src.usage.as_str()
                    ));
                } else {
                    fails.push(
                        &src.name,
                        format!(
                            "LICENSE 摘要不符 —— 清单记录 {}，实际 {}（{}）",
                            src.license_sha256, actual, src.license_url
                        ),
                    );
                }
            }
            Err(e) => fails.push(&src.name, format!("取不到 LICENSE，无法核实许可：{e:#}")),
        }

        let mut seen_paths = BTreeSet::new();
        for asset in &src.assets {
            if !seen_paths.insert(asset.path.clone()) {
                fails.push(
                    &format!("{}::{}", src.name, asset.path),
                    "同一个源里出现重复的 asset path".to_owned(),
                );
            }
            if asset.underlying_work.trim().is_empty() {
                fails.push(
                    &format!("{}::{}", src.name, asset.path),
                    "underlying_work 为空，底本来源必须写明".to_owned(),
                );
            }
            if asset.note.as_ref().is_some_and(|n| n.trim().is_empty()) {
                fails.push(
                    &format!("{}::{}", src.name, asset.path),
                    "note 存在但为空".to_owned(),
                );
            }
            if asset.shippable && asset.license_class == LicenseClass::Unverified {
                fails.push(
                    &format!("{}::{}", src.name, asset.path),
                    format!(
                        "license_class = \"unverified\" 却标了 shippable = true —— \
                         授权链未核实的资产不得进入分发产物（底本：{}）",
                        asset.underlying_work
                    ),
                );
            }

            if asset.shippable {
                shippable_assets += 1;
            } else {
                withheld_assets += 1;
            }
            emit(&format!(
                "    {}  资产 {}::{}  {}  底本 {}",
                if asset.shippable { "SHIP  " } else { "WITHHD" },
                src.name,
                asset.path,
                asset.license_class.as_str(),
                asset.underlying_work
            ));
        }

        if src.assets.is_empty() {
            fails.push(&src.name, "没有任何 asset，逐资产校验无从进行".to_owned());
        }
    }

    emit(&format!(
        "合计：{} 个资产可分发，{} 个资产扣留",
        shippable_assets, withheld_assets
    ));

    fails.into_result()?;
    emit("verify-sources 通过");
    Ok(())
}

fn check_source_identity(src: &Source, fails: &mut Failures) {
    if !is_full_sha1(&src.git_rev) {
        fails.push(
            &src.name,
            format!(
                "git_rev `{}` 不是 40 位小写十六进制 commit SHA —— \
                 分支名或 tag 会随上游移动，等于没有锁定",
                src.git_rev
            ),
        );
    }
    if !SPDX_ALLOW.contains(&src.license.as_str()) {
        fails.push(
            &src.name,
            format!("SPDX `{}` 不在允许列表 {:?} 内", src.license, SPDX_ALLOW),
        );
    }
    if !is_sha256_hex(&src.license_sha256) {
        fails.push(
            &src.name,
            format!(
                "license_sha256 `{}` 不是 64 位小写十六进制",
                src.license_sha256
            ),
        );
    }
    if !is_iso_date(&src.retrieved_at) {
        fails.push(
            &src.name,
            format!("retrieved_at `{}` 不是 YYYY-MM-DD", src.retrieved_at),
        );
    }
    if !src.license_url.contains(&src.git_rev) {
        fails.push(
            &src.name,
            format!(
                "license_url 里没有锁定的 revision，取到的可能是别的版本：{}",
                src.license_url
            ),
        );
    }
    if src.note.as_ref().is_some_and(|n| n.trim().is_empty()) {
        fails.push(&src.name, "note 存在但为空".to_owned());
    }
}

/// 只拿 `name` / `url` 去匹配。绝不匹配 asset 的 `path`：被拒绝的独立仓库
/// `huajianji` 与 MIT 仓库内的子目录 `五代诗词/huajianji/` 同名，匹配路径会误杀后者。
fn check_denylist(src: &Source, denylist: &[String], fails: &mut Failures) {
    let name = src.name.to_lowercase();
    let url = src.url.to_lowercase();
    for id in denylist {
        let needle = id.to_lowercase();
        if name.contains(&needle) || url.contains(&needle) {
            fails.push(
                &src.name,
                format!("命中 {DENYLIST} 的拒绝条目 `{id}`，该源不得进入清单（理由见 {DENYLIST}）"),
            );
        }
    }
}

fn license_bytes(src: &Source, root: &Path, offline: bool) -> Result<Vec<u8>> {
    let vendored_path = root.join(&src.license_file);
    let vendored = std::fs::read(&vendored_path)
        .with_context(|| format!("读取随仓保存的 LICENSE {} 失败", vendored_path.display()))?;

    if offline {
        return Ok(vendored);
    }

    let fetched = fetch(&src.license_url)?;
    if fetched != vendored {
        bail!(
            "上游 {} 的字节与随仓保存的 {} 不一致（{} vs {} 字节）—— \
             锁定 revision 的内容不该变化，请核对是否改了 git_rev",
            src.license_url,
            src.license_file,
            fetched.len(),
            vendored.len()
        );
    }
    Ok(fetched)
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url)
        .call()
        .with_context(|| format!("请求 {url} 失败"))?;
    let bytes = resp
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("读取 {url} 响应体失败"))?;
    if bytes.is_empty() {
        bail!("{url} 返回空响应体");
    }
    Ok(bytes)
}

fn parse_denylist(path: &Path) -> Result<Vec<String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))?;

    let mut ids = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim() == DENYLIST_SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(id) = backticked_list_item(line) {
            ids.push(id.to_owned());
        }
    }

    if ids.is_empty() {
        bail!(
            "{} 的 `{}` 一节里没有解析出任何条目，格式应为 - `标识符` —— 理由",
            path.display(),
            DENYLIST_SECTION
        );
    }
    Ok(ids)
}

fn backticked_list_item(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("- `")?;
    let end = rest.find('`')?;
    let id = &rest[..end];
    (!id.is_empty()).then_some(id)
}

fn is_full_sha1(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| b[i].is_ascii_digit())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `CARGO_MANIFEST_DIR` 指向 `xtask/`，仓库根是它的父目录。用编译期常量而不是
/// 当前工作目录，这样从任何子目录调用 `cargo run -p xtask` 都能找到清单。
fn repo_root() -> Result<PathBuf> {
    let xtask_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xtask_dir
        .parent()
        .map(Path::to_path_buf)
        .context("定位仓库根目录失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn full_sha1_rejects_branch_names_and_short_revs() {
        assert!(is_full_sha1("b8594f81a89752241442f2ce267d6f66f96704ee"));
        assert!(!is_full_sha1("master"));
        assert!(!is_full_sha1("main"));
        assert!(!is_full_sha1("v1.0.0"));
        assert!(!is_full_sha1("b8594f8"));
        assert!(!is_full_sha1("B8594F81A89752241442F2CE267D6F66F96704EE"));
        assert!(!is_full_sha1("b8594f81a89752241442f2ce267d6f66f96704eg"));
    }

    #[test]
    fn denylist_item_parser_takes_first_backticked_token() {
        assert_eq!(
            backticked_list_item("- `sheepzh/poetry` —— LICENSE 与 README 矛盾"),
            Some("sheepzh/poetry")
        );
        assert_eq!(backticked_list_item("- 没有反引号的普通条目"), None);
        assert_eq!(backticked_list_item("普通段落 `foo`"), None);
        assert_eq!(backticked_list_item("- ``"), None);
    }

    #[test]
    fn iso_date_shape() {
        assert!(is_iso_date("2026-08-10"));
        assert!(!is_iso_date("2026-8-10"));
        assert!(!is_iso_date("2026/08/10"));
        assert!(!is_iso_date("20260810"));
    }

    /// 真清单必须自洽：这条用例让 `sources.toml` 的结构错误在 `cargo test` 阶段
    /// 就暴露，而不必等到跑子命令。不联网。
    #[test]
    fn shipped_manifest_parses_and_has_no_unverified_shippable_asset() {
        let root = repo_root().expect("repo root");
        let text = std::fs::read_to_string(root.join(MANIFEST)).expect("read manifest");
        let manifest: Manifest = toml::from_str(&text).expect("parse manifest");

        assert!(!manifest.sources.is_empty());
        for src in &manifest.sources {
            assert!(is_full_sha1(&src.git_rev), "{} git_rev", src.name);
            assert!(SPDX_ALLOW.contains(&src.license.as_str()), "{}", src.name);
            assert!(!src.assets.is_empty(), "{} 无资产", src.name);
            for a in &src.assets {
                assert!(!a.underlying_work.trim().is_empty(), "{}", a.path);
                assert!(
                    !(a.shippable && a.license_class == LicenseClass::Unverified),
                    "{}::{} 未核实却标为可分发",
                    src.name,
                    a.path
                );
            }
        }
    }

    #[test]
    fn shipped_manifest_holds_no_denylisted_source() {
        let root = repo_root().expect("repo root");
        let denylist = parse_denylist(&root.join(DENYLIST)).expect("parse denylist");
        for id in REQUIRED_DENYLIST {
            assert!(denylist.iter().any(|d| d == id), "拒绝清单缺少 `{id}`");
        }

        let manifest: Manifest =
            toml::from_str(&std::fs::read_to_string(root.join(MANIFEST)).expect("read"))
                .expect("parse");
        let mut fails = Failures::default();
        for src in &manifest.sources {
            check_denylist(src, &denylist, &mut fails);
        }
        assert!(fails.into_result().is_ok());
    }

    /// 随仓保存的 LICENSE 必须与清单记录的摘要一致，否则离线校验形同虚设。
    #[test]
    fn vendored_licenses_match_recorded_hashes() {
        let root = repo_root().expect("repo root");
        let manifest: Manifest =
            toml::from_str(&std::fs::read_to_string(root.join(MANIFEST)).expect("read"))
                .expect("parse");
        for src in &manifest.sources {
            let bytes = std::fs::read(root.join(&src.license_file))
                .unwrap_or_else(|e| panic!("{} 读取 {} 失败: {e}", src.name, src.license_file));
            assert_eq!(sha256_hex(&bytes), src.license_sha256, "{}", src.name);
        }
    }
}
