//! 移动端可行性门禁。
//!
//! 本模块只把**实际机器测量**变成框架选型，不用 mock、模拟器或宿主机结果顶替物理设备。
//! 当前仓库没有可由 CI 凭空取得的 USB 授权、Apple 签名身份或 App Store Connect 凭据，
//! 因此缺少前置条件时仍写出结构完整的报告，但四项均为 `NOT EXECUTED`，总裁决为
//! `undetermined`。这与产品实测失败不同：只有实际执行后的 `FAIL` 才选择
//! `uniffi_native`，也只有四项全 `PASS` 才选择 `tauri_mobile`。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use super::{Platform, Verdict, commit_sha, device_farm, os_build, today};
use crate::verify_sources::emit;

mod full;

#[cfg(test)]
pub(crate) use full::{FULL_DECLARED, build_unexecuted_full_report, validate_full_report_json};

pub(super) fn run_full(root: &Path, platform: Platform) -> Result<()> {
    full::run(root, platform)
}

const REPORT_JSON: &str = "docs/reports/mobile-spike.json";
const REPORT_MARKDOWN: &str = "docs/reports/mobile-spike.md";
const EVIDENCE_LOG: &str = ".omo/evidence/task-68-yunjian.log";

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeclaredCriterion {
    pub(crate) id: &'static str,
    pub(crate) what: &'static str,
    pub(crate) threshold: &'static str,
    pub(crate) required_measurements: &'static [&'static str],
    driver: &'static str,
    executable_when: &'static str,
}

pub(crate) const DECLARED: &[DeclaredCriterion] = &[
    DeclaredCriterion {
        id: "microphone_capture",
        what: "经 todo 46 权限插件在物理 Android 与物理 iOS 设备上采集麦克风 PCM",
        threshold: "Android 与 iOS 均满足 sample_rate_hz == 16000、channel_count == 1、rms > 0",
        required_measurements: &[
            "device_model",
            "os_build",
            "sample_rate_hz",
            "channel_count",
            "rms",
            "permission_plugin",
        ],
        driver: "Android: adb + instrumented test APK；iOS: xcrun devicectl + XCUITest bundle",
        executable_when: "同时备妥已授权 USB 调试的物理 Android、已注册到签名身份的物理 iOS、两端已安装的 instrumented 测试包，并授予麦克风权限",
    },
    DeclaredCriterion {
        id: "corpus_materialization",
        what: "在中端物理 Android 上走生产下载路径校验并原子解压发布语料",
        threshold: "下载 .db.gz、SHA-256 校验成功、原子落入 app storage，duration_seconds < 60 且 crashed == false",
        required_measurements: &[
            "device_model",
            "os_build",
            "artifact_bytes",
            "sha256_verified",
            "duration_seconds",
            "atomic_install",
            "crashed",
            "production_path",
        ],
        driver: "adb + instrumented test APK，调用与生产 corpus fetch 相同的下载、校验、解压和原子替换路径",
        executable_when: "连接一台已授权 USB 调试的中端物理 Android，安装调用生产语料物化路径的 instrumented test APK，并提供可下载的 .db.gz 发布工件与 SHA-256",
    },
    DeclaredCriterion {
        id: "chinese_ime",
        what: "在 targetSdk 35 的物理 Android 上用中文输入法向检索框输入中文",
        threshold: "target_sdk == 35、中文提交成功、keyboard_overlap_px == 0，输入框始终可见且 visualViewport 正常更新",
        required_measurements: &[
            "device_model",
            "os_build",
            "target_sdk",
            "entered_text",
            "keyboard_overlap_px",
            "input_visible",
            "visual_viewport_updated",
        ],
        driver: "adb + targetSdk 35 instrumented test APK；物理键盘输入法交互由设备端测试记录 viewport 与控件边界",
        executable_when: "连接已授权的物理 Android，安装 targetSdk 35 测试 APK，启用可输入中文的软键盘，并由设备端 instrumentation 记录输入文本、键盘遮挡和 visualViewport",
    },
    DeclaredCriterion {
        id: "ios_testflight_submission",
        what: "用 Xcode 26 / iOS 26 SDK 完成一次真实 archive、链接与 TestFlight 上传",
        threshold: "xcode_major >= 26、ios_sdk_major >= 26、archive_link_succeeded == true、upload_succeeded == true 且 testflight_build_id 非空",
        required_measurements: &[
            "device_model",
            "os_build",
            "xcode_version",
            "ios_sdk_version",
            "archive_link_succeeded",
            "upload_succeeded",
            "testflight_build_id",
        ],
        driver: "xcrun devicectl + XCUITest bundle + xcodebuild archive + App Store Connect upload",
        executable_when: "在安装 Xcode 26 与 iOS 26 SDK 的 macOS 上连接已注册到签名身份的物理 iOS 设备，配置 Distribution 证书、provisioning profile 与 App Store Connect 上传凭据",
    },
];

/// 移动框架选型。第三态防止把未执行伪造成产品失败或成功。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelectionVerdict {
    TauriMobile,
    UniffiNative,
    Undetermined,
}

impl SelectionVerdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::TauriMobile => "tauri_mobile",
            Self::UniffiNative => "uniffi_native",
            Self::Undetermined => "undetermined",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CriterionResult {
    pub(crate) id: &'static str,
    what: &'static str,
    pub(crate) threshold: &'static str,
    driver: &'static str,
    pub(crate) verdict: Verdict,
    pub(crate) measurement: BTreeMap<&'static str, Value>,
    detail: String,
    pub(crate) executable_when: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct MobileReport {
    schema_version: u32,
    set: &'static str,
    requested_platform: String,
    generated_date: String,
    commit_sha: String,
    host_os_build: String,
    physical_devices: Vec<PhysicalDevice>,
    preflight: ToolProbe,
    pub(crate) verdict: SelectionVerdict,
    pub(crate) criteria: Vec<CriterionResult>,
}

#[derive(Debug, Serialize)]
struct PhysicalDevice {
    platform: String,
    model: String,
    os_build: String,
    identifier: String,
}

#[derive(Debug, Serialize)]
struct ToolProbe {
    command: String,
    available: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

pub(crate) fn selection_verdict(verdicts: &[Verdict]) -> SelectionVerdict {
    if verdicts.contains(&Verdict::Fail) {
        SelectionVerdict::UniffiNative
    } else if verdicts.len() == DECLARED.len()
        && verdicts.iter().all(|verdict| *verdict == Verdict::Pass)
    {
        SelectionVerdict::TauriMobile
    } else {
        SelectionVerdict::Undetermined
    }
}

#[cfg(test)]
pub(crate) fn build_unexecuted_report(platform: Platform) -> MobileReport {
    build_report(
        platform,
        ToolProbe {
            command: "not probed".to_owned(),
            available: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "unit-test constructor: platform driver was not invoked".to_owned(),
        },
        Vec::new(),
    )
}

fn build_report(
    platform: Platform,
    preflight: ToolProbe,
    physical_devices: Vec<PhysicalDevice>,
) -> MobileReport {
    let criteria = DECLARED
        .iter()
        .map(|declared| CriterionResult {
            id: declared.id,
            what: declared.what,
            threshold: declared.threshold,
            driver: declared.driver,
            verdict: Verdict::NotExecuted,
            measurement: declared
                .required_measurements
                .iter()
                .map(|key| (*key, Value::Null))
                .collect(),
            detail: format!(
                "NOT EXECUTED：本轮没有满足 `{}` 所需的完整物理设备、测试载体与凭据；没有用模拟器或宿主机数据顶替",
                declared.id
            ),
            executable_when: declared.executable_when.to_owned(),
        })
        .collect::<Vec<_>>();
    let verdict = selection_verdict(
        &criteria
            .iter()
            .map(|criterion| criterion.verdict)
            .collect::<Vec<_>>(),
    );
    MobileReport {
        schema_version: 1,
        set: "spike",
        requested_platform: platform.as_str().to_owned(),
        generated_date: today(),
        commit_sha: commit_sha(&super::repo_root()),
        host_os_build: os_build(),
        physical_devices,
        preflight,
        verdict,
        criteria,
    }
}

pub(super) fn run(root: &Path, platform: Platform) -> Result<()> {
    emit("== 移动端可行性门禁（四项阈值在执行前声明）==");
    emit(&format!("  请求平台 {}  断言集 spike", platform.as_str()));

    let remote =
        device_farm::load(root)?.and_then(|config| device_farm::status(root, &config, platform));
    let preflight = match remote {
        Some(status) => {
            emit(&format!(
                "  远端真机驱动 AWS Device Farm；{}",
                status.reason
            ));
            for step in &status.plan {
                emit(&format!("    调度步骤 {step}"));
            }
            ToolProbe {
                command: status.probe.command,
                available: status.probe.available,
                exit_code: status.probe.exit_code,
                stdout: status.probe.stdout,
                stderr: status.probe.stderr,
            }
        }
        None => probe_driver(platform),
    };
    let report = build_report(platform, preflight, Vec::new());
    validate_consistency(&report)?;
    let paths = write_report(root, &report)?;
    append_evidence(root, &report, &paths)?;

    emit(&format!(
        "  报告 {} 与 {}；证据 {}",
        paths.markdown.display(),
        paths.json.display(),
        root.join(EVIDENCE_LOG).display()
    ));
    emit(&format!(
        "  verdict={}；PASS {} FAIL {} NOT EXECUTED {}",
        report.verdict.as_str(),
        count(&report, Verdict::Pass),
        count(&report, Verdict::Fail),
        count(&report, Verdict::NotExecuted)
    ));

    if report
        .criteria
        .iter()
        .any(|item| item.verdict == Verdict::Fail)
    {
        bail!("移动端实测出现 FAIL，选型已强制为 uniffi_native");
    }
    Ok(())
}

fn probe_driver(platform: Platform) -> ToolProbe {
    let (program, args): (&str, &[&str]) = match platform {
        Platform::Android => ("adb", &["devices", "-l"]),
        Platform::Ios => ("xcrun", &["devicectl", "list", "devices"]),
        Platform::Windows | Platform::MacOs | Platform::Linux => {
            unreachable!("移动探测器只接受 Android 或 iOS")
        }
    };
    let command = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    match Command::new(program).args(args).output() {
        Ok(output) => ToolProbe {
            command,
            available: true,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        },
        Err(error) => ToolProbe {
            command,
            available: false,
            exit_code: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

fn validate_consistency(report: &MobileReport) -> Result<()> {
    if report.criteria.len() != DECLARED.len() {
        bail!(
            "移动报告有 {} 项，预声明 {} 项",
            report.criteria.len(),
            DECLARED.len()
        );
    }
    for (actual, declared) in report.criteria.iter().zip(DECLARED) {
        if actual.id != declared.id || actual.threshold != declared.threshold {
            bail!("移动报告判据 `{}` 与预声明发生漂移", declared.id);
        }
        if actual.verdict == Verdict::NotExecuted && actual.executable_when.trim().is_empty() {
            bail!("判据 `{}` 未执行但没有可执行条件", actual.id);
        }
        for key in declared.required_measurements {
            if !actual.measurement.contains_key(key) {
                bail!("判据 `{}` 缺少测量字段 `{key}`", actual.id);
            }
        }
    }
    let derived = selection_verdict(
        &report
            .criteria
            .iter()
            .map(|criterion| criterion.verdict)
            .collect::<Vec<_>>(),
    );
    if report.verdict != derived {
        bail!(
            "移动选型不一致：报告写 {}，四项结果机械导出 {}",
            report.verdict.as_str(),
            derived.as_str()
        );
    }
    Ok(())
}

struct ReportPaths {
    markdown: PathBuf,
    json: PathBuf,
}

fn write_report(root: &Path, report: &MobileReport) -> Result<ReportPaths> {
    let json = root.join(REPORT_JSON);
    let markdown = root.join(REPORT_MARKDOWN);
    if let Some(parent) = json.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let mut encoded = serde_json::to_string_pretty(report).context("序列化移动实测报告失败")?;
    encoded.push('\n');
    fs::write(&json, encoded).with_context(|| format!("写 {} 失败", json.display()))?;
    fs::write(&markdown, render_markdown(report))
        .with_context(|| format!("写 {} 失败", markdown.display()))?;
    Ok(ReportPaths { markdown, json })
}

fn append_evidence(root: &Path, report: &MobileReport, paths: &ReportPaths) -> Result<()> {
    let path = root.join(EVIDENCE_LOG);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开 {} 失败", path.display()))?;
    writeln!(
        file,
        "\n=== mobile spike: platform={} date={} commit={} ===\ncommand: cargo run -p xtask -- acceptance --platform {} --set spike\npreflight: {}\navailable: {}\nexit_code: {:?}\nstdout:\n{}\nstderr:\n{}\nreport_json: {}\nreport_markdown: {}\nverdict: {}\ncriteria: PASS={} FAIL={} NOT_EXECUTED={}",
        report.requested_platform,
        report.generated_date,
        report.commit_sha,
        report.requested_platform,
        report.preflight.command,
        report.preflight.available,
        report.preflight.exit_code,
        report.preflight.stdout,
        report.preflight.stderr,
        paths.json.display(),
        paths.markdown.display(),
        report.verdict.as_str(),
        count(report, Verdict::Pass),
        count(report, Verdict::Fail),
        count(report, Verdict::NotExecuted),
    )
    .with_context(|| format!("追加 {} 失败", path.display()))?;
    Ok(())
}

fn count(report: &MobileReport, verdict: Verdict) -> usize {
    report
        .criteria
        .iter()
        .filter(|criterion| criterion.verdict == verdict)
        .count()
}

fn render_markdown(report: &MobileReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 移动端可行性实测");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> [!WARNING]\n> **verdict: `{}`。** `NOT EXECUTED` 不是产品失败，也不是通过；\n> 在四项真机判据全部得到 PASS/FAIL 之前，移动端框架选型保持 `undetermined`。",
        report.verdict.as_str()
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## 本次运行");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 项 | 值 |");
    let _ = writeln!(out, "| --- | --- |");
    let _ = writeln!(out, "| 请求平台 | `{}` |", report.requested_platform);
    let _ = writeln!(out, "| 日期 | `{}` |", report.generated_date);
    let _ = writeln!(out, "| 提交 | `{}` |", report.commit_sha);
    let _ = writeln!(out, "| 宿主 OS | `{}` |", report.host_os_build);
    let _ = writeln!(out, "| 前置探测 | `{}` |", report.preflight.command);
    let _ = writeln!(out, "| 工具可用 | `{}` |", report.preflight.available);
    let _ = writeln!(out, "| 工具退出码 | `{:?}` |", report.preflight.exit_code);
    let _ = writeln!(
        out,
        "| 已识别物理设备 | `{}` |",
        report.physical_devices.len()
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "### 前置探测原始输出");
    let _ = writeln!(out);
    let _ = writeln!(out, "```text");
    let _ = writeln!(out, "stdout:\n{}", report.preflight.stdout);
    let _ = writeln!(out, "stderr:\n{}", report.preflight.stderr);
    let _ = writeln!(out, "```");
    let _ = writeln!(out);
    let _ = writeln!(out, "## 四项预声明判据");
    let _ = writeln!(out);
    for criterion in &report.criteria {
        let _ = writeln!(out, "### `{}` · {}", criterion.id, criterion.what);
        let _ = writeln!(out);
        let _ = writeln!(out, "- **verdict**: `{}`", criterion.verdict.as_str());
        let _ = writeln!(out, "- **threshold**: {}", criterion.threshold);
        let _ = writeln!(out, "- **driver**: {}", criterion.driver);
        let _ = writeln!(out, "- **detail**: {}", criterion.detail);
        let _ = writeln!(out, "- **可执行条件**: {}", criterion.executable_when);
        let _ = writeln!(out, "- **measurement**:");
        let _ = writeln!(out);
        let _ = writeln!(out, "```json");
        let measurement = serde_json::to_string_pretty(&criterion.measurement)
            .unwrap_or_else(|error| format!("{{\"serialization_error\":\"{error}\"}}"));
        let _ = writeln!(out, "{measurement}");
        let _ = writeln!(out, "```");
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "## 机械选型规则");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "1. 四项全为 `PASS` → `tauri_mobile`；\n2. 任一项为 `FAIL` → `uniffi_native`；\n3. 没有 `FAIL` 但存在 `NOT EXECUTED` → `undetermined`。\n\n第三态防止把缺设备误写成产品失败，也防止在没有证据时推进移动 shell。"
    );
    out
}
