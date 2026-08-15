//! Todo 71 的移动端真机验收报告器。
//!
//! 物理设备、签名身份和 instrumented/XCUITest 测试载体都不能由 CI 伪造。本模块因此把
//! 十项断言预先冻结；缺任一前置时仍输出完整报告，但每项只能是 `NOT EXECUTED`。报告命令
//! 退出 0 只表示审计记录成功写出，不表示产品通过。

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::super::{Platform, Verdict, commit_sha, device_farm, read_app_version, today};
use crate::verify_sources::emit;

const EVIDENCE_LOG: &str = ".omo/evidence/task-71-yunjian.log";
const DEVICE_PLACEHOLDER: &str = "NOT EXECUTED: 需物理设备";
const ANDROID_DRIVER: &str = "adb devices -l";
const IOS_DRIVER: &str = "xcrun devicectl list devices";

#[derive(Debug, Clone, Copy)]
pub(crate) struct FullDeclared {
    pub(crate) id: &'static str,
    pub(crate) what: &'static str,
    pub(crate) needs_screenshot: bool,
    pub(crate) executable_when: &'static str,
    pub(crate) exact_command: &'static str,
}

const FULL_PREREQUISITES: &str = "已授权 USB 调试的物理 Android 与已注册到签名身份的物理 iOS；已选定 binding 并构建 instrumented test APK/XCUITest bundle；Android SDK/NDK、adb、macOS、Xcode 26、iOS 26 SDK、签名与麦克风权限齐全";
const BOTH_COMMANDS: &str = "Android: adb -s <ANDROID_SERIAL> shell am instrument -w -r -e class top.onethinker.yunjian.FullAcceptanceTest top.onethinker.yunjian.test/androidx.test.runner.AndroidJUnitRunner；iOS: xcodebuild test-without-building -xctestrun <YUNJIAN_XCTESTRUN> -destination 'platform=iOS,id=<IOS_UDID>' -resultBundlePath <YUNJIAN_RESULT_BUNDLE>";

/// **执行前冻结**的十项真机断言。顺序也是 JSON/Markdown 报告顺序。
pub(crate) const FULL_DECLARED: &[FullDeclared] = &[
    FullDeclared {
        id: "install_and_launch",
        what: "安装真实移动端产物并启动到可交互首屏",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "corpus_first_run_materialization",
        what: "首次运行显示语料下载、校验与原子物化进度且不崩溃",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "two_char_search_returns_results",
        what: "输入两字查询“明月”并返回至少一条结果",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "reading_view_citations_and_ai_appreciation",
        what: "阅读页显示带出处的集评与明确标注、未经人工审校的 AI 赏析",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "typed_recitation_scores_correctly",
        what: "完成一轮打字背诵并得到与输入一致的评分结果",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "voice_recitation_round_succeeds_end_to_end",
        what: "语音背诵从授权、采集、识别到反馈端到端成功",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "voice_permission_denied_degrades",
        what: "拒绝麦克风权限后降级到打字模式并显示具体原因",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "chinese_ime_prefilled_field_visible",
        what: "中文输入法向已有内容的字段输入且键盘不遮挡输入框",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "background_return_preserves_layout",
        what: "应用进入后台再返回时页面不空白、视图不折叠",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
    FullDeclared {
        id: "app_exits_cleanly",
        what: "由自动化驱动正常退出且不崩溃、不遗留孤儿进程",
        needs_screenshot: true,
        executable_when: FULL_PREREQUISITES,
        exact_command: BOTH_COMMANDS,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FullAssertion {
    pub(crate) id: String,
    what: String,
    screenshot_required: bool,
    pub(crate) verdict: Verdict,
    pub(crate) detail: String,
    pub(crate) executable_when: String,
    pub(crate) exact_command: String,
    pub(crate) screenshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverProbe {
    command: String,
    available: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FullPlatformReport {
    platform: String,
    pub(crate) physical_device_used: bool,
    pub(crate) device_model: String,
    pub(crate) os_version: String,
    driver_probe: DriverProbe,
    pub(crate) assertions: Vec<FullAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FullReport {
    schema_version: u32,
    set: String,
    generated_date: String,
    pub(crate) app_version: String,
    pub(crate) commit_sha: String,
    pub(crate) all_pass: bool,
    pub(crate) platforms: Vec<FullPlatformReport>,
}

struct ReportPaths {
    markdown: PathBuf,
    json: PathBuf,
}

/// 缺真机时的测试构造器。字段与生产路径完全相同，仅不执行外部驱动探测。
#[cfg(test)]
pub(crate) fn build_unexecuted_full_report() -> FullReport {
    build_report(&super::super::repo_root(), None).expect("测试仓库必须有可读取的应用版本")
}

pub(super) fn run(root: &Path, requested: Platform) -> Result<()> {
    emit("== 移动端真机验收（十项断言在执行之前声明）==");
    emit(&format!(
        "  请求平台 {}  断言集 full  声明 {} 条",
        requested.as_str(),
        FULL_DECLARED.len()
    ));

    let probe = resolve_probe(root, requested)?;
    let mut report = build_report(root, Some((requested, probe)))?;
    merge_existing_other_platform(root, requested, &mut report)?;
    report.all_pass = derived_all_pass(&report);
    validate_full_report(&report)?;
    let encoded = serde_json::to_string_pretty(&report).context("序列化移动真机验收报告失败")?;
    validate_full_report_json(&encoded)?;
    let paths = write_report(root, &report, &encoded)?;
    append_evidence(root, requested, &report, &paths)?;

    let (passed, failed, not_executed) = counts(&report);
    emit(&format!(
        "  报告 {} 与 {}；PASS {passed} FAIL {failed} NOT EXECUTED {not_executed} all_pass={}",
        paths.markdown.display(),
        paths.json.display(),
        report.all_pass
    ));
    if failed > 0 {
        bail!("移动端真机验收出现 {failed} 条 FAIL");
    }
    Ok(())
}

fn merge_existing_other_platform(
    root: &Path,
    requested: Platform,
    report: &mut FullReport,
) -> Result<()> {
    let path = root.join(format!(
        "docs/reports/mobile-qa-{}.json",
        report.generated_date
    ));
    if !path.is_file() {
        return Ok(());
    }
    let encoded = fs::read_to_string(&path)
        .with_context(|| format!("读取既有移动真机报告 {} 失败", path.display()))?;
    let existing: FullReport =
        serde_json::from_str(&encoded).context("解析既有移动真机报告失败")?;
    validate_full_report(&existing)?;
    if existing.app_version != report.app_version || existing.commit_sha != report.commit_sha {
        return Ok(());
    }
    let requested_name = requested.as_str();
    for platform in &mut report.platforms {
        if platform.platform == requested_name {
            continue;
        }
        if let Some(previous) = existing
            .platforms
            .iter()
            .find(|previous| previous.platform == platform.platform)
        {
            *platform = previous.clone();
        }
    }
    Ok(())
}

fn build_report(
    root: &Path,
    requested_probe: Option<(Platform, DriverProbe)>,
) -> Result<FullReport> {
    let platforms = [Platform::Android, Platform::Ios]
        .into_iter()
        .map(|platform| {
            let probe = requested_probe
                .as_ref()
                .filter(|(requested, _)| *requested == platform)
                .map(|(_, probe)| probe.clone())
                .unwrap_or_else(|| unrequested_probe(platform));
            unexecuted_platform(platform, probe)
        })
        .collect::<Vec<_>>();
    let mut report = FullReport {
        schema_version: 1,
        set: "full".to_owned(),
        generated_date: today(),
        app_version: read_app_version(root)?,
        commit_sha: commit_sha(root),
        all_pass: false,
        platforms,
    };
    report.all_pass = derived_all_pass(&report);
    Ok(report)
}

fn unexecuted_platform(platform: Platform, probe: DriverProbe) -> FullPlatformReport {
    let probe_reason = if probe.available {
        format!(
            "驱动探测已执行，但仓库当前没有 gate-selected 移动外壳及可安装的 instrumented/XCUITest 测试载体；探测退出码 {:?}",
            probe.exit_code
        )
    } else {
        format!("驱动不可用：{}", probe.stderr)
    };
    FullPlatformReport {
        platform: platform.as_str().to_owned(),
        physical_device_used: false,
        device_model: DEVICE_PLACEHOLDER.to_owned(),
        os_version: DEVICE_PLACEHOLDER.to_owned(),
        driver_probe: probe,
        assertions: FULL_DECLARED
            .iter()
            .map(|declared| FullAssertion {
                id: declared.id.to_owned(),
                what: declared.what.to_owned(),
                screenshot_required: declared.needs_screenshot,
                verdict: Verdict::NotExecuted,
                detail: format!(
                    "NOT EXECUTED：{probe_reason}；未使用模拟器、mock、宿主机结果或人工操作顶替物理设备"
                ),
                executable_when: declared.executable_when.to_owned(),
                exact_command: declared.exact_command.to_owned(),
                screenshot: None,
            })
            .collect(),
    }
}

/// 优先用 AWS Device Farm 远端真机驱动；未配置时回落到本地 `adb` / `xcrun`。
///
/// 无论走哪条路，产物不齐时十项断言仍是 `NOT EXECUTED`：远端池可达证明的是「有真机」，
/// 不是「真机上装过我们的应用」。
fn resolve_probe(root: &Path, platform: Platform) -> Result<DriverProbe> {
    let Some(status) =
        device_farm::load(root)?.and_then(|config| device_farm::status(root, &config, platform))
    else {
        return Ok(probe_driver(platform));
    };
    emit(&format!(
        "  远端真机驱动 AWS Device Farm；{}",
        status.reason
    ));
    for step in &status.plan {
        emit(&format!("    调度步骤 {step}"));
    }
    Ok(DriverProbe {
        command: status.probe.command,
        available: status.probe.available,
        exit_code: status.probe.exit_code,
        stdout: status.probe.stdout,
        stderr: status.probe.stderr,
    })
}

fn probe_driver(platform: Platform) -> DriverProbe {
    let (program, args): (&str, &[&str]) = match platform {
        Platform::Android => ("adb", &["devices", "-l"]),
        Platform::Ios => ("xcrun", &["devicectl", "list", "devices"]),
        Platform::Windows | Platform::MacOs | Platform::Linux => {
            unreachable!("full 只接受移动平台")
        }
    };
    let command = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    match Command::new(program).args(args).output() {
        Ok(output) => DriverProbe {
            command,
            available: true,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        },
        Err(error) => DriverProbe {
            command,
            available: false,
            exit_code: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

fn unrequested_probe(platform: Platform) -> DriverProbe {
    DriverProbe {
        command: driver_command(platform).to_owned(),
        available: false,
        exit_code: None,
        stdout: String::new(),
        stderr: "本次命令未请求该平台；需在对应宿主机另跑同一断言集".to_owned(),
    }
}

const fn driver_command(platform: Platform) -> &'static str {
    match platform {
        Platform::Android => ANDROID_DRIVER,
        Platform::Ios => IOS_DRIVER,
        Platform::Windows | Platform::MacOs | Platform::Linux => "",
    }
}

/// 从 JSON 重新解析并校验，供终验和失败注入直接调用。
pub(crate) fn validate_full_report_json(encoded: &str) -> Result<()> {
    let report: FullReport = serde_json::from_str(encoded).context("解析移动真机验收 JSON 失败")?;
    validate_full_report(&report)
}

fn validate_full_report(report: &FullReport) -> Result<()> {
    if report.schema_version != 1 || report.set != "full" {
        bail!("移动真机报告 schema_version/set 不受支持");
    }
    for (field, value) in [
        ("generated_date", report.generated_date.as_str()),
        ("app_version", report.app_version.as_str()),
        ("commit_sha", report.commit_sha.as_str()),
    ] {
        if value.trim().is_empty() || value == "unknown" {
            bail!("移动真机报告字段 `{field}` 为空或未知");
        }
    }
    if report.platforms.len() != 2
        || report.platforms[0].platform != "android"
        || report.platforms[1].platform != "ios"
    {
        bail!("移动真机报告必须按 android、ios 顺序各有一节");
    }
    for platform in &report.platforms {
        if platform.device_model.trim().is_empty() || platform.os_version.trim().is_empty() {
            bail!("平台 `{}` 的设备型号或 OS 版本为空", platform.platform);
        }
        if platform.physical_device_used
            && (platform.device_model.starts_with("NOT EXECUTED")
                || platform.os_version.starts_with("NOT EXECUTED"))
        {
            bail!("平台 `{}` 声称使用物理设备却保留占位值", platform.platform);
        }
        if platform.assertions.len() != FULL_DECLARED.len() {
            bail!(
                "平台 `{}` 有 {} 项断言，预声明 {} 项",
                platform.platform,
                platform.assertions.len(),
                FULL_DECLARED.len()
            );
        }
        for (actual, declared) in platform.assertions.iter().zip(FULL_DECLARED) {
            if actual.id != declared.id {
                bail!(
                    "平台 `{}` 出现未声明或顺序漂移的断言 `{}`；期望 `{}`",
                    platform.platform,
                    actual.id,
                    declared.id
                );
            }
            if actual.what != declared.what
                || actual.screenshot_required != declared.needs_screenshot
                || actual.exact_command != declared.exact_command
            {
                bail!(
                    "断言 `{}` 的说明、截图要求或自动化命令与预声明发生漂移",
                    actual.id
                );
            }
            if actual.what.trim().is_empty()
                || actual.detail.trim().is_empty()
                || actual.exact_command.trim().is_empty()
            {
                bail!("断言 `{}` 的说明、依据或确切命令为空", actual.id);
            }
            if actual.verdict == Verdict::NotExecuted && actual.executable_when.trim().is_empty() {
                bail!("断言 `{}` 未执行但没有可执行条件", actual.id);
            }
            if actual.verdict == Verdict::Pass
                && declared.needs_screenshot
                && actual.screenshot.is_none()
            {
                bail!("断言 `{}` 判为 PASS 但没有截图", actual.id);
            }
            if !platform.physical_device_used && actual.verdict != Verdict::NotExecuted {
                bail!(
                    "平台 `{}` 没有物理设备记录，断言 `{}` 只能是 NOT EXECUTED",
                    platform.platform,
                    actual.id
                );
            }
        }
    }
    let derived = derived_all_pass(report);
    if report.all_pass != derived {
        bail!(
            "移动真机报告 all_pass={}，逐项机械导出为 {derived}",
            report.all_pass
        );
    }
    Ok(())
}

fn derived_all_pass(report: &FullReport) -> bool {
    report.platforms.len() == 2
        && report.platforms.iter().all(|platform| {
            platform.physical_device_used
                && platform.assertions.len() == FULL_DECLARED.len()
                && platform
                    .assertions
                    .iter()
                    .all(|assertion| assertion.verdict == Verdict::Pass)
        })
}

fn counts(report: &FullReport) -> (usize, usize, usize) {
    let assertions = report
        .platforms
        .iter()
        .flat_map(|platform| platform.assertions.iter());
    assertions.fold((0, 0, 0), |mut counts, assertion| {
        match assertion.verdict {
            Verdict::Pass => counts.0 += 1,
            Verdict::Fail => counts.1 += 1,
            Verdict::NotExecuted => counts.2 += 1,
        }
        counts
    })
}

fn write_report(root: &Path, report: &FullReport, encoded: &str) -> Result<ReportPaths> {
    let dir = root.join("docs/reports");
    fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    let stem = format!("mobile-qa-{}", report.generated_date);
    let json = dir.join(format!("{stem}.json"));
    let markdown = dir.join(format!("{stem}.md"));
    fs::write(&json, format!("{encoded}\n"))
        .with_context(|| format!("写 {} 失败", json.display()))?;
    fs::write(&markdown, render_markdown(report))
        .with_context(|| format!("写 {} 失败", markdown.display()))?;
    Ok(ReportPaths { markdown, json })
}

fn render_markdown(report: &FullReport) -> String {
    let mut out = String::new();
    let (passed, failed, not_executed) = counts(report);
    let _ = writeln!(out, "# 移动端真机验收 · {}", report.generated_date);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> [!WARNING]\n> **`all_pass = {}`；物理 Android 与 iOS 未全部完成前不得解读为通过。**\n> `NOT EXECUTED` 表示缺真机、授权、签名或测试载体；没有用模拟器、mock、宿主机数据或人工操作顶替。",
        report.all_pass
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "- 断言集：`{}`", report.set);
    let _ = writeln!(out, "- 应用版本：`{}`", report.app_version);
    let _ = writeln!(out, "- 提交：`{}`", report.commit_sha);
    let _ = writeln!(
        out,
        "- 汇总：PASS {passed} / FAIL {failed} / NOT EXECUTED {not_executed}"
    );
    let _ = writeln!(out);
    for platform in &report.platforms {
        let _ = writeln!(out, "## {}", platform.platform);
        let _ = writeln!(out);
        let _ = writeln!(out, "- 物理设备：`{}`", platform.physical_device_used);
        let _ = writeln!(out, "- 设备型号：`{}`", platform.device_model);
        let _ = writeln!(out, "- OS 版本：`{}`", platform.os_version);
        let _ = writeln!(out, "- 驱动探测：`{}`", platform.driver_probe.command);
        let _ = writeln!(out, "- 驱动可用：`{}`", platform.driver_probe.available);
        let _ = writeln!(out);
        let _ = writeln!(out, "| 断言 | 裁决 | 依据 / 可执行条件 / 确切命令 | 截图 |");
        let _ = writeln!(out, "| --- | --- | --- | --- |");
        for assertion in &platform.assertions {
            let screenshot = assertion
                .screenshot
                .as_deref()
                .map_or_else(|| "—".to_owned(), |path| format!("[`{path}`]({path})"));
            let _ = writeln!(
                out,
                "| `{}`<br>{} | **{}** | {}<br>**可执行条件**：{}<br>**确切命令**：`{}` | {} |",
                assertion.id,
                assertion.what,
                assertion.verdict.as_str(),
                assertion.detail,
                assertion.executable_when,
                assertion.exact_command,
                screenshot
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "### 驱动原始输出");
        let _ = writeln!(out);
        let _ = writeln!(out, "```text");
        let _ = writeln!(out, "stdout:\n{}", platform.driver_probe.stdout);
        let _ = writeln!(out, "stderr:\n{}", platform.driver_probe.stderr);
        let _ = writeln!(out, "```");
        let _ = writeln!(out);
    }
    out
}

fn append_evidence(
    root: &Path,
    requested: Platform,
    report: &FullReport,
    paths: &ReportPaths,
) -> Result<()> {
    let path = root.join(EVIDENCE_LOG);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开 {} 失败", path.display()))?;
    let (passed, failed, not_executed) = counts(report);
    writeln!(
        file,
        "\n=== mobile full: platform={} date={} commit={} ===\ncommand: cargo run -p xtask -- acceptance --platform {} --set full\nreport_json: {}\nreport_markdown: {}\nall_pass: {}\nassertions: PASS={} FAIL={} NOT_EXECUTED={}",
        requested.as_str(),
        report.generated_date,
        report.commit_sha,
        requested.as_str(),
        paths.json.display(),
        paths.markdown.display(),
        report.all_pass,
        passed,
        failed,
        not_executed
    )
    .with_context(|| format!("追加 {} 失败", path.display()))?;
    Ok(())
}
