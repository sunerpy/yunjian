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

/// 四个 ABI 与它们的 Rust target triple。
///
/// 用来把「缺哪个 ABI」翻成「缺它的原因」：`crates/yunjian-voice/build.rs` 的
/// `PREBUILT_TARGETS` 列出上游 sherpa-onnx 提供预编译产物的 triple，不在其中的 ABI
/// 在**默认开 voice**的前提下无法构建。下面那条测试逐条比对两份清单，防止悄悄分叉。
const ABI_RUST_TARGETS: [(&str, &str); 4] = [
    ("arm64-v8a", "aarch64-linux-android"),
    ("armeabi-v7a", "armv7-linux-androideabi"),
    ("x86", "i686-linux-android"),
    ("x86_64", "x86_64-linux-android"),
];

/// 上游 sherpa-onnx 预编译目标清单的来源。判据不内置第二份副本。
const VOICE_BUILD_SCRIPT: &str = "crates/yunjian-voice/build.rs";

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
    tauri_generated_project: String,
    min_sdk: u32,
    abis: Vec<String>,
    formats: Vec<String>,
    build_command: String,
    native_build_note: String,
}

#[derive(Debug, Deserialize)]
struct IosConfig {
    project: String,
    tauri_generated_project: String,
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

pub(crate) fn run(
    artifacts_dir: Option<PathBuf>,
    smoke_json: Option<PathBuf>,
    smoke_log: Option<PathBuf>,
) -> Result<()> {
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
    let smoke = match (smoke_json.as_deref(), smoke_log.as_deref()) {
        (Some(_), Some(_)) => {
            bail!("--smoke-json 与 --smoke-log 只能给一个，否则说不清观测来自哪份证据")
        }
        (Some(path), None) => Some(read_smoke(path)?),
        (None, Some(path)) => Some(read_smoke_from_device_log(path)?),
        (None, None) => None,
    };
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
        || config.android.native_build_note.trim().is_empty()
    {
        bail!("Android 分发配置缺少 minSdk 26、四 ABI、APK/AAB 或原生构建说明");
    }
    if config.android.project != "mobile/android"
        || config.android.tauri_generated_project != "crates/yunjian-app/gen/android"
        || config.ios.project != "mobile/ios"
        || config.ios.tauri_generated_project != "crates/yunjian-app/gen/apple"
        || config.ios.archive_kind != "xcarchive_or_ipa"
    {
        bail!("移动工程落点发生漂移");
    }
    validate_build_commands(root, config)?;
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

/// 构建命令必须属于**被选中的那条分支**，而且那条分支的选中状态在文件系统上可判定。
///
/// # 为什么这条必须存在
///
/// 曾经这里只校验 `--split-per-abi` 与 `--export-method app-store-connect` 的存在，那是
/// **tauri_mobile 分支**的命令形状。裁决在 2026-08-16 变成 `uniffi_native` 之后，配置仍写着
/// 那两条命令，于是 `docs/reports/mobile-size.json` 描述的是一条根本没有被走的构建路径——
/// 而报告本身看起来完好（有 schema、有断言、有 `all_pass` 字段）。
///
/// 「报告存在 ≠ 报告描述的是当前系统」，所以判据不能只看命令里有没有某个开关，还要看
/// **另一条分支的产物在不在**：`uniffi_native` 下 Tauri 的生成工程必须不存在。
fn validate_build_commands(root: &Path, config: &DistributionConfig) -> Result<()> {
    match config.binding_verdict.as_str() {
        "uniffi_native" => {
            for (label, generated) in [
                ("Android", config.android.tauri_generated_project.as_str()),
                ("iOS", config.ios.tauri_generated_project.as_str()),
            ] {
                if root.join(generated).exists() {
                    bail!(
                        "裁决是 uniffi_native，但 {label} 的 Tauri 生成工程 `{generated}` 存在；\
                         两条分支的产物同时在树上时，报告说不清测的是哪一个"
                    );
                }
            }
            for (label, command, needles) in [
                (
                    "Android",
                    config.android.build_command.as_str(),
                    ["mobile/android", "assembleRelease", "bundleRelease"].as_slice(),
                ),
                (
                    "iOS",
                    config.ios.build_command.as_str(),
                    ["mobile/ios", "xcodegen", "xcodebuild"].as_slice(),
                ),
            ] {
                for needle in needles {
                    if !command.contains(needle) {
                        bail!(
                            "裁决是 uniffi_native，{label} 构建命令里没有 `{needle}`：\
                             它描述的不是原生工程那条路径"
                        );
                    }
                }
                if command.contains("cargo tauri") {
                    bail!(
                        "裁决是 uniffi_native，{label} 构建命令仍是 `cargo tauri …`：\
                         那是 tauri_mobile 分支的命令"
                    );
                }
            }
            validate_loop_stops_on_failure(config.android.build_command.as_str())?;
            validate_per_abi_artifacts_are_collected(config.android.build_command.as_str())?;
        }
        "tauri_mobile" => {
            if !config.android.build_command.contains("--split-per-abi")
                || !config.android.build_command.contains("--apk --aab")
                || !config
                    .ios
                    .build_command
                    .contains("--export-method app-store-connect")
            {
                bail!(
                    "裁决是 tauri_mobile，构建命令必须是带 split 与 app-store-connect 的 tauri 命令"
                );
            }
        }
        other => bail!("未知 binding verdict `{other}`：只接受 uniffi_native 或 tauri_mobile"),
    }
    Ok(())
}

/// 逐 ABI 的构建循环不得吞掉单个 ABI 的失败。
///
/// # 为什么这条是门禁而不是注释
///
/// 命令曾经是裸 `for abi in …; do gradle …; done`。x86 那一轮失败时循环照样走完，
/// 整条命令**退出码 0**，产物集只有三个 APK——`android_per_abi_apks` 判 FAIL 的直接
/// 成因就是这个被吞掉的失败。**退出码 0 不等于成功**，而这种缺口只会在下游的产物
/// 计数上露头，读起来像构建时少做了一个。
///
/// 只认 `|| exit`，**`set -e` 在这里是个假修复**：POSIX 规定 `-e` 对 AND-OR 列表里
/// 除最后一个之外的命令一律忽略，而这条命令的形状正是
/// `cd … && <循环> && gradle bundleRelease`——循环不是最后一个。实测过：加了
/// `set -e`，x86 那一轮失败后整条命令仍然退出 0，`bundleRelease` 照样跑。
fn validate_loop_stops_on_failure(command: &str) -> Result<()> {
    let has_loop = command.contains("for ") && command.contains("; do ");
    if has_loop && !command.contains("|| exit") {
        bail!(
            "Android 构建命令里的逐 ABI 循环没有 `|| exit`：\
             单个 ABI 失败时循环会走完、整条命令退出码 0，\
             于是「构建成功」却只产出部分 APK。命令：{command}"
        );
    }
    Ok(())
}

/// 逐 ABI 的循环必须把每轮的 APK 搬到带 ABI 名的目标上。
///
/// `assembleRelease` 固定写 `app/build/outputs/apk/release/app-release.apk`，四轮互相覆盖。
/// 命令曾经不含搬运，照它跑完只剩最后一个 ABI 的 APK，而本模块扫的是
/// `target/mobile/dist/yunjian-<abi>-release.apk`——搬运在命令之外手工做，没有记录。
/// 「命令跑完得不到报告扫描的那个产物集」与被吞掉的失败是同一类缺陷：
/// 报告看起来完好，而任何人照命令重跑都复现不出来。
fn validate_per_abi_artifacts_are_collected(command: &str) -> Result<()> {
    let has_loop = command.contains("for ") && command.contains("; do ");
    if has_loop && !command.contains("yunjian-$abi") {
        bail!(
            "Android 构建命令的逐 ABI 循环没有把 APK 搬到带 ABI 名的目标（`yunjian-$abi`…）：\
             assembleRelease 每轮写同一个 app-release.apk，四轮互相覆盖，\
             照命令跑完只剩最后一个 ABI。命令：{command}"
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
    let zip =
        ZipArchive::new(File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?)
            .with_context(|| format!("{} 不是可读 ZIP 容器", path.display()))?;
    // 只读**中央目录里的名字**，不构造解压读取器。
    //
    // `by_index` 会为该条目建一个解压器，于是 deflate 条目在 `zip` 的
    // `default-features = false` 配置下报 `Compression method not supported`——而真实的
    // release APK 里恰好一部分条目是 deflate（实测 arm64 那份：70 条 stored、45 条 deflate）。
    // 那条报错读起来像「产物坏了」，实际是扫描器要了它不需要的能力：这个守卫问的是
    // 「包里有没有 corpus .db 或语音模型」，只需要**条目名**。
    let entries: Vec<String> = zip
        .file_names()
        .map(|name| name.replace('\\', "/"))
        .collect();
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

/// 从真机 instrumented 测量日志（`YUNJIAN-FULL …` 那套）导出 smoke 观测。
///
/// # 为什么要有这条路径
///
/// 之前 smoke 观测靠一份**手写、未入库**的 JSON 经 `--smoke-json` 传进来。于是
/// `mobile-size.json` 里那条 PASS 没法重放：想重跑报告的人手上没有那份 JSON，
/// 只能自己编一份——而编出来的东西与真机上发生过的事没有关系。这与「构建命令跑完
/// 得不到报告扫描的产物集」是同一类缺陷：报告看起来完好，却复现不出来。
///
/// 日志本身是入库的真机产物（`docs/reports/mobile-qa-android-measurements.log`），
/// 所以从它导出等于把唯一的事实来源接进来，而不是新增一份可以与它分叉的副本。
///
/// # 为什么不放宽任何一项
///
/// 物理设备判定沿用 `full_criteria::DeviceIdentity::is_physical`（按 `ro.hardware`、
/// `ro.kernel.qemu`、fingerprint 的模拟器标记判），不在这里重写一套——重写就是第二份
/// 会分叉的判据，而分叉的形态是模拟器被当成真机。三项行为断言各自要求日志里那条
/// 明确为 `true` 的键；键缺失时返回 `false` 而不是当成通过，缺证据不等于通过。
fn read_smoke_from_device_log(path: &Path) -> Result<SmokeObservation> {
    use crate::acceptance::mobile::full_criteria::MeasurementSet;

    let text = fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))?;
    let measurements = MeasurementSet::parse(&text);
    if measurements.is_empty() {
        bail!(
            "{} 里没有一行 `YUNJIAN-FULL …` 测量：它不是真机 instrumented 日志",
            path.display()
        );
    }
    let identity = measurements.device_identity();
    let flag = |assertion: &str, key: &str| measurements.get(assertion, key) == Some("true");
    Ok(SmokeObservation {
        physical_device: identity.is_physical(),
        device_model: measurements
            .get("install_and_launch", "device_model")
            .unwrap_or(identity.model.as_str())
            .to_owned(),
        os_build: measurements
            .get("install_and_launch", "os_build")
            .unwrap_or(identity.os_build.as_str())
            .to_owned(),
        two_character_search: measurements
            .get("two_char_search_returns_results", "hits")
            .and_then(|hits| hits.parse::<u32>().ok())
            .is_some_and(|hits| hits > 0),
        typed_recitation_round: flag(
            "typed_recitation_scores_correctly",
            "answer_equals_reference",
        ),
        voice_session_start_stop: flag("voice_recitation_round_succeeds_end_to_end", "spoke")
            && flag(
                "voice_recitation_round_succeeds_end_to_end",
                "native_voice_enabled",
            ),
    })
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
        // **判词要带上缺失的原因**，否则读者只看到「实际 3 个」，会以为是构建时偷懒。
        // 判词的信息量决定了下一步能不能动手（本仓库在真机验收上反复吃过这条）。
        let missing: Vec<&str> = expected
            .iter()
            .filter(|abi| !actual.contains(*abi))
            .map(String::as_str)
            .collect();
        let mut detail = format!(
            "要求四个单 ABI APK 且禁 universal；实际 {} 个（{}），universal={universal}",
            apks.len(),
            actual.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        if !missing.is_empty() {
            let _ = write!(detail, "；缺 {}", missing.join(", "));
            for abi in &missing {
                if let Some(triple) = abi_rust_target(abi) {
                    let _ = write!(
                        detail,
                        "；{abi} 的 {triple} 应在 {VOICE_BUILD_SCRIPT} 的 PREBUILT_TARGETS 里——\
                         上游 sherpa-onnx 的 Android 归档一个包含四个 ABI，\
                         `jniLibs/{abi}/` 就在里面；若该 triple 还不在 sherpa-rs-sys 的 \
                         dist.json 映射表内，则由 {VOICE_BUILD_SCRIPT} 的 \
                         UNMAPPED_ANDROID_TARGETS 那条路径补链接标志，\
                         漏掉它时链接期只报 undefined symbol 而不指向真因"
                    );
                }
            }
        }
        fail("android_per_abi_apks", detail)
    }
}

fn abi_rust_target(abi: &str) -> Option<&'static str> {
    ABI_RUST_TARGETS
        .iter()
        .find(|(name, _)| *name == abi)
        .map(|(_, triple)| *triple)
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
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    static NEXT_SANDBOX: AtomicU32 = AtomicU32::new(0);

    /// 与仓库其他测试同一个手法（见 `crates/yunjian-cli/tests/cli.rs`）：进程号 + 序号
    /// 命名，Drop 时清掉。不为写几行假日志引一个新依赖。
    struct Sandbox {
        dir: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yunjian-mobile-distribution-{}-{}",
                std::process::id(),
                NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("创建沙箱目录");
            Self { dir }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

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

    /// ABI → triple 的映射必须与语音构建脚本认识的目标对得上。
    ///
    /// 判据不内置第二份预编译清单：**有备份，两份清单就能悄悄分叉**，而分叉的形态是判词
    /// 说「x86 没有上游预编译」而实际上游已经补上了（或反过来）。这条测试解析
    /// `crates/yunjian-voice/build.rs` 里的那份清单，并要求每个 triple 的收录状态可判定。
    #[test]
    fn the_abi_to_triple_map_agrees_with_the_voice_build_script() {
        let script = fs::read_to_string(repo_root().join(VOICE_BUILD_SCRIPT))
            .expect("必须能读到语音构建脚本");
        // 先切到 `= &[` 之后再找 `]`：直接找第一个 `]` 会切在类型标注 `&[&str]` 上，
        // 于是解析出一个空清单——那会让这条测试**通过**（空清单里当然没有 i686），
        // 也就是一条永远绿的假门禁。
        let prebuilt = script
            .split_once("const PREBUILT_TARGETS")
            .and_then(|(_, tail)| tail.split_once("= &["))
            .and_then(|(_, tail)| tail.split_once(']'))
            .map(|(list, _)| list.to_owned())
            .expect("语音构建脚本里必须有 PREBUILT_TARGETS 清单");
        assert!(
            prebuilt.contains("aarch64-linux-android"),
            "解析器与脚本格式已经不匹配：清单里连 arm64 都没读到"
        );
        let mapped: BTreeSet<&str> = ABI_RUST_TARGETS.iter().map(|(abi, _)| *abi).collect();
        let required: BTreeSet<&str> = REQUIRED_ANDROID_ABIS.iter().copied().collect();
        assert_eq!(
            mapped, required,
            "映射必须逐个覆盖声明的四个 ABI，少一个就有 ABI 的成因说不出来"
        );

        // 逐个断言收录状态，而不是数个数：四个 ABI 都必须在清单里，因为上游
        // sherpa-onnx 的 Android 归档**一个包含四个 ABI**（`jniLibs/x86/*.so` 实测是
        // `ELF 32-bit LSB shared object, Intel 80386`）。曾经这里断言 i686 **不在**
        // 清单里，并把 `android_per_abi_apks` 的 FAIL 归因成「上游没有 x86 预编译，
        // 需用户裁量」——那个前提是错的，产物一直在同一个包里。
        for (_, triple) in ABI_RUST_TARGETS {
            assert!(
                prebuilt.contains(triple),
                "`{triple}` 应在上游预编译清单里；它不在说明清单或解析器变了"
            );
        }
    }

    #[test]
    fn a_missing_abi_is_reported_with_its_cause() {
        let apk = InspectedArtifact {
            kind: ArtifactKind::Apk,
            path: PathBuf::from("target/mobile/dist/yunjian-arm64-v8a-release.apk"),
            bytes: 1,
            abis: BTreeSet::from(["arm64-v8a".to_owned()]),
            entries: vec!["lib/arm64-v8a/libyunjian_mobile.so".to_owned()],
            forbidden_entries: Vec::new(),
        };
        let outcome = android_apk_outcome(&[&apk]);
        assert_eq!(
            outcome.verdict,
            Verdict::Fail,
            "缺 ABI 是真失败，不是未执行"
        );
        assert!(
            outcome.detail.contains("缺 armeabi-v7a, x86, x86_64"),
            "判词必须列出缺哪几个：{}",
            outcome.detail
        );
        assert!(
            outcome.detail.contains("i686-linux-android")
                && outcome.detail.contains("UNMAPPED_ANDROID_TARGETS"),
            "判词必须带上成因与可执行的下一步，否则读者只看到「实际 1 个」：{}",
            outcome.detail
        );
    }

    /// 逐 ABI 循环吞掉单个 ABI 的失败正是 `android_per_abi_apks` 判 FAIL 的直接成因，
    /// 所以「循环会不会停」必须是一条会变红的门禁，而不是命令旁边的一句注释。
    #[test]
    fn a_failure_swallowing_abi_loop_is_rejected() {
        let swallowing = "cd mobile/android && for abi in arm64-v8a x86; do gradle -Pyunjian.abis=$abi :app:assembleRelease; done";
        let err = validate_loop_stops_on_failure(swallowing).expect_err("吞失败的循环必须被拒");
        assert!(
            format!("{err:#}").contains("退出码 0"),
            "判词要说清为什么这很危险：{err:#}"
        );

        let stopping = "cd mobile/android && for abi in arm64-v8a x86; do gradle -Pyunjian.abis=$abi :app:assembleRelease || exit 1; done";
        validate_loop_stops_on_failure(stopping).expect("带 `|| exit 1` 的循环应通过");

        // 没有循环的命令不该被这条门禁牵连——tauri_mobile 分支就是单条命令。
        validate_loop_stops_on_failure("cargo tauri android build --apk --aab")
            .expect("没有循环的命令与这条门禁无关");
    }

    /// 入库的那份真机日志必须能导出一个六项齐全的观测，否则 `mobile-size.json` 里
    /// 那条 smoke PASS 又变回不可复现。
    #[test]
    fn the_committed_device_log_yields_a_complete_smoke_observation() {
        let smoke = read_smoke_from_device_log(
            &repo_root().join("docs/reports/mobile-qa-android-measurements.log"),
        )
        .expect("入库的真机日志必须能导出观测");
        assert!(smoke.physical_device, "日志来自 Pixel 8，不是模拟器");
        assert!(
            smoke.device_model.contains("Pixel 8"),
            "型号应来自日志：{}",
            smoke.device_model
        );
        assert_eq!(smoke.os_build, "15/35");
        assert!(smoke.two_character_search, "两字检索有 20 条命中");
        assert!(smoke.typed_recitation_round, "打字背诵答案与参考一致");
        assert!(smoke.voice_session_start_stop, "语音会话真的开口并结束");
        assert_eq!(
            smoke_outcome(Some(&smoke)).verdict,
            Verdict::Pass,
            "六项齐备时才是 PASS"
        );
    }

    /// 模拟器日志必须导出 `physical_device=false`，从而让 smoke 判 FAIL 而不是 PASS。
    /// 判据委派给 `DeviceIdentity::is_physical`，这条测试守住那次委派没被绕开。
    #[test]
    fn an_emulator_log_never_yields_a_physical_device() {
        let sandbox = Sandbox::new();
        let path = sandbox.dir.join("emulator.log");
        fs::write(
            &path,
            "YUNJIAN-FULL device_identity model=sdk_gphone64_x86_64\n\
             YUNJIAN-FULL device_identity os_build=15/35\n\
             YUNJIAN-FULL device_identity ro_hardware=ranchu\n\
             YUNJIAN-FULL device_identity ro_kernel_qemu=1\n\
             YUNJIAN-FULL device_identity fingerprint=google/sdk_gphone64_x86_64/emu:15/x:user/test-keys\n\
             YUNJIAN-FULL two_char_search_returns_results hits=20\n\
             YUNJIAN-FULL typed_recitation_scores_correctly answer_equals_reference=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end spoke=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end native_voice_enabled=true\n",
        )
        .expect("写假日志");

        let smoke = read_smoke_from_device_log(&path).expect("能解析");
        assert!(!smoke.physical_device, "ranchu + qemu=1 是模拟器");
        assert_eq!(
            smoke_outcome(Some(&smoke)).verdict,
            Verdict::Fail,
            "模拟器观测不得算 PASS"
        );
    }

    /// 键缺失时返回 `false` 而不是当成通过。缺证据不等于通过——这条是本仓库
    /// 反复记录的那类错误里最贵的一种。
    #[test]
    fn missing_behaviour_keys_are_not_treated_as_passing() {
        let sandbox = Sandbox::new();
        let path = sandbox.dir.join("partial.log");
        fs::write(
            &path,
            "YUNJIAN-FULL device_identity model=Pixel 8\n\
             YUNJIAN-FULL device_identity os_build=15/35\n\
             YUNJIAN-FULL device_identity ro_hardware=shiba\n\
             YUNJIAN-FULL device_identity ro_kernel_qemu=unset\n\
             YUNJIAN-FULL device_identity fingerprint=google/shiba/shiba:15/x:user/release-keys\n",
        )
        .expect("写假日志");

        let smoke = read_smoke_from_device_log(&path).expect("能解析");
        assert!(smoke.physical_device, "身份齐备");
        assert!(!smoke.two_character_search, "没有 hits 就不是通过");
        assert!(!smoke.typed_recitation_round, "没有答案比对就不是通过");
        assert!(!smoke.voice_session_start_stop, "没有开口记录就不是通过");
    }

    #[test]
    fn a_log_without_any_measurement_line_is_rejected() {
        let sandbox = Sandbox::new();
        let path = sandbox.dir.join("noise.log");
        fs::write(&path, "Gradle build finished\nadb: device offline\n").expect("写假日志");

        let err = read_smoke_from_device_log(&path).expect_err("不是真机日志就该报错");
        assert!(
            format!("{err:#}").contains("YUNJIAN-FULL"),
            "判词要指名缺的是什么：{err:#}"
        );
    }

    #[test]
    fn a_loop_that_lets_apks_overwrite_each_other_is_rejected() {
        let overwriting = "cd mobile/android && for abi in arm64-v8a x86; do gradle -Pyunjian.abis=$abi :app:assembleRelease || exit 1; done";
        let err = validate_per_abi_artifacts_are_collected(overwriting)
            .expect_err("不搬运的循环必须被拒");
        assert!(
            format!("{err:#}").contains("互相覆盖"),
            "判词要说清为什么只剩一个：{err:#}"
        );

        let collecting = "cd mobile/android && for abi in arm64-v8a x86; do gradle -Pyunjian.abis=$abi :app:assembleRelease || exit 1; cp app/build/outputs/apk/release/app-release.apk ../../target/mobile/dist/yunjian-$abi-release.apk || exit 1; done";
        validate_per_abi_artifacts_are_collected(collecting).expect("带搬运的循环应通过");

        validate_per_abi_artifacts_are_collected("cargo tauri android build --apk --aab")
            .expect("没有循环的命令与这条门禁无关");
    }

    /// 仓库里那条真实命令必须自己就过这两道门禁。只测构造出来的字符串会让配置
    /// 悄悄改回裸循环而门禁毫无察觉。
    #[test]
    fn the_shipped_android_command_survives_both_loop_gates() {
        let config = read_config(&repo_root()).expect("读取分发配置");
        validate_loop_stops_on_failure(&config.android.build_command)
            .expect("仓库里的 Android 构建命令必须在单 ABI 失败时停下");
        validate_per_abi_artifacts_are_collected(&config.android.build_command)
            .expect("仓库里的 Android 构建命令必须把每个 ABI 的 APK 搬到各自的名字上");
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
