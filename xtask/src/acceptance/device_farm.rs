//! AWS Device Farm 远端真机驱动的解析与前置探测。
//!
//! 本模块**只回答两件事**：远端真机池是否真的可达，以及跑一轮实测还缺哪些产物。
//! 它不产生任何 verdict——判据的 PASS/FAIL 只能由设备回传的测量值决定，缺产物时
//! 上层仍写 `NOT EXECUTED`。这样做的理由是 Device Farm 的真机确实能解开「没有物理
//! 设备」这一条阻塞，但**解不开「没有可安装的移动产物」**：`mobile/android` 与
//! `mobile/ios` 目前只有 README，仓库里不存在 APK 或 `.ipa`。把远端池可达误读成
//! 判据可执行，等于用「云上有设备」冒充「设备上跑过我们的应用」。
//!
//! 区域差异必须写在代码里而不是留给记忆：Device Farm **只在 `us-west-2`**，而本项目
//! 的 CodeBuild runner 在 `us-east-2`。因此 region 是配置项，且默认值就是 `us-west-2`。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::Platform;

pub(crate) const CONFIG_PATH: &str = "mobile/device-farm.toml";

/// Device Farm 只在这个区域提供服务；换区不是配置问题而是服务不存在。
pub(crate) const ONLY_REGION: &str = "us-west-2";

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) schema_version: u32,
    pub(crate) region: String,
    pub(crate) project_arn: String,
    pub(crate) billing_method: String,
    pub(crate) locale: String,
    pub(crate) job_timeout_minutes: u32,
    pub(crate) android: PlatformConfig,
    pub(crate) ios: PlatformConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlatformConfig {
    /// 该平台的远端链路是否已打通。iOS 目前为 `false`：Device Farm 会用通配
    /// provisioning profile 重签名，但**构建 `.ipa` 仍需 macOS 与 Xcode**。
    pub(crate) enabled: bool,
    pub(crate) device_pool_arn: String,
    pub(crate) app_artifact: String,
    pub(crate) app_upload_type: String,
    pub(crate) test_package_artifact: String,
    pub(crate) test_package_upload_type: String,
    pub(crate) test_spec: String,
    pub(crate) test_type: String,
    /// 人或 CI 为产出上面两个产物必须先跑的确切命令。写进报告的
    /// `executable_when`，让「未执行」带上可执行条件而不是空话。
    pub(crate) build_command: String,
    pub(crate) blocked_reason: String,
}

impl Config {
    pub(crate) fn platform(&self, platform: Platform) -> Option<&PlatformConfig> {
        match platform {
            Platform::Android => Some(&self.android),
            Platform::Ios => Some(&self.ios),
            Platform::Windows | Platform::MacOs | Platform::Linux => None,
        }
    }
}

/// 远端探测结果。字段与两个报告结构里的 probe 同形，由调用方各自映射。
#[derive(Debug, Clone)]
pub(crate) struct RemoteProbe {
    pub(crate) command: String,
    pub(crate) available: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// 读配置；文件不存在时返回 `None`，让上层回落到本地 `adb` / `xcrun` 探测。
pub(crate) fn load(root: &Path) -> Result<Option<Config>> {
    let path = root.join(CONFIG_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
    parse(&text).map(Some)
}

pub(crate) fn parse(text: &str) -> Result<Config> {
    let config: Config = toml::from_str(text).context("解析 Device Farm 配置失败")?;
    if config.schema_version != 1 {
        anyhow::bail!(
            "Device Farm 配置 schema_version 期望 1，实际 {}",
            config.schema_version
        );
    }
    if config.region != ONLY_REGION {
        anyhow::bail!(
            "Device Farm 只在 {ONLY_REGION} 提供服务，配置写的是 {}；换区不是配置问题",
            config.region
        );
    }
    Ok(config)
}

/// 列出该平台还缺的产物。空表示产物齐备，可以真正调度一轮实测。
pub(crate) fn missing_artifacts(root: &Path, platform: &PlatformConfig) -> Vec<PathBuf> {
    [
        platform.app_artifact.as_str(),
        platform.test_package_artifact.as_str(),
        platform.test_spec.as_str(),
    ]
    .into_iter()
    .filter(|relative| !relative.trim().is_empty())
    .map(|relative| root.join(relative))
    .filter(|path| !path.exists())
    .collect()
}

/// 用一次 `get-device-pool` 同时证明凭据、区域与设备池三件事。
///
/// 刻意不用 `list-devices`：那个调用不需要账号内资源存在也会成功，证不了设备池已就位。
pub(crate) fn probe(config: &Config, platform: &PlatformConfig) -> RemoteProbe {
    let args = [
        "devicefarm",
        "get-device-pool",
        "--arn",
        platform.device_pool_arn.as_str(),
        "--region",
        config.region.as_str(),
        "--output",
        "json",
    ];
    let command = std::iter::once("aws")
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    match Command::new("aws").args(args).output() {
        Ok(output) => RemoteProbe {
            command,
            available: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        },
        Err(error) => RemoteProbe {
            command,
            available: false,
            exit_code: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

/// 渲染出把产物送上真机所需的**确切命令序列**。
///
/// 报告里的「未执行」必须带上可照抄的配方，否则读者只知道缺东西、不知道下一步敲什么。
pub(crate) fn schedule_plan(config: &Config, platform: &PlatformConfig) -> Vec<String> {
    vec![
        format!(
            "aws devicefarm create-upload --project-arn {} --name {} --type {} --region {}",
            config.project_arn,
            file_name(&platform.app_artifact),
            platform.app_upload_type,
            config.region
        ),
        format!(
            "aws devicefarm create-upload --project-arn {} --name {} --type {} --region {}",
            config.project_arn,
            file_name(&platform.test_package_artifact),
            platform.test_package_upload_type,
            config.region
        ),
        format!(
            "aws devicefarm create-upload --project-arn {} --name {} --type {}_TEST_SPEC --region {}",
            config.project_arn,
            file_name(&platform.test_spec),
            platform.test_type,
            config.region
        ),
        format!(
            "aws devicefarm schedule-run --project-arn {} --app-arn <APP> --device-pool-arn {} --test '{{\"type\":\"{}\",\"testPackageArn\":\"<PKG>\",\"testSpecArn\":\"<SPEC>\"}}' --configuration '{{\"locale\":\"{}\",\"billingMethod\":\"{}\"}}' --execution-configuration '{{\"jobTimeoutMinutes\":{}}}' --region {}",
            config.project_arn,
            platform.device_pool_arn,
            platform.test_type,
            config.locale,
            config.billing_method,
            config.job_timeout_minutes,
            config.region
        ),
    ]
}

fn file_name(relative: &str) -> &str {
    relative.rsplit('/').next().unwrap_or(relative)
}

/// 把「远端可达但产物没有」这件事写成一句能直接照着做的话。
pub(crate) fn unexecuted_reason(
    platform_name: &str,
    platform: &PlatformConfig,
    missing: &[PathBuf],
) -> String {
    if !platform.enabled {
        return format!(
            "{platform_name} 的 Device Farm 链路尚未打通：{}",
            platform.blocked_reason
        );
    }
    if missing.is_empty() {
        return format!(
            "{platform_name} 产物齐备；执行 `{}` 后由设备回传测量值定 verdict",
            platform.build_command
        );
    }
    let names = missing
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("、");
    format!(
        "{platform_name} 远端真机池可达，但缺产物 {names}；先跑 `{}` 产出后再调度",
        platform.build_command
    )
}

/// 远端链路的完整状态，供报告与终端同时消费。
pub(crate) struct RemoteStatus {
    pub(crate) probe: RemoteProbe,
    pub(crate) reason: String,
    pub(crate) plan: Vec<String>,
}

pub(crate) fn status(root: &Path, config: &Config, platform: Platform) -> Option<RemoteStatus> {
    let entry = config.platform(platform)?;
    let missing = missing_artifacts(root, entry);
    Some(RemoteStatus {
        probe: probe(config, entry),
        reason: unexecuted_reason(platform.as_str(), entry, &missing),
        plan: schedule_plan(config, entry),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema_version = 1
region = "us-west-2"
project_arn = "arn:aws:devicefarm:us-west-2:891377171033:project:9b17cc74"
billing_method = "METERED"
locale = "zh_CN"
job_timeout_minutes = 30

[android]
enabled = true
device_pool_arn = "arn:aws:devicefarm:us-west-2:891377171033:devicepool:9b17cc74/7c385981"
app_artifact = "target/mobile/yunjian-spike.apk"
app_upload_type = "ANDROID_APP"
test_package_artifact = "target/mobile/spike-tests.zip"
test_package_upload_type = "APPIUM_NODE_TEST_PACKAGE"
test_spec = ".aws/devicefarm/spike-android.yml"
test_type = "APPIUM_NODE"
build_command = "aws codebuild start-build --project-name yunjian-android-spike"
blocked_reason = ""

[ios]
enabled = false
device_pool_arn = "arn:aws:devicefarm:us-west-2:891377171033:devicepool:9b17cc74/deadbeef"
app_artifact = "target/mobile/yunjian-spike.ipa"
app_upload_type = "IOS_APP"
test_package_artifact = "target/mobile/spike-xcuitest.zip"
test_package_upload_type = "XCTEST_UI_TEST_PACKAGE"
test_spec = ".aws/devicefarm/spike-ios.yml"
test_type = "XCTEST_UI"
build_command = "aws codebuild start-build --project-name yunjian-ios-spike"
blocked_reason = "构建 .ipa 需 macOS 与 Xcode"
"#;

    #[test]
    fn parses_a_complete_configuration() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        assert_eq!(config.region, ONLY_REGION);
        assert_eq!(config.locale, "zh_CN");
        assert!(config.android.enabled);
        assert!(!config.ios.enabled);
        assert_eq!(config.android.test_type, "APPIUM_NODE");
    }

    #[test]
    fn rejects_a_region_other_than_us_west_2() {
        let text = SAMPLE.replace("us-west-2\"\nproject", "us-east-2\"\nproject");
        let error = parse(&text).expect_err("非 us-west-2 必须被拒绝");
        assert!(
            error.to_string().contains("只在 us-west-2"),
            "错误信息应点明区域限制，实际 {error}"
        );
    }

    #[test]
    fn rejects_an_unknown_schema_version() {
        let text = SAMPLE.replace("schema_version = 1", "schema_version = 2");
        let error = parse(&text).expect_err("未知 schema_version 必须被拒绝");
        assert!(error.to_string().contains("schema_version"));
    }

    #[test]
    fn platform_lookup_covers_only_mobile_platforms() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        assert!(config.platform(Platform::Android).is_some());
        assert!(config.platform(Platform::Ios).is_some());
        assert!(config.platform(Platform::Linux).is_none());
        assert!(config.platform(Platform::MacOs).is_none());
        assert!(config.platform(Platform::Windows).is_none());
    }

    #[test]
    fn missing_artifacts_names_every_absent_input() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        let root = Path::new("/nonexistent-root-for-yunjian-device-farm-test");
        let missing = missing_artifacts(root, &config.android);
        assert_eq!(
            missing.len(),
            3,
            "三个产物都不存在时应当全部点名：{missing:?}"
        );
        assert!(
            missing
                .iter()
                .any(|path| path.ends_with("yunjian-spike.apk"))
        );
        assert!(missing.iter().any(|path| path.ends_with("spike-tests.zip")));
        assert!(
            missing
                .iter()
                .any(|path| path.ends_with("spike-android.yml"))
        );
    }

    #[test]
    fn unexecuted_reason_names_the_build_command_when_artifacts_are_missing() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        let missing = vec![PathBuf::from("target/mobile/yunjian-spike.apk")];
        let reason = unexecuted_reason("Android", &config.android, &missing);
        assert!(
            reason.contains("yunjian-spike.apk"),
            "应点名缺失产物：{reason}"
        );
        assert!(
            reason.contains("yunjian-android-spike"),
            "应给出确切构建命令：{reason}"
        );
    }

    #[test]
    fn unexecuted_reason_reports_a_disabled_platform_as_blocked_not_as_missing_artifacts() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        let reason = unexecuted_reason("iOS", &config.ios, &[]);
        assert!(
            reason.contains("macOS"),
            "未打通的平台应报告阻塞原因而不是产物缺失：{reason}"
        );
    }

    #[test]
    fn schedule_plan_renders_every_upload_and_the_run_itself() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        let plan = schedule_plan(&config, &config.android);
        assert_eq!(
            plan.len(),
            4,
            "三次 create-upload 加一次 schedule-run：{plan:?}"
        );
        assert!(plan[0].contains("--type ANDROID_APP"));
        assert!(plan[1].contains("--type APPIUM_NODE_TEST_PACKAGE"));
        assert!(plan[2].contains("--type APPIUM_NODE_TEST_SPEC"));
        assert!(plan[3].contains("schedule-run"));
        assert!(
            plan[3].contains("\"locale\":\"zh_CN\""),
            "中文 IME 判据要求把设备 locale 覆盖成 zh_CN：{}",
            plan[3]
        );
        assert!(plan[3].contains("\"jobTimeoutMinutes\":30"));
        for step in &plan {
            assert!(
                step.contains("--region us-west-2"),
                "每一步都要钉住区域：{step}"
            );
        }
    }

    #[test]
    fn the_shipped_configuration_targets_us_west_2_and_leaves_ios_blocked() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask 应当有父目录");
        let config = load(root)
            .expect("随仓库的 Device Farm 配置应当可解析")
            .expect("mobile/device-farm.toml 应当存在");
        assert_eq!(config.region, ONLY_REGION);
        assert!(
            config.android.device_pool_arn.contains(":devicepool:"),
            "Android 设备池必须是真实的 devicepool ARN：{}",
            config.android.device_pool_arn
        );
        assert!(
            !config.ios.enabled && !config.ios.blocked_reason.trim().is_empty(),
            "iOS 链路未打通时必须写明阻塞原因，不得留空冒充可执行"
        );
    }

    #[test]
    fn probe_command_pins_the_region_and_the_pool() {
        let config = parse(SAMPLE).expect("样例配置应当可解析");
        let probe = probe(&config, &config.android);
        assert!(
            probe.command.contains("--region us-west-2"),
            "探测命令必须钉住区域：{}",
            probe.command
        );
        assert!(
            probe.command.contains("get-device-pool"),
            "探测必须证明设备池存在而不仅是凭据可用：{}",
            probe.command
        );
    }
}
