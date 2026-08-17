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

use super::measurements::{self, MeasurementsByCriterion};
use super::{Platform, Verdict, commit_sha, device_farm, os_build, today};
use crate::verify_sources::emit;

mod full;
mod full_criteria;

#[cfg(test)]
pub(crate) use full::{FULL_DECLARED, build_unexecuted_full_report, validate_full_report_json};

pub(super) fn run_full(root: &Path, platform: Platform) -> Result<()> {
    full::run(root, platform)
}

const REPORT_JSON: &str = "docs/reports/mobile-spike.json";
const REPORT_MARKDOWN: &str = "docs/reports/mobile-spike.md";
const EVIDENCE_LOG: &str = ".omo/evidence/task-68-yunjian.log";

/// 真机日志的落点。由 `aws devicefarm` 一轮 run 的 artifacts 下载而来，
/// 内容是 `.aws/devicefarm/spike-measure.sh` 打印的 `YUNJIAN-MEASURE` 行。
///
/// 文件不存在时四条判据保持 `NOT EXECUTED`——**缺日志不是产品失败**。
const DEVICE_LOG: &str = "docs/reports/device-farm-android-measurements.log";

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
        what: "经 todo 46 声明的权限路径在物理 Android 与物理 iOS 设备上采集麦克风 PCM",
        // `permission_plugin` 一项的阈值在 2026-08-15 由「插件类参与」修订为「插件的可观测
        // 契约成立」，理由记在 `docs/reports/mobile-spike.md` 的「两处判据措辞修订」一节：
        // todo 46 的 `AudioPermissionPlugin` 是 Tauri 插件，只有被 Rust 侧 `tauri::plugin`
        // 注册才存在于进程里，而那份注册属于 todo 69 的 `tauri_mobile` 分支——正是本门禁
        // 要决定的事。让判据依赖它就成了循环：门禁等 binding，binding 等门禁。
        // 采集参数与 RMS 三项阈值**一字未改**。
        threshold: "sample_rate_hz == 16000、channel_count == 1、rms > 0，且 permission_plugin == record_audio_granted+modify_audio_settings_granted（todo 46 声明的两条权限在包内齐备且录音权限运行期已授予）",
        required_measurements: &[
            "device_model",
            "os_build",
            "sample_rate_hz",
            "channel_count",
            "rms",
            "permission_plugin",
        ],
        driver: "Android: adb + instrumented test APK（SpikeMicrophoneTest）；iOS: xcrun devicectl + XCUITest bundle",
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
        what: "在 targetSdk 35 的物理 Android 上用中文输入法向边到边窗口的检索框输入中文",
        // `target_sdk == 35` **保持原样**。PR #102 在真机上量到 36 不是阈值定错了，
        // 而是构建从未把它钉住（tauri 模板取 compileSdk 默认值，AGP 又自动下载缺失平台）。
        // 修构建比改阈值正确：判据要求在 35 上测，`mobile/android/spike/`
        // 的 gradle 片段就把 `targetSdk = 35` 写进应用模块。
        //
        // 新增的必需项是 `edge_to_edge`。少了它，`keyboard_overlap_px == 0` 证明不了东西：
        // 非边到边窗口里系统会自己替应用避让键盘，遮挡天然为 0，而判据引用的恰是
        // edge-to-edge 与 visualViewport 那两个长期缺陷。
        threshold: "target_sdk == 35、edge_to_edge == true、中文提交成功、keyboard_overlap_px == 0，输入框始终可见且 visualViewport 正常更新",
        required_measurements: &[
            "device_model",
            "os_build",
            "target_sdk",
            "edge_to_edge",
            "entered_text",
            "keyboard_overlap_px",
            "input_visible",
            "visual_viewport_updated",
        ],
        driver: "adb + targetSdk 35 instrumented test APK（SpikeImeTest 驱动 SpikeWebViewActivity）；键盘交互由设备端测试记录 viewport 与控件边界",
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
        executable_when: "用户已决定不做商店提交，本判据因此在范围外，不会有测量值。若日后恢复：在安装 Xcode 26 与 iOS 26 SDK 的 macOS 上连接已注册到签名身份的物理 iOS 设备，配置 Distribution 证书、provisioning profile 与 App Store Connect 上传凭据",
    },
];

/// 判据①要求的权限路径取值。写成常量而不是散在阈值函数与设备端字符串里，
/// 是因为两边必须逐字一致：设备端报 `record_audio_granted+modify_audio_settings_granted`
/// 而这里写别的，结果会是一个读不出原因的 FAIL。
const PERMISSION_PATH_READY: &str = "record_audio_granted+modify_audio_settings_granted";

/// 预声明阈值的修订记录。门禁的价值取决于阈值不能事后被谈掉，因此任何一次修订都必须
/// 留在报告里，而不是只留在提交记录里。
const AMENDMENTS: &[&str] = &[
    "判据①的 `permission_plugin`：由「todo 46 的 Tauri 插件类参与采集」修订为「该插件的可观测契约成立」，即两条权限在已安装包内声明齐备且 `RECORD_AUDIO` 运行期已授予。理由：那个类是 Tauri 插件，只有被 Rust 侧 `tauri::plugin` 注册才存在于进程里，而这份注册属于 todo 69 的 `tauri_mobile` 分支——正是本门禁要决定的事，依赖它会形成循环。采集参数与 RMS 三项阈值一字未改。",
    "判据③的 `target_sdk == 35`：**未修订**。PR #102 真机实测 36 是构建缺陷而非阈值错误（tauri 模板取 compileSdk 默认值，AGP 又自动下载缺失平台）；修正方式是在应用模块把 `targetSdk` 钉在 35，而不是放宽判据。",
    "判据③新增必需项 `edge_to_edge`：非边到边窗口里系统会替应用避让键盘，遮挡天然为 0，那样的 PASS 证明不了产品自己处理了 ime 插入——而判据引用的正是 edge-to-edge 与 visualViewport 两个长期缺陷。",
];

/// 顶层裁决的语义说明，写进报告。
///
/// 三态各有各的话要说，所以按裁决取文案而不是写一句放之四海的套话——一份写着
/// `uniffi_native` 却在解释 `undetermined` 为何合理的报告，比没有说明更糟。
///
/// 关键判断在 `Undetermined` 那一支：判据④（iOS TestFlight）**用户已决定不做**，它既不是
/// FAIL 也不是 PASS。把 `NOT EXECUTED` 当 FAIL 会让 `uniffi_native` 被一个从未测过的结论
/// 选中；当 PASS 会让 `tauri_mobile` 建立在一次没做过的上传上。两者都不可接受，所以在没有
/// FAIL 时顶层只能停在 `undetermined`。
///
/// 反过来，**一旦出现实测 FAIL，判据④的范围问题就不再影响结论**：FAIL 在机械规则里是
/// 决定性的，不需要知道④的结果也能定选型。
fn verdict_rationale(verdict: SelectionVerdict) -> &'static str {
    match verdict {
        SelectionVerdict::UniffiNative => {
            "存在实测 FAIL，机械规则据此选择 uniffi_native。FAIL 是决定性的：不需要知道判据④（iOS TestFlight，用户已决定不做）的结果也能定选型，因此④的范围问题不影响本次结论。要改变它只能靠让那条 FAIL 在真机上变成 PASS，而不是重新解释阈值——阈值在执行前已声明，事后放宽等于把门禁谈掉。"
        }
        SelectionVerdict::TauriMobile => "四条判据全部实测 PASS，机械规则据此选择 tauri_mobile。",
        SelectionVerdict::Undetermined => {
            "没有实测 FAIL，但存在 NOT EXECUTED，故顶层保持 undetermined。其中判据④ iOS TestFlight 提交经用户决定不做，处于门禁范围外：它不是 FAIL（没有实测失败），也不是 PASS（没有成功上传）。机械规则要求四条全 PASS 才选 tauri_mobile、任一 FAIL 才选 uniffi_native，两者都不成立。结束它需要用户决定：正式把判据④移出门禁（于是三条判据的结果直接决定选型），或恢复 iOS 提交并实测。"
        }
    }
}

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
    pub(crate) detail: String,
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
    pub(crate) verdict_rationale: &'static str,
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
        unprobed(),
        Vec::new(),
        &MeasurementsByCriterion::new(),
    )
}

/// 用一段设备侧测量日志构造报告。
///
/// 一致性测试必须走**真实的解析 + 判决 + 导出**这条链，而不是手搓 `CriterionResult`：
/// 手搓能造出「阈值函数从未被调用过」的报告，于是「注入一个 FAIL 会强制 uniffi_native」
/// 这条断言就绕过了它本该保护的那段代码。
#[cfg(test)]
pub(crate) fn build_report_from_device_log(platform: Platform, log: &str) -> MobileReport {
    build_report(
        platform,
        unprobed(),
        Vec::new(),
        &measurements::parse_measurements(log),
    )
}

#[cfg(test)]
fn unprobed() -> ToolProbe {
    ToolProbe {
        command: "not probed".to_owned(),
        available: false,
        exit_code: None,
        stdout: String::new(),
        stderr: "unit-test constructor: platform driver was not invoked".to_owned(),
    }
}

/// 四条判据的阈值判定。**只在必需测量值全部齐备后才会被调用**，
/// 所以这里可以直接取值，不必再处理缺键。
fn threshold_for(
    id: &str,
) -> fn(&std::collections::BTreeMap<String, String>) -> Result<(), String> {
    match id {
        "microphone_capture" => |values| {
            let rate = values.get("sample_rate_hz").map(String::as_str);
            if rate != Some("16000") {
                return Err(format!("识别器硬要求 16 kHz，实测 {rate:?}"));
            }
            let channels = values.get("channel_count").map(String::as_str);
            if channels != Some("1") {
                return Err(format!("识别器硬要求单声道，实测 {channels:?}"));
            }
            let rms = values
                .get("rms")
                .and_then(|raw| raw.parse::<f64>().ok())
                .unwrap_or(0.0);
            if rms <= 0.0 {
                return Err(format!("采到的是静音，RMS={rms}"));
            }
            let path = values.get("permission_plugin").map(String::as_str);
            if path != Some(PERMISSION_PATH_READY) {
                return Err(format!(
                    "todo 46 的权限路径未成立，要求 {PERMISSION_PATH_READY}，实测 {path:?}"
                ));
            }
            Ok(())
        },
        "corpus_materialization" => |values| {
            if values.get("sha256_verified").map(String::as_str) != Some("true") {
                return Err("SHA-256 校验未通过".to_owned());
            }
            if values.get("atomic_install").map(String::as_str) != Some("true") {
                return Err("语料未原子落盘".to_owned());
            }
            if values.get("crashed").map(String::as_str) == Some("true") {
                return Err("物化过程中应用崩溃".to_owned());
            }
            let seconds = values
                .get("duration_seconds")
                .and_then(|raw| raw.parse::<f64>().ok())
                .unwrap_or(f64::MAX);
            if seconds >= 60.0 {
                return Err(format!("物化耗时 {seconds} 秒，超过 60 秒阈值"));
            }
            Ok(())
        },
        "chinese_ime" => |values| {
            if values.get("target_sdk").map(String::as_str) != Some("35") {
                return Err(format!(
                    "判据要求 targetSdk 35，实测 {:?}",
                    values.get("target_sdk")
                ));
            }
            if values.get("edge_to_edge").map(String::as_str) != Some("true") {
                return Err(
                    "窗口不在 edge-to-edge 下，键盘遮挡为零只是系统替应用避让的结果，证明不了产品自己处理了 ime 插入".to_owned(),
                );
            }
            let overlap = values
                .get("keyboard_overlap_px")
                .and_then(|raw| raw.parse::<f64>().ok())
                .unwrap_or(f64::MAX);
            if overlap != 0.0 {
                return Err(format!("键盘遮挡输入框 {overlap} px"));
            }
            if values.get("input_visible").map(String::as_str) != Some("true") {
                return Err("输入框在输入过程中不可见".to_owned());
            }
            if values.get("visual_viewport_updated").map(String::as_str) != Some("true") {
                return Err("visualViewport 未随键盘更新".to_owned());
            }
            if values
                .get("entered_text")
                .is_none_or(|text| text.trim().is_empty())
            {
                return Err("没有成功提交中文文本".to_owned());
            }
            Ok(())
        },
        // iOS TestFlight。**用户已决定不发商店**，因此这条判据不会有测量值，
        // 永远停在 NOT EXECUTED；阈值函数保留只为让四条判据的处理保持同构。
        _ => |values| {
            if values.get("upload_succeeded").map(String::as_str) != Some("true") {
                return Err("TestFlight 上传未成功".to_owned());
            }
            Ok(())
        },
    }
}

fn build_report(
    platform: Platform,
    preflight: ToolProbe,
    physical_devices: Vec<PhysicalDevice>,
    device: &MeasurementsByCriterion,
) -> MobileReport {
    let criteria = DECLARED
        .iter()
        .map(|declared| {
            let outcome = measurements::judge(
                declared.required_measurements,
                device.get(declared.id),
                threshold_for(declared.id),
            );
            CriterionResult {
                id: declared.id,
                what: declared.what,
                threshold: declared.threshold,
                driver: declared.driver,
                verdict: outcome.verdict,
                measurement: outcome.measurement,
                detail: outcome.detail,
                executable_when: declared.executable_when.to_owned(),
            }
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
        verdict_rationale: verdict_rationale(verdict),
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
    let device = load_device_measurements(root, platform);
    let report = build_report(platform, preflight, Vec::new(), &device);
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

/// 读取真机回传的测量日志。
///
/// 只有 Android 有这条路：iOS 侧连产物都造不出来（缺 macOS 与 Xcode），
/// 给它读一个永远不存在的文件只会让日志里多一行误导性的「未找到」。
fn load_device_measurements(root: &Path, platform: Platform) -> MeasurementsByCriterion {
    if platform != Platform::Android {
        return MeasurementsByCriterion::new();
    }
    let path = root.join(DEVICE_LOG);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = measurements::parse_measurements(&text);
            emit(&format!(
                "  真机测量日志 {}；识别到 {} 条判据的回传",
                path.display(),
                parsed.len()
            ));
            parsed
        }
        Err(_) => {
            emit(&format!(
                "  未找到真机测量日志 {}；四条判据保持 NOT EXECUTED",
                path.display()
            ));
            MeasurementsByCriterion::new()
        }
    }
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

pub(crate) fn validate_consistency(report: &MobileReport) -> Result<()> {
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
        "> [!WARNING]\n> **verdict: `{}`。** `NOT EXECUTED` 既不是产品失败也不是通过；只有实测 FAIL 才选 `uniffi_native`，只有四条全 PASS 才选 `tauri_mobile`。\n>\n> {}",
        report.verdict.as_str(),
        report.verdict_rationale
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
    let _ = writeln!(out);
    let _ = writeln!(out, "## 本次顶层裁决的语义");
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", report.verdict_rationale);
    let _ = writeln!(out);
    let _ = writeln!(out, "## 预声明阈值的修订记录");
    let _ = writeln!(out);
    for amendment in AMENDMENTS {
        let _ = writeln!(out, "- {amendment}");
    }
    out
}
