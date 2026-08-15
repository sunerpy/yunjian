use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::verify_sources::emit;

const CONFIG_PATH: &str = "mobile/distribution.toml";
const SPIKE_REPORT: &str = "docs/reports/mobile-spike.json";
const MODELS_MANIFEST: &str = "models.toml";
const REPORT_JSON: &str = "docs/reports/mobile-size.json";
const REPORT_MARKDOWN: &str = "docs/reports/mobile-size.md";
const APK_CEILING_BYTES: u64 = 80 * 1024 * 1024;
const REQUIRED_ANDROID_ABIS: [&str; 4] = ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"];

#[derive(Debug, Deserialize)]
struct DistributionConfig {
    schema_version: u32,
    binding_verdict: String,
    shared_ui: String,
    universal_apk_default: bool,
    apk_ceiling_bytes: u64,
    corpus_packaged: bool,
    voice_models_packaged: bool,
    first_play_upload: String,
    android: AndroidConfig,
    ios: IosConfig,
}

#[derive(Debug, Deserialize)]
struct AndroidConfig {
    project: String,
    generated_project: String,
    min_sdk: u32,
    abis: Vec<String>,
    formats: Vec<String>,
    build_command: String,
    native_build_note: String,
}

#[derive(Debug, Deserialize)]
struct IosConfig {
    project: String,
    generated_project: String,
    build_command: String,
    archive_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Verdict {
    Pass,
    Fail,
    #[serde(rename = "NOT EXECUTED")]
    NotExecuted,
}

impl Verdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotExecuted => "NOT EXECUTED",
        }
    }
}

#[derive(Debug, Serialize)]
struct Outcome {
    id: &'static str,
    verdict: Verdict,
    detail: String,
    executable_when: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactKind {
    Apk,
    Aab,
    IosArchive,
    Ipa,
}

#[derive(Debug, Serialize)]
struct ArtifactRecord {
    kind: ArtifactKind,
    path: String,
    bytes: u64,
    abi: Option<String>,
    archive_entries_scanned: Option<usize>,
    forbidden_entries: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ToolProbe {
    available: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    generated_date: String,
    commit_sha: String,
    binding_verdict: String,
    all_pass: bool,
    executed_pass: usize,
    failed: usize,
    not_executed: usize,
    apk_ceiling_bytes: u64,
    universal_apk_default: bool,
    build_commands: BTreeMap<&'static str, String>,
    tool_probes: BTreeMap<&'static str, ToolProbe>,
    artifacts: Vec<ArtifactRecord>,
    assertions: Vec<Outcome>,
}

#[derive(Debug, Deserialize)]
struct SmokeObservation {
    physical_device: bool,
    device_model: String,
    os_build: String,
    two_character_search: bool,
    typed_recitation_round: bool,
    voice_session_start_stop: bool,
}

struct InspectedArtifact {
    kind: ArtifactKind,
    path: PathBuf,
    bytes: u64,
    abis: BTreeSet<String>,
    entries: Vec<String>,
    forbidden_entries: Vec<String>,
}

pub(crate) fn run(artifacts_dir: Option<PathBuf>, smoke_json: Option<PathBuf>) -> Result<()> {
    let root = repo_root();
    let config = read_config(&root)?;
    validate_config(&root, &config)?;
    let model_names = read_model_names(&root)?;
    let paths = artifacts_dir
        .as_deref()
        .map(discover_artifacts)
        .transpose()?
        .unwrap_or_default();
    if config.binding_verdict == "undetermined" && !paths.is_empty() {
        bail!("移动 binding verdict 仍是 undetermined，不接受来源不明的移动产物；先完成真机选型");
    }

    let inspected = paths
        .iter()
        .map(|path| inspect_artifact(path, &model_names))
        .collect::<Result<Vec<_>>>()?;
    let smoke = smoke_json.as_deref().map(read_smoke).transpose()?;
    let report = build_report(&root, &config, inspected, smoke);
    write_report(&root, &report)?;

    emit(&format!(
        "移动分发报告：{}；PASS {} FAIL {} NOT EXECUTED {} all_pass={}",
        root.join(REPORT_MARKDOWN).display(),
        report.executed_pass,
        report.failed,
        report.not_executed,
        report.all_pass
    ));
    if report.failed > 0 {
        bail!("移动分发守卫有 {} 条 FAIL", report.failed);
    }
    Ok(())
}

fn read_config(root: &Path) -> Result<DistributionConfig> {
    let path = root.join(CONFIG_PATH);
    let text =
        fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
    toml::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))
}

fn validate_config(root: &Path, config: &DistributionConfig) -> Result<()> {
    if config.schema_version != 1 {
        bail!("移动分发配置 schema_version 必须为 1");
    }
    if config.shared_ui != "app"
        || config.universal_apk_default
        || config.corpus_packaged
        || config.voice_models_packaged
        || config.first_play_upload != "manual"
    {
        bail!("移动分发配置违反共享 UI、按需资产、手工首传或 split APK 契约");
    }
    if config.apk_ceiling_bytes != APK_CEILING_BYTES {
        bail!(
            "APK 上限配置为 {}，冻结值是 {APK_CEILING_BYTES}；调整预算要同步修改方案与自锁测试",
            config.apk_ceiling_bytes
        );
    }
    if config.android.min_sdk != 26
        || config.android.abis != REQUIRED_ANDROID_ABIS
        || config.android.formats != ["apk", "aab"]
        || !config.android.build_command.contains("--split-per-abi")
        || !config.android.build_command.contains("--apk --aab")
        || config.android.native_build_note.trim().is_empty()
    {
        bail!("Android 分发配置缺少 minSdk 26、四 ABI、APK/AAB、split 或 NDK 说明");
    }
    if config.android.project != "mobile/android"
        || config.android.generated_project != "crates/yunjian-app/gen/android"
        || config.ios.project != "mobile/ios"
        || config.ios.generated_project != "crates/yunjian-app/gen/apple"
        || config.ios.archive_kind != "xcarchive_or_ipa"
        || !config
            .ios
            .build_command
            .contains("--export-method app-store-connect")
    {
        bail!("移动工程落点或 iOS archive 命令发生漂移");
    }
    let spike: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(SPIKE_REPORT)).context("读取 mobile spike 报告失败")?,
    )
    .context("解析 mobile spike 报告失败")?;
    let recorded = spike
        .get("verdict")
        .and_then(serde_json::Value::as_str)
        .context("mobile spike 报告缺少 verdict")?;
    if config.binding_verdict != recorded {
        bail!(
            "分发配置 binding_verdict={}，实测报告 verdict={recorded}",
            config.binding_verdict
        );
    }
    Ok(())
}

fn read_model_names(root: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(root.join(MODELS_MANIFEST)).context("读取 models.toml 失败")?;
    let value: toml::Value = toml::from_str(&text).context("解析 models.toml 失败")?;
    Ok(value
        .get("model")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("name").and_then(toml::Value::as_str))
        .map(|name| name.to_ascii_lowercase())
        .collect())
}

fn discover_artifacts(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        bail!("产物目录不存在：{}", dir.display());
    }
    let mut out = Vec::new();
    collect_artifacts(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_artifacts(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xcarchive"))
    {
        out.push(path.to_path_buf());
        return Ok(());
    }
    if path.is_file() {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if ["apk", "aab", "ipa"]
            .iter()
            .any(|expected| extension.eq_ignore_ascii_case(expected))
        {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("遍历 {} 失败", path.display()))? {
        collect_artifacts(&entry?.path(), out)?;
    }
    Ok(())
}

fn inspect_artifact(path: &Path, model_names: &[String]) -> Result<InspectedArtifact> {
    let kind = artifact_kind(path)?;
    if kind == ArtifactKind::IosArchive {
        return Ok(InspectedArtifact {
            kind,
            path: path.to_path_buf(),
            bytes: directory_bytes(path)?,
            abis: BTreeSet::new(),
            entries: Vec::new(),
            forbidden_entries: Vec::new(),
        });
    }
    let bytes = fs::metadata(path)
        .with_context(|| format!("读取 {} 元信息失败", path.display()))?
        .len();
    let mut zip =
        ZipArchive::new(File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?)
            .with_context(|| format!("{} 不是可读 ZIP 容器", path.display()))?;
    let mut entries = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        entries.push(
            zip.by_index(index)
                .with_context(|| format!("读取 {} 第 {index} 个 ZIP 条目失败", path.display()))?
                .name()
                .replace('\\', "/"),
        );
    }
    let abis = archive_abis(&entries);
    let forbidden_entries = entries
        .iter()
        .filter(|entry| forbidden_asset(entry, model_names))
        .cloned()
        .collect();
    Ok(InspectedArtifact {
        kind,
        path: path.to_path_buf(),
        bytes,
        abis,
        entries,
        forbidden_entries,
    })
}

fn artifact_kind(path: &Path) -> Result<ArtifactKind> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "apk" => Ok(ArtifactKind::Apk),
        "aab" => Ok(ArtifactKind::Aab),
        "ipa" => Ok(ArtifactKind::Ipa),
        "xcarchive" => Ok(ArtifactKind::IosArchive),
        _ => bail!("不认识的移动产物：{}", path.display()),
    }
}

fn archive_abis(entries: &[String]) -> BTreeSet<String> {
    REQUIRED_ANDROID_ABIS
        .iter()
        .filter(|abi| {
            let marker = format!("/lib/{abi}/");
            entries.iter().any(|entry| {
                let normalized = format!("/{entry}");
                normalized.contains(&marker)
            })
        })
        .map(|abi| (*abi).to_owned())
        .collect()
}

fn forbidden_asset(entry: &str, model_names: &[String]) -> bool {
    let lower = entry.to_ascii_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    basename.ends_with(".db")
        || basename.ends_with(".db.gz")
        || basename.ends_with(".onnx")
        || basename.ends_with(".ort")
        || basename == "tokens.txt"
        || lower.contains("espeak-ng-data/")
        || model_names.iter().any(|name| lower.contains(name))
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).with_context(|| format!("遍历 {} 失败", path.display()))? {
        let path = entry?.path();
        total += if path.is_dir() {
            directory_bytes(&path)?
        } else {
            fs::metadata(&path)?.len()
        };
    }
    Ok(total)
}

fn read_smoke(path: &Path) -> Result<SmokeObservation> {
    let text = fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))
}

fn build_report(
    root: &Path,
    config: &DistributionConfig,
    artifacts: Vec<InspectedArtifact>,
    smoke: Option<SmokeObservation>,
) -> Report {
    let apks: Vec<&InspectedArtifact> = artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Apk)
        .collect();
    let aabs: Vec<&InspectedArtifact> = artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Aab)
        .collect();
    let ios: Vec<&InspectedArtifact> = artifacts
        .iter()
        .filter(|artifact| matches!(artifact.kind, ArtifactKind::IosArchive | ArtifactKind::Ipa))
        .collect();

    let assertions = vec![
        android_apk_outcome(&apks),
        single_artifact_outcome(
            "android_aab",
            &aabs,
            "在有完整 Android SDK/NDK 与已选 binding 的 runner 上执行 distribution.toml 的 Android 命令",
        ),
        single_artifact_outcome(
            "ios_archive",
            &ios,
            "在安装 Xcode 26 / iOS 26 SDK 且配置 Distribution 证书与 provisioning profile 的 macOS 上执行 distribution.toml 的 iOS 命令",
        ),
        complete_sizes_outcome(&apks, &aabs, &ios),
        apk_ceiling_outcome(&apks),
        asset_guard_outcome(&artifacts),
        smoke_outcome(smoke.as_ref()),
    ];

    let failed = assertions
        .iter()
        .filter(|outcome| outcome.verdict == Verdict::Fail)
        .count();
    let not_executed = assertions
        .iter()
        .filter(|outcome| outcome.verdict == Verdict::NotExecuted)
        .count();
    let executed_pass = assertions
        .iter()
        .filter(|outcome| outcome.verdict == Verdict::Pass)
        .count();
    let records = artifacts
        .into_iter()
        .map(|artifact| ArtifactRecord {
            kind: artifact.kind,
            path: artifact.path.to_string_lossy().replace('\\', "/"),
            bytes: artifact.bytes,
            abi: (artifact.kind == ArtifactKind::Apk && artifact.abis.len() == 1)
                .then(|| artifact.abis.iter().next().cloned())
                .flatten(),
            archive_entries_scanned: (!artifact.entries.is_empty())
                .then_some(artifact.entries.len()),
            forbidden_entries: artifact.forbidden_entries,
        })
        .collect();
    Report {
        schema_version: 1,
        generated_date: today(),
        commit_sha: commit_sha(root),
        binding_verdict: config.binding_verdict.clone(),
        all_pass: failed == 0 && not_executed == 0,
        executed_pass,
        failed,
        not_executed,
        apk_ceiling_bytes: APK_CEILING_BYTES,
        universal_apk_default: config.universal_apk_default,
        build_commands: BTreeMap::from([
            ("android", config.android.build_command.clone()),
            ("ios", config.ios.build_command.clone()),
        ]),
        tool_probes: BTreeMap::from([
            ("tauri_cli", probe("cargo", &["tauri", "--version"])),
            ("adb", probe("adb", &["version"])),
            ("sdkmanager", probe("sdkmanager", &["--version"])),
            ("gradle", probe("gradle", &["--version"])),
            ("xcrun", probe("xcrun", &["--version"])),
        ]),
        artifacts: records,
        assertions,
    }
}

fn android_apk_outcome(apks: &[&InspectedArtifact]) -> Outcome {
    if apks.is_empty() {
        return not_executed(
            "android_per_abi_apks",
            "没有真实 APK，未执行四 ABI 完整性判断",
            "提供完整 Android SDK/NDK 与已选 binding，执行带 --split-per-abi --apk 的构建命令",
        );
    }
    let actual: BTreeSet<String> = apks
        .iter()
        .filter(|apk| apk.abis.len() == 1)
        .filter_map(|apk| apk.abis.iter().next().cloned())
        .collect();
    let expected: BTreeSet<String> = REQUIRED_ANDROID_ABIS
        .iter()
        .map(|abi| (*abi).to_owned())
        .collect();
    let universal = apks.iter().any(|apk| {
        apk.abis.len() != 1
            || apk.path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("universal")
            })
    });
    if apks.len() == REQUIRED_ANDROID_ABIS.len() && actual == expected && !universal {
        pass(
            "android_per_abi_apks",
            format!(
                "四个 split APK 齐备：{}",
                actual.into_iter().collect::<Vec<_>>().join(", ")
            ),
        )
    } else {
        fail(
            "android_per_abi_apks",
            format!(
                "要求四个单 ABI APK 且禁 universal；实际 {} 个，识别 ABI={actual:?}，universal={universal}",
                apks.len()
            ),
        )
    }
}

fn single_artifact_outcome(
    id: &'static str,
    artifacts: &[&InspectedArtifact],
    executable_when: &str,
) -> Outcome {
    match artifacts.len() {
        0 => not_executed(id, "没有真实构建产物", executable_when),
        1 => pass(
            id,
            format!(
                "产物存在：{}（{} bytes）",
                artifacts[0].path.display(),
                artifacts[0].bytes
            ),
        ),
        count => fail(id, format!("期望一个产物，实际发现 {count} 个")),
    }
}

fn complete_sizes_outcome(
    apks: &[&InspectedArtifact],
    aabs: &[&InspectedArtifact],
    ios: &[&InspectedArtifact],
) -> Outcome {
    if apks.len() == 4 && aabs.len() == 1 && ios.len() == 1 {
        pass(
            "artifact_sizes_measured",
            "四 APK、AAB 与 iOS archive 均记录了真实字节数",
        )
    } else {
        not_executed(
            "artifact_sizes_measured",
            format!(
                "产物集不完整：APK {} / 4，AAB {} / 1，iOS archive {} / 1；未用估算补空值",
                apks.len(),
                aabs.len(),
                ios.len()
            ),
            "在 Android 与 macOS 签名 runner 上完成构建，把全部产物下载到同一目录后重跑",
        )
    }
}

fn apk_ceiling_outcome(apks: &[&InspectedArtifact]) -> Outcome {
    if apks.is_empty() {
        return not_executed(
            "apk_ceiling",
            "没有 APK，无法测量上限",
            "构建四个 split release APK 后重跑；上限固定为 80 MiB/个",
        );
    }
    let oversized: Vec<String> = apks
        .iter()
        .filter(|apk| apk.bytes > APK_CEILING_BYTES)
        .map(|apk| format!("{}={} bytes", apk.path.display(), apk.bytes))
        .collect();
    if oversized.is_empty() {
        pass(
            "apk_ceiling",
            format!("{} 个 APK 均不超过 {APK_CEILING_BYTES} bytes", apks.len()),
        )
    } else {
        fail("apk_ceiling", format!("超限：{}", oversized.join("；")))
    }
}

fn asset_guard_outcome(artifacts: &[InspectedArtifact]) -> Outcome {
    let scanned: Vec<&InspectedArtifact> = artifacts
        .iter()
        .filter(|artifact| !artifact.entries.is_empty())
        .collect();
    if scanned.is_empty() {
        return not_executed(
            "packaged_assets",
            "没有 APK/AAB/IPA，未执行 ZIP 内容扫描",
            "提供真实 APK/AAB/IPA；守卫会读取 ZIP 中央目录并拒绝 corpus .db 与语音模型",
        );
    }
    let forbidden: Vec<String> = scanned
        .iter()
        .flat_map(|artifact| {
            artifact
                .forbidden_entries
                .iter()
                .map(|entry| format!("{}:{entry}", artifact.path.display()))
        })
        .collect();
    if forbidden.is_empty() {
        pass(
            "packaged_assets",
            format!(
                "扫描 {} 个 ZIP 产物，未发现 corpus .db 或语音模型",
                scanned.len()
            ),
        )
    } else {
        fail(
            "packaged_assets",
            format!("发现禁带资产：{}", forbidden.join("；")),
        )
    }
}

fn smoke_outcome(smoke: Option<&SmokeObservation>) -> Outcome {
    let Some(smoke) = smoke else {
        return not_executed(
            "instrumented_device_smoke",
            "没有物理设备 instrumented smoke 观测；未用 jsdom、模拟器或宿主结果顶替",
            "连接已授权物理设备，运行 instrumented test 完成两字检索、打字背诵一轮、语音会话启动与停止，并通过 --smoke-json 提交观测",
        );
    };
    let passed = smoke.physical_device
        && smoke.two_character_search
        && smoke.typed_recitation_round
        && smoke.voice_session_start_stop
        && !smoke.device_model.trim().is_empty()
        && !smoke.os_build.trim().is_empty();
    if passed {
        pass(
            "instrumented_device_smoke",
            format!(
                "{} / {}：检索、打字背诵、语音 start-stop 均通过",
                smoke.device_model, smoke.os_build
            ),
        )
    } else {
        fail(
            "instrumented_device_smoke",
            format!(
                "观测不满足物理设备三步 smoke：physical={} search={} typed={} voice={}",
                smoke.physical_device,
                smoke.two_character_search,
                smoke.typed_recitation_round,
                smoke.voice_session_start_stop
            ),
        )
    }
}

fn pass(id: &'static str, detail: impl Into<String>) -> Outcome {
    Outcome {
        id,
        verdict: Verdict::Pass,
        detail: detail.into(),
        executable_when: None,
    }
}

fn fail(id: &'static str, detail: impl Into<String>) -> Outcome {
    Outcome {
        id,
        verdict: Verdict::Fail,
        detail: detail.into(),
        executable_when: None,
    }
}

fn not_executed(
    id: &'static str,
    detail: impl Into<String>,
    executable_when: impl Into<String>,
) -> Outcome {
    Outcome {
        id,
        verdict: Verdict::NotExecuted,
        detail: detail.into(),
        executable_when: Some(executable_when.into()),
    }
}

fn probe(program: &str, args: &[&str]) -> ToolProbe {
    match Command::new(program).args(args).output() {
        Ok(output) => ToolProbe {
            available: output.status.success(),
            detail: format!(
                "exit={:?}; stdout={}; stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        },
        Err(error) => ToolProbe {
            available: false,
            detail: error.to_string(),
        },
    }
}

fn write_report(root: &Path, report: &Report) -> Result<()> {
    let json_path = root.join(REPORT_JSON);
    let markdown_path = root.join(REPORT_MARKDOWN);
    let mut json = serde_json::to_string_pretty(report).context("序列化移动体积报告失败")?;
    json.push('\n');
    fs::write(&json_path, json).with_context(|| format!("写 {} 失败", json_path.display()))?;
    fs::write(&markdown_path, render_markdown(report))
        .with_context(|| format!("写 {} 失败", markdown_path.display()))?;
    Ok(())
}

fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 移动端分发与体积报告");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> [!WARNING]\n> **`all_pass = {}`；binding verdict 为 `{}`。**\n> Android/iOS 构建、真实体积与设备 smoke 没有产物或物理证据时一律是 `NOT EXECUTED`，不是 PASS。",
        report.all_pass, report.binding_verdict
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "- 日期：`{}`", report.generated_date);
    let _ = writeln!(out, "- 提交：`{}`", report.commit_sha);
    let _ = writeln!(
        out,
        "- APK 上限：`{}` bytes（80 MiB / split APK）",
        report.apk_ceiling_bytes
    );
    let _ = writeln!(
        out,
        "- universal APK 默认分发：`{}`",
        report.universal_apk_default
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## 构建命令");
    let _ = writeln!(out);
    for (platform, command) in &report.build_commands {
        let _ = writeln!(out, "- `{platform}`: `{command}`");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## 工具探测");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 工具 | 可用 | 依据 |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for (tool, probe) in &report.tool_probes {
        let _ = writeln!(
            out,
            "| `{tool}` | `{}` | {} |",
            probe.available,
            probe.detail.replace('|', "\\|")
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## 逐条裁决");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 断言 | 裁决 | 依据 / 可执行条件 |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for assertion in &report.assertions {
        let when = assertion
            .executable_when
            .as_deref()
            .map_or_else(String::new, |value| format!("<br>**可执行条件**：{value}"));
        let _ = writeln!(
            out,
            "| `{}` | **{}** | {}{} |",
            assertion.id,
            assertion.verdict.as_str(),
            assertion.detail,
            when
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## 实测产物");
    let _ = writeln!(out);
    if report.artifacts.is_empty() {
        let _ = writeln!(out, "没有真实产物；不记录估算体积。");
    } else {
        let _ = writeln!(out, "| 类型 | ABI | 字节 | ZIP 条目 | 路径 |");
        let _ = writeln!(out, "| --- | --- | ---: | ---: | --- |");
        for artifact in &report.artifacts {
            let _ = writeln!(
                out,
                "| `{:?}` | `{}` | {} | {} | `{}` |",
                artifact.kind,
                artifact.abi.as_deref().unwrap_or("—"),
                artifact.bytes,
                artifact
                    .archive_entries_scanned
                    .map_or_else(|| "—".to_owned(), |value| value.to_string()),
                artifact.path
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## 商店提交");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "1. Play 首次上传必须手工在 Play Console 建立应用与 internal testing track，上传 AAB、完成内容与数据安全声明；后续自动化才能复用该应用身份。"
    );
    let _ = writeln!(
        out,
        "2. APK 只作为逐 ABI 测试/旁载产物，默认发布不生成 universal APK；Play 分发使用 AAB。"
    );
    let _ = writeln!(
        out,
        "3. iOS 在有 Xcode 26 / iOS 26 SDK 与签名身份的 macOS 上 archive，再通过 App Store Connect / TestFlight 上传。"
    );
    let _ = writeln!(
        out,
        "4. corpus 与语音模型均按需下载并校验，不进入商店初始包；ZIP 资产守卫把这条架构约束变成失败条件。"
    );
    out
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须位于仓库根的直接子目录")
        .to_path_buf()
}

fn commit_sha(root: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn today() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_distribution_constants_cannot_be_relaxed_silently() {
        assert_eq!(
            APK_CEILING_BYTES,
            80 * 1024 * 1024,
            "调整 APK 上限先修改方案和报告口径，不要只改这条断言"
        );
        assert_eq!(
            REQUIRED_ANDROID_ABIS,
            ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"],
            "默认分发必须保持四个 split ABI；不要改成 universal APK"
        );
    }

    #[test]
    fn forbidden_asset_guard_catches_corpus_and_voice_models() {
        let models = vec!["sherpa-onnx-whisper-tiny".to_owned()];
        for entry in [
            "assets/corpus.db",
            "assets/corpus.db.gz",
            "assets/model.onnx",
            "assets/sherpa-onnx-whisper-tiny/tokens.txt",
            "assets/espeak-ng-data/zh_dict",
        ] {
            assert!(forbidden_asset(entry, &models), "应拒绝 {entry}");
        }
        assert!(!forbidden_asset("lib/arm64-v8a/libyunjian.so", &models));
    }

    #[test]
    fn abi_detector_distinguishes_split_from_universal_archives() {
        let split = archive_abis(&["lib/arm64-v8a/libyunjian.so".to_owned()]);
        assert_eq!(split, BTreeSet::from(["arm64-v8a".to_owned()]));
        let universal = archive_abis(&[
            "lib/arm64-v8a/libyunjian.so".to_owned(),
            "lib/x86_64/libyunjian.so".to_owned(),
        ]);
        assert_eq!(universal.len(), 2);
    }

    #[test]
    fn no_artifact_report_is_explicitly_not_complete() {
        let config = read_config(&repo_root()).expect("读取分发配置");
        let report = build_report(&repo_root(), &config, Vec::new(), None);
        assert!(!report.all_pass);
        assert_eq!(report.failed, 0);
        assert_eq!(report.not_executed, report.assertions.len());
    }
}
