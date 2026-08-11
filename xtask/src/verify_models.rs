//! `xtask verify-models`：语音模型权重许可闸门。
//!
//! 与 `verify-sources` 的关系是同构而非复用：那边校验的是**语料**上游（按 git revision
//! 锁定、按资产判定授权链），这边校验的是**权重**发布包（按压缩包 SHA-256 锁定、按
//! 证据形态判定许可）。两者的判据不同，硬凑成一个抽象只会让两边都变模糊。
//!
//! 核心设计：`license` 字段本身**不被信任**。门禁会打开随仓保存的证据文件，
//! 按证据形态核对里面真的写着那个 SPDX 标记。否则清单里写 `license = "MIT"`
//! 就成了自证，而这个 todo 存在的全部理由正是「不接受自证」。

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::verify_sources::emit;

const MANIFEST: &str = "models.toml";
const DENYLIST: &str = "models/DENYLIST.md";
const LOCKFILE: &str = "models.lock.json";
const DENYLIST_SECTION: &str = "## 拒绝清单";

/// 权重许可的允许列表。比语料那边窄：语料可以是公有领域或 CC0，权重只接受
/// MIT 与 Apache-2.0——方案原文的措辞是「无法确认为 MIT 或 Apache-2.0 的一律移出计划」。
const SPDX_ALLOW: [&str; 2] = ["MIT", "Apache-2.0"];

/// 必须出现在 `models/DENYLIST.md` 里的标识符。
///
/// 前五条是方案点名要拒的；后六条是本 todo 逐模型核实后新增的，其中
/// `SenseVoice` / `sense-voice` / `paraformer` 推翻了研究阶段「ASR 许可整体健康」的结论。
const REQUIRED_DENYLIST: [&str; 12] = [
    "matcha-icefall-zh-baker",
    "vits-zh-hf-",
    "aishell3",
    "edge-tts",
    "MCGA",
    "SenseVoice",
    "sense-voice",
    "paraformer",
    "streaming-zipformer-small-ctc-zh",
    "streaming-zipformer-zh-2025-06-30",
    "zipformer-zh-en-2023-11-22",
    "vosk-model",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    #[serde(rename = "model")]
    models: Vec<Model>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    name: String,
    kind: Kind,
    role: Role,
    url: String,
    sha256: String,
    size_bytes: u64,
    license: String,
    license_url: String,
    license_rev: String,
    license_file: String,
    license_sha256: String,
    license_evidence: Evidence,
    underlying_work: String,
    verified_at: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    bundled: Vec<Bundled>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundled {
    path: String,
    work: String,
    license: String,
    #[serde(default)]
    distribution_impact: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Asr,
    Tts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Production,
    Smoke,
}

/// 许可证据的形态。三者强度递减，但都必须是**可打开、可核对**的文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Evidence {
    /// 发布包内自带 LICENSE 文件。
    PackageLicense,
    /// 转换包不带 LICENSE，证据是原始权重发布方的 LICENSE。
    UpstreamLicense,
    /// 证据是 HuggingFace 模型卡 front-matter 里的 `license:` 声明。
    ModelCard,
}

impl Kind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Asr => "asr",
            Self::Tts => "tts",
        }
    }
}

impl Role {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Smoke => "smoke",
        }
    }
}

impl Evidence {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PackageLicense => "package_license",
            Self::UpstreamLicense => "upstream_license",
            Self::ModelCard => "model_card",
        }
    }
}

/// `models.lock.json` 的形状。存在的唯一理由：`models.toml` 是 TOML，`jq` 读不了，
/// 而验收判据与 CI 都要能用 `jq` 直接断言每一行的许可。
#[derive(Debug, Serialize)]
struct Lock {
    schema_version: u32,
    generated_by: &'static str,
    models: Vec<LockModel>,
}

#[derive(Debug, Serialize)]
struct LockModel {
    name: String,
    kind: &'static str,
    role: &'static str,
    url: String,
    sha256: String,
    size_bytes: u64,
    license: String,
    license_url: String,
    license_file: String,
    license_sha256: String,
    license_evidence: &'static str,
    underlying_work: String,
    verified_at: String,
    note: Option<String>,
    bundled: Vec<LockBundled>,
}

#[derive(Debug, Serialize)]
struct LockBundled {
    path: String,
    work: String,
    license: String,
    distribution_impact: Option<String>,
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
        let mut msg = format!("模型许可校验失败，共 {} 项：\n", self.0.len());
        for f in &self.0 {
            let _ = writeln!(msg, "  FAIL  {f}");
        }
        bail!(msg)
    }
}

pub fn run(offline: bool) -> Result<()> {
    let root = repo_root()?;
    let manifest = load_manifest(&root)?;
    let denylist = parse_denylist(&root.join(DENYLIST))?;

    let mut fails = Failures::default();

    for id in REQUIRED_DENYLIST {
        if !denylist.iter().any(|d| d.id == id) {
            fails.push(
                DENYLIST,
                format!("必须列入拒绝清单的 `{id}` 不见了，不允许从清单里删条目"),
            );
        }
    }

    if manifest.schema_version != 1 {
        fails.push(
            MANIFEST,
            format!(
                "schema_version = {} 不是 1；读这份清单的代码（todo 53 的按需下载）\
                 只认版本 1，静默接受未知版本等于放弃校验",
                manifest.schema_version
            ),
        );
    }

    emit(&format!(
        "清单 {}：{} 个模型，拒绝清单 {} 条{}",
        MANIFEST,
        manifest.models.len(),
        denylist.len(),
        if offline { "（离线模式）" } else { "" }
    ));

    let mut seen = BTreeSet::new();
    for model in &manifest.models {
        if !seen.insert(model.name.clone()) {
            fails.push(&model.name, "清单里出现重名的 model".to_owned());
        }
        check_identity(model, &mut fails);
        check_denylist(model, &denylist, &mut fails);
        check_bundled(model, &mut fails);

        match evidence_bytes(model, &root, offline) {
            Ok(bytes) => {
                let actual = sha256_hex(&bytes);
                if actual == model.license_sha256 {
                    check_evidence_declares_license(model, &bytes, &mut fails);
                    emit(&format!(
                        "  OK   {:<10} {:<11} {:<58} {:<11} 证据 {} {} 字节",
                        model.kind.as_str(),
                        model.role.as_str(),
                        model.name,
                        model.license,
                        model.license_evidence.as_str(),
                        bytes.len(),
                    ));
                } else {
                    fails.push(
                        &model.name,
                        format!(
                            "许可证据摘要不符 —— 清单记录 {}，实际 {}（{}）",
                            model.license_sha256, actual, model.license_url
                        ),
                    );
                }
            }
            Err(e) => fails.push(&model.name, format!("取不到许可证据，无法核实许可：{e:#}")),
        }

        for b in &model.bundled {
            emit(&format!(
                "    夹带 {}::{}  {}  {}",
                model.name, b.path, b.license, b.work
            ));
        }
    }

    check_coverage(&manifest, &mut fails);

    fails.into_result()?;

    let lock_path = root.join(LOCKFILE);
    write_lock(&manifest, &lock_path)?;
    emit(&format!(
        "verify-models 通过，已写出 {}（{} 行）",
        LOCKFILE,
        manifest.models.len()
    ));
    Ok(())
}

fn load_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join(MANIFEST);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
    toml::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))
}

fn check_identity(model: &Model, fails: &mut Failures) {
    // `UNVERIFIED` 会被下面的允许列表拦住，但单独给一条消息：这是方案点名要求的场景，
    // 报错要能让读者一眼看出「问题是许可没核实」，而不是「SPDX 拼错了」。
    if model.license.eq_ignore_ascii_case("UNVERIFIED") {
        fails.push(
            &model.name,
            "license = \"UNVERIFIED\" —— 许可未核实的权重不得进入清单，\
             要么补齐证据把 SPDX 写实，要么按 models/DENYLIST.md 拒绝它"
                .to_owned(),
        );
    } else if !SPDX_ALLOW.contains(&model.license.as_str()) {
        fails.push(
            &model.name,
            format!(
                "SPDX `{}` 不在允许列表 {:?} 内 —— 权重只接受 MIT 与 Apache-2.0",
                model.license, SPDX_ALLOW
            ),
        );
    }

    if !is_sha256_hex(&model.sha256) {
        fails.push(
            &model.name,
            format!("sha256 `{}` 不是 64 位小写十六进制", model.sha256),
        );
    }
    if !is_sha256_hex(&model.license_sha256) {
        fails.push(
            &model.name,
            format!(
                "license_sha256 `{}` 不是 64 位小写十六进制",
                model.license_sha256
            ),
        );
    }
    if model.size_bytes == 0 {
        fails.push(
            &model.name,
            "size_bytes 为 0，按需下载无法校验体积".to_owned(),
        );
    }
    if !is_full_sha1(&model.license_rev) {
        fails.push(
            &model.name,
            format!(
                "license_rev `{}` 不是 40 位小写十六进制 commit SHA —— \
                 `main` 之类会随上游移动，等于没锁定证据",
                model.license_rev
            ),
        );
    }
    if !model.license_url.contains(&model.license_rev) {
        fails.push(
            &model.name,
            format!(
                "license_url 里没有锁定的 revision，取到的可能是别的版本：{}",
                model.license_url
            ),
        );
    }
    if !model.url.starts_with("https://") {
        fails.push(&model.name, format!("下载地址不是 https：{}", model.url));
    }
    if !model.url.contains(&model.name) {
        fails.push(
            &model.name,
            format!("下载地址里不含模型名，清单与产物可能对不上：{}", model.url),
        );
    }
    if !is_iso_date(&model.verified_at) {
        fails.push(
            &model.name,
            format!("verified_at `{}` 不是 YYYY-MM-DD", model.verified_at),
        );
    }
    if model.underlying_work.trim().is_empty() {
        fails.push(
            &model.name,
            "underlying_work 为空 —— 原始权重是谁训练的、许可链怎么走过来的，必须写明".to_owned(),
        );
    }
    if model.note.as_ref().is_some_and(|n| n.trim().is_empty()) {
        fails.push(&model.name, "note 存在但为空".to_owned());
    }
    if model.license_evidence == Evidence::UpstreamLicense && model.note.is_none() {
        fails.push(
            &model.name,
            "license_evidence = \"upstream_license\" 却没有 note —— \
             「转换包自己不带许可、证据来自原始权重方」这条链必须写出来，不能让读者自己推"
                .to_owned(),
        );
    }
}

/// 证据文件里必须真的出现对应的 SPDX 标记。
///
/// 这是本闸门与「相信清单里写了什么」的分界线：没有这一步，`license = "MIT"`
/// 配一份 Apache-2.0 的证据文件也能通过，摘要校验只能证明文件没被改，
/// 证明不了它说的是哪个许可。
fn check_evidence_declares_license(model: &Model, bytes: &[u8], fails: &mut Failures) {
    let text = String::from_utf8_lossy(bytes);
    let ok = match model.license_evidence {
        Evidence::ModelCard => {
            let needle = format!("license: {}", model.license.to_lowercase());
            text.lines().any(|l| l.trim() == needle)
        }
        Evidence::PackageLicense | Evidence::UpstreamLicense => match model.license.as_str() {
            "MIT" => {
                text.contains("MIT License")
                    || text.contains("Permission is hereby granted, free of charge")
            }
            "Apache-2.0" => text.contains("Apache License") && text.contains("Version 2.0"),
            _ => false,
        },
    };
    if !ok {
        fails.push(
            &model.name,
            format!(
                "证据文件 {} 里找不到 `{}` 的声明（证据形态 {}）—— \
                 清单字段与证据内容不符，摘要对得上不等于许可对得上",
                model.license_file,
                model.license,
                model.license_evidence.as_str()
            ),
        );
    }
}

fn check_bundled(model: &Model, fails: &mut Failures) {
    for b in &model.bundled {
        if b.path.trim().is_empty() || b.work.trim().is_empty() {
            fails.push(
                &format!("{}::{}", model.name, b.path),
                "夹带项的 path 与 work 都不能为空".to_owned(),
            );
        }
        // 夹带项本身不受 SPDX_ALLOW 约束——GPL-3.0 的 espeak-ng 数据是既成事实，
        // 拒绝它等于拒绝全部 sherpa-onnx 的 TTS。要求的是把分发影响写下来。
        if !SPDX_ALLOW.contains(&b.license.as_str())
            && b.distribution_impact
                .as_ref()
                .is_none_or(|s| s.trim().is_empty())
        {
            fails.push(
                &format!("{}::{}", model.name, b.path),
                format!(
                    "夹带了 `{}` 许可的产物却没写 distribution_impact —— \
                     分发影响不能只活在某个人的记忆里",
                    b.license
                ),
            );
        }
    }
}

/// 清单必须至少各有一个投产用的 ASR 与 TTS。
///
/// 存在的理由：逐模型拒绝做到最后，很可能把某一类**全部**拒完。那种情况下
/// 命令不该「通过」，它该指出语音功能已经没有可用模型了。
fn check_coverage(manifest: &Manifest, fails: &mut Failures) {
    for kind in [Kind::Asr, Kind::Tts] {
        let n = manifest
            .models
            .iter()
            .filter(|m| m.kind == kind && m.role == Role::Production)
            .count();
        if n == 0 {
            fails.push(
                MANIFEST,
                format!(
                    "没有任何 role = \"production\" 的 {} 模型 —— \
                     逐模型拒绝之后这一类已经空了，语音功能无模型可用",
                    kind.as_str()
                ),
            );
        }
    }
}

/// 拒绝命中时把**理由原文**一起报出来，不只给一个文件名。
///
/// 理由是可操作性：读者看到「命中拒绝条目，详见 DENYLIST.md」还得再去翻文件，
/// 而报错本身应当足以让人判断该不该改清单。理由文本就存在拒绝清单的同一行里，
/// 顺手带出来的成本为零。
fn check_denylist(model: &Model, denylist: &[DenyEntry], fails: &mut Failures) {
    let name = model.name.to_lowercase();
    let url = model.url.to_lowercase();
    for entry in denylist {
        let needle = entry.id.to_lowercase();
        if name.contains(&needle) || url.contains(&needle) {
            fails.push(
                &model.name,
                format!(
                    "命中 {DENYLIST} 的拒绝条目 `{}`，该模型不得进入清单。理由：{}",
                    entry.id, entry.reason
                ),
            );
        }
    }
}

fn evidence_bytes(model: &Model, root: &Path, offline: bool) -> Result<Vec<u8>> {
    let vendored_path = root.join(&model.license_file);
    let vendored = std::fs::read(&vendored_path)
        .with_context(|| format!("读取随仓保存的许可证据 {} 失败", vendored_path.display()))?;

    if offline {
        return Ok(vendored);
    }

    let fetched = fetch(&model.license_url)?;
    if fetched != vendored {
        bail!(
            "上游 {} 的字节与随仓保存的 {} 不一致（{} vs {} 字节）—— \
             锁定 revision 的内容不该变化，请核对是否改了 license_rev",
            model.license_url,
            model.license_file,
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

fn write_lock(manifest: &Manifest, path: &Path) -> Result<()> {
    let lock = Lock {
        schema_version: manifest.schema_version,
        generated_by: "xtask verify-models",
        models: manifest
            .models
            .iter()
            .map(|m| LockModel {
                name: m.name.clone(),
                kind: m.kind.as_str(),
                role: m.role.as_str(),
                url: m.url.clone(),
                sha256: m.sha256.clone(),
                size_bytes: m.size_bytes,
                license: m.license.clone(),
                license_url: m.license_url.clone(),
                license_file: m.license_file.clone(),
                license_sha256: m.license_sha256.clone(),
                license_evidence: m.license_evidence.as_str(),
                underlying_work: m.underlying_work.clone(),
                verified_at: m.verified_at.clone(),
                note: m.note.clone(),
                bundled: m
                    .bundled
                    .iter()
                    .map(|b| LockBundled {
                        path: b.path.clone(),
                        work: b.work.clone(),
                        license: b.license.clone(),
                        distribution_impact: b.distribution_impact.clone(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&lock).context("序列化 models.lock.json 失败")?;
    json.push('\n');
    std::fs::write(path, json).with_context(|| format!("写出 {} 失败", path.display()))
}

/// 拒绝清单的一条：标识符加上它的理由原文。
#[derive(Debug, Clone)]
struct DenyEntry {
    id: String,
    reason: String,
}

fn parse_denylist(path: &Path) -> Result<Vec<DenyEntry>> {
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
        if let Some((id, reason)) = backticked_list_item(line) {
            ids.push(DenyEntry {
                id: id.to_owned(),
                reason: reason.to_owned(),
            });
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

/// 解析 ``- `标识符` —— 理由`` 一行，返回标识符与理由原文。
fn backticked_list_item(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("- `")?;
    let end = rest.find('`')?;
    let id = &rest[..end];
    if id.is_empty() {
        return None;
    }
    let reason = rest[end + 1..]
        .trim_start()
        .trim_start_matches("——")
        .trim_start_matches('—')
        .trim();
    Some((id, reason))
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

    fn shipped() -> Manifest {
        load_manifest(&repo_root().expect("repo root")).expect("清单可解析")
    }

    /// 真清单必须自洽，且不联网就能查出结构问题。
    #[test]
    fn shipped_manifest_parses_and_only_holds_allowed_licenses() {
        let manifest = shipped();
        assert_eq!(manifest.schema_version, 1);
        assert!(!manifest.models.is_empty());
        for m in &manifest.models {
            assert!(
                SPDX_ALLOW.contains(&m.license.as_str()),
                "{} 的许可 {} 不在允许列表",
                m.name,
                m.license
            );
            assert!(is_sha256_hex(&m.sha256), "{}", m.name);
            assert!(is_sha256_hex(&m.license_sha256), "{}", m.name);
            assert!(is_full_sha1(&m.license_rev), "{}", m.name);
            assert!(is_iso_date(&m.verified_at), "{}", m.name);
            assert!(m.size_bytes > 0, "{}", m.name);
        }
    }

    /// 随仓保存的证据必须与清单摘要一致，否则离线校验形同虚设。
    #[test]
    fn vendored_evidence_matches_recorded_hashes_and_declares_the_license() {
        let root = repo_root().expect("repo root");
        let manifest = shipped();
        for m in &manifest.models {
            let bytes = std::fs::read(root.join(&m.license_file))
                .unwrap_or_else(|e| panic!("{} 读取 {} 失败: {e}", m.name, m.license_file));
            assert_eq!(sha256_hex(&bytes), m.license_sha256, "{}", m.name);
            let mut fails = Failures::default();
            check_evidence_declares_license(m, &bytes, &mut fails);
            assert!(fails.into_result().is_ok(), "{} 的证据未声明其许可", m.name);
        }
    }

    #[test]
    fn shipped_manifest_holds_no_denylisted_model() {
        let root = repo_root().expect("repo root");
        let denylist = parse_denylist(&root.join(DENYLIST)).expect("拒绝清单可解析");
        for id in REQUIRED_DENYLIST {
            assert!(denylist.iter().any(|d| d.id == id), "拒绝清单缺少 `{id}`");
        }
        let manifest = shipped();
        let mut fails = Failures::default();
        for m in &manifest.models {
            check_denylist(m, &denylist, &mut fails);
        }
        assert!(fails.into_result().is_ok());
    }

    /// 方案点名的失败场景之一：`license = "UNVERIFIED"` 必须被拒，且报错要指名那个模型。
    #[test]
    fn unverified_license_is_rejected_naming_the_model() {
        let mut model = sample();
        model.license = "UNVERIFIED".to_owned();
        let mut fails = Failures::default();
        check_identity(&model, &mut fails);
        let err = fails.into_result().expect_err("应拒绝").to_string();
        assert!(err.contains("测试模型"), "报错要指名模型：{err}");
        assert!(err.contains("UNVERIFIED"), "{err}");
    }

    /// 方案点名的失败场景之二：拒绝清单里的名字必须被拦下，报错要指名它并带上拒绝理由原文。
    #[test]
    fn denylisted_names_are_rejected_naming_the_model() {
        let denylist = parse_denylist(&repo_root().expect("repo root").join(DENYLIST))
            .expect("拒绝清单可解析");
        for name in [
            "matcha-icefall-zh-baker",
            "vits-zh-hf-keqing",
            "vits-zh-aishell3",
            "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17",
            "sherpa-onnx-paraformer-zh-2023-09-14",
            "sherpa-onnx-streaming-zipformer-small-ctc-zh-int8-2025-04-01",
        ] {
            let mut model = sample();
            model.name = name.to_owned();
            model.url = format!(
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/{name}.tar.bz2"
            );
            let mut fails = Failures::default();
            check_denylist(&model, &denylist, &mut fails);
            let err = fails.into_result().unwrap_err().to_string();
            assert!(err.contains(name), "报错要指名 {name}：{err}");
            assert!(err.contains("DENYLIST"), "{err}");
            assert!(err.contains("理由："), "报错要带上拒绝理由原文：{err}");
        }
    }

    /// 许可字段与证据内容不符必须被抓住——这是本闸门与「相信清单」的分界线。
    #[test]
    fn license_field_must_match_the_evidence_text() {
        let mut model = sample();
        model.license = "MIT".to_owned();
        let mut fails = Failures::default();
        check_evidence_declares_license(&model, b"Apache License\nVersion 2.0\n", &mut fails);
        let err = fails.into_result().expect_err("应拒绝").to_string();
        assert!(err.contains("找不到"), "{err}");

        let mut ok = Failures::default();
        check_evidence_declares_license(&model, b"MIT License\n\nCopyright (c) 2024\n", &mut ok);
        assert!(ok.into_result().is_ok());
    }

    /// 模型卡形态的证据只认 front-matter 里那一行，不能被正文里随便一句话糊弄过去。
    #[test]
    fn model_card_evidence_requires_the_front_matter_line() {
        let mut model = sample();
        model.license = "Apache-2.0".to_owned();
        model.license_evidence = Evidence::ModelCard;

        let mut good = Failures::default();
        check_evidence_declares_license(
            &model,
            b"---\nlicense: apache-2.0\n---\n# Streaming zipformer\n",
            &mut good,
        );
        assert!(good.into_result().is_ok());

        let mut bad = Failures::default();
        check_evidence_declares_license(
            &model,
            b"# Model\n\nThis is probably apache-2.0 licensed, I think.\n",
            &mut bad,
        );
        assert!(bad.into_result().is_err(), "散文里提一句不算声明");
    }

    /// 夹带 GPL-3.0 数据却不写分发影响，必须失败。
    #[test]
    fn bundled_copyleft_without_impact_statement_is_rejected() {
        let mut model = sample();
        model.bundled.push(Bundled {
            path: "espeak-ng-data".to_owned(),
            work: "espeak-ng 发音词典".to_owned(),
            license: "GPL-3.0".to_owned(),
            distribution_impact: None,
        });
        let mut fails = Failures::default();
        check_bundled(&model, &mut fails);
        let err = fails.into_result().expect_err("应拒绝").to_string();
        assert!(err.contains("GPL-3.0"), "{err}");
    }

    /// 某一类模型被拒到空，命令必须失败而不是「通过」。
    #[test]
    fn empty_production_asr_or_tts_fails_the_gate() {
        let manifest = Manifest {
            schema_version: 1,
            models: vec![sample()],
        };
        let mut fails = Failures::default();
        check_coverage(&manifest, &mut fails);
        let err = fails
            .into_result()
            .expect_err("只有 TTS 应失败")
            .to_string();
        assert!(err.contains("asr"), "{err}");
    }

    fn sample() -> Model {
        Model {
            name: "测试模型".to_owned(),
            kind: Kind::Tts,
            role: Role::Production,
            url: "https://example.invalid/测试模型.tar.bz2".to_owned(),
            sha256: "0".repeat(64),
            size_bytes: 1,
            license: "MIT".to_owned(),
            license_url: format!("https://example.invalid/{}/LICENSE", "a".repeat(40)),
            license_rev: "a".repeat(40),
            license_file: "models/licenses/测试.LICENSE".to_owned(),
            license_sha256: "0".repeat(64),
            license_evidence: Evidence::PackageLicense,
            underlying_work: "测试".to_owned(),
            verified_at: "2026-08-10".to_owned(),
            note: None,
            bundled: Vec::new(),
        }
    }
}
