//! 平台配置测试。断言 `mobile/` 下那些**在本机跑不起来**的配置文件仍然是对的。
//!
//! 这类断言看着琐碎，实际是本 spike 唯一能在 Linux 上守住的部分：macOS 的
//! entitlements 缺失**只在已签名并公证的构建里**才失败（`tauri#8314`），Android 的
//! 权限缺失只在真机上才失败。两者都不会在开发期报错，所以只能由配置层的静态断言
//! 加 CI 上的 `codesign -d` 一起兜。

use std::path::{Path, PathBuf};

const MOBILE: &str = "mobile";
const ENTITLEMENT_KEYS: [&str; 2] = [
    "com.apple.security.device.microphone",
    "com.apple.security.device.audio-input",
];
const ANDROID_PERMISSIONS: [&str; 2] = [
    "android.permission.RECORD_AUDIO",
    "android.permission.MODIFY_AUDIO_SETTINGS",
];

fn mobile_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(MOBILE)
}

fn read(relative: &str) -> String {
    let path = mobile_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("读不到 {}：{err}", path.display()))
}

fn tauri_config() -> serde_json::Value {
    serde_json::from_str(&read("tauri.audio.conf.json"))
        .expect("tauri.audio.conf.json 必须是合法 JSON")
}

#[test]
fn android_manifest_declares_both_audio_permissions() {
    let manifest = read("android/AndroidManifest.xml");
    for permission in ANDROID_PERMISSIONS {
        assert!(
            manifest.contains(&format!("android:name=\"{permission}\"")),
            "manifest 缺 {permission}；只声明 RECORD_AUDIO 是 tauri#10846 那个坑"
        );
    }
}

#[test]
fn android_manifest_permission_names_match_the_rust_contract() {
    let manifest = read("android/AndroidManifest.xml");
    for permission in yunjian_voice::permission::ANDROID_PERMISSIONS {
        assert!(
            manifest.contains(permission),
            "Rust 契约声明了 {permission}，manifest 里没有——两边必须逐字一致"
        );
    }
}

#[test]
fn android_min_sdk_is_at_least_26_in_both_gradle_and_tauri_config() {
    let gradle = read("android/build.gradle.kts");
    let declared = gradle
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("minSdk")
                .and_then(|rest| rest.trim().strip_prefix('='))
                .and_then(|rest| rest.trim().parse::<u32>().ok())
        })
        .expect("build.gradle.kts 必须显式声明 minSdk");
    assert!(
        declared >= yunjian_voice::platform::ANDROID_MIN_SDK,
        "cpal 的 AAudio 绑定要求 API {}，gradle 写的是 {declared}",
        yunjian_voice::platform::ANDROID_MIN_SDK
    );

    let config = tauri_config();
    let from_config = config["bundle"]["android"]["minSdkVersion"]
        .as_u64()
        .expect("bundle.android.minSdkVersion 必须显式写出，Tauri 默认的 24 太低");
    assert!(
        from_config >= u64::from(yunjian_voice::platform::ANDROID_MIN_SDK),
        "Tauri 配置里的 minSdkVersion 是 {from_config}，低于 cpal 要求的 {}",
        yunjian_voice::platform::ANDROID_MIN_SDK
    );
    assert_eq!(
        u64::from(declared),
        from_config,
        "gradle 与 Tauri 配置的 minSdk 必须一致，否则打包结果取决于哪一份先生效"
    );
}

/// 失败场景 1 的正向断言：`bundle.macOS.entitlements` 必须指向一个**存在的**、
/// **含两个麦克风键的**文件。把该字段删掉或指向别处，这条测试必须失败。
#[test]
fn macos_bundle_config_points_at_an_entitlements_file_with_both_microphone_keys() {
    let config = tauri_config();
    let declared = config["bundle"]["macOS"]["entitlements"]
        .as_str()
        .expect("bundle.macOS.entitlements 缺失；缺它时只有公证构建会失败，开发期全绿");

    let file_name = Path::new(declared)
        .file_name()
        .expect("entitlements 路径必须指向一个文件");
    let resolved = mobile_dir().join("macos").join(file_name);
    assert!(
        resolved.is_file(),
        "bundle.macOS.entitlements 指向 {declared}，但 {} 不存在",
        resolved.display()
    );

    let plist = std::fs::read_to_string(&resolved).expect("entitlements 可读");
    for key in ENTITLEMENT_KEYS {
        assert!(
            plist.contains(key),
            "entitlements 文件缺 {key}；两个键都要，缺 audio-input 时只有公证构建会失败"
        );
    }
    assert!(
        plist.contains("<true/>"),
        "entitlement 键必须取值 true，声明了键而值为 false 与没声明等价"
    );
}

#[test]
fn macos_and_ios_info_plists_carry_the_usage_description() {
    let key = yunjian_voice::permission::MICROPHONE_USAGE_DESCRIPTION_KEY;
    for platform in ["macos", "ios"] {
        let plist = read(&format!("{platform}/Info.plist"));
        assert!(
            plist.contains(key),
            "{platform}/Info.plist 缺 {key}；缺它时进程被系统直接终止，不是返回错误"
        );
        let description = plist
            .split("<string>")
            .nth(1)
            .and_then(|rest| rest.split("</string>").next())
            .unwrap_or_default();
        assert!(
            description.chars().count() >= 10,
            "{platform} 的用途说明太短，App Store 审核会驳回：{description}"
        );
    }

    let config = tauri_config();
    for (platform, key) in [("macOS", "infoPlist"), ("iOS", "infoPlist")] {
        assert!(
            config["bundle"][platform][key].is_string(),
            "bundle.{platform}.{key} 必须指向 Info.plist，否则用途说明进不了产物"
        );
    }
}

#[test]
fn macos_hardened_runtime_stays_on() {
    let config = tauri_config();
    assert_eq!(
        config["bundle"]["macOS"]["hardenedRuntime"].as_bool(),
        Some(true),
        "关掉 hardened runtime 能让 entitlements 缺失不再报错，但公证会失败——\
         那是把一个可测的失败换成一个不可测的失败"
    );
}

#[test]
fn declared_minimum_system_versions_match_the_platform_floors() {
    use yunjian_voice::platform::{Platform, floor_of};

    let config = tauri_config();
    let macos = config["bundle"]["macOS"]["minimumSystemVersion"]
        .as_str()
        .expect("必须显式声明 macOS 最低版本；Tauri 默认的 10.13 低于 cpal 要求");
    assert_eq!(
        macos,
        floor_of(Platform::MacOs).minimum,
        "Tauri 配置与底线表必须一致"
    );

    let ios = config["bundle"]["iOS"]["minimumSystemVersion"]
        .as_str()
        .expect("必须显式声明 iOS 最低版本");
    assert!(
        floor_of(Platform::Ios).minimum.contains(ios),
        "iOS 底线表写 {}，配置写 {ios}",
        floor_of(Platform::Ios).minimum
    );
}

#[test]
fn ios_plugin_activates_the_audio_session_before_capture() {
    let swift = read("ios/AudioPermissionPlugin.swift");
    for needle in ["setCategory", "setActive", "playAndRecord"] {
        assert!(
            swift.contains(needle),
            "iOS 插件缺 {needle}；实读 cpal 的 iOS 后端确认它从不激活会话，\
             未激活时 inputNumberOfChannels() 为 0，cpal 会报告没有输入设备"
        );
    }
    assert!(
        swift.contains("AVFoundation"),
        "AVAudioSession 在 AVFoundation 里"
    );
    let config = tauri_config();
    let frameworks = config["bundle"]["iOS"]["frameworks"]
        .as_array()
        .expect("bundle.iOS.frameworks 必须列出 AVFoundation");
    assert!(
        frameworks
            .iter()
            .any(|f| f.as_str() == Some("AVFoundation")),
        "AVFoundation 未进 bundle.iOS.frameworks，链接会失败"
    );
}

#[test]
fn android_plugin_reports_both_permissions_separately() {
    let kotlin = read("android/AudioPermissionPlugin.kt");
    for needle in ["RECORD_AUDIO", "MODIFY_AUDIO_SETTINGS"] {
        assert!(kotlin.contains(needle), "Kotlin 插件未处理 {needle}");
    }
    assert!(
        kotlin.contains("requestPermissionForAlias"),
        "必须经 Android framework 发起授权；cpal/AAudio 做不到这件事"
    );
    assert!(
        kotlin.contains("shouldShowRequestPermissionRationale"),
        "必须区分「还没问过」与「勾了不再询问」——后者再申请不弹窗，只能引导去系统设置"
    );
}

/// 文档是产物之一，不是附注：五个平台各要一个**具体数字**。
#[test]
fn platform_requirements_document_states_a_concrete_minimum_for_all_five_platforms() {
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PLATFORM-REQUIREMENTS.zh.md"),
    )
    .expect("docs/PLATFORM-REQUIREMENTS.zh.md 必须存在");

    for floor in yunjian_voice::platform::FLOORS {
        assert!(
            doc.contains(floor.platform.label()),
            "文档缺 {} 一节",
            floor.platform.label()
        );
        assert!(
            doc.contains(floor.minimum),
            "文档没写 {} 的具体最低版本 {}",
            floor.platform.label(),
            floor.minimum
        );
    }
    for key in ENTITLEMENT_KEYS {
        assert!(doc.contains(key), "文档应列出 entitlement 键 {key}");
    }
    for permission in ANDROID_PERMISSIONS {
        assert!(
            doc.contains(permission),
            "文档应列出 Android 权限 {permission}"
        );
    }
}
