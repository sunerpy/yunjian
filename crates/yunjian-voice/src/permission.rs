//! 麦克风权限，以及权限拿不到时的降级判定。
//!
//! **在 Rust 里采集绕开了 WebView，但没有绕开操作系统。** 每个平台都要一次授权，
//! 而各平台把「授权」放在完全不同的层：
//!
//! | 平台 | 授权点 | 谁能发起 |
//! |---|---|---|
//! | Linux | 无系统级麦克风门（ALSA/PulseAudio 直接开设备） | 进程自己 |
//! | Windows | 「设置 → 隐私 → 麦克风」的应用开关，首次 WASAPI 采集触发 | 进程自己 |
//! | macOS | TCC 弹窗，由**已签名**进程首次触达输入设备触发 | 进程自己 |
//! | Android | 运行时权限对话框 | **只有 Android framework**，`cpal`/AAudio 做不到 |
//! | iOS | `AVAudioSession` 激活 + 录音授权 | 需要先激活会话，`cpal` 不做这件事 |
//!
//! 因此本模块只定义**契约与判定**：状态取值、必须申请的权限名、拿不到时降级到什么。
//! 实际发起授权的代码在 `mobile/` 下的 Kotlin 与 Swift 里，由外壳 crate 接线。

use crate::platform::Platform;

/// Android 必须在 manifest 里声明、并在运行时申请的权限。
///
/// **两个，不是一个。** 只声明 `RECORD_AUDIO` 时 WebView 侧的 `getUserMedia` 会失败
/// （`tauri#10846`），原生采集在部分机型上同样拿不到可用的输入路由，因为音频路由与
/// 模式切换归 `MODIFY_AUDIO_SETTINGS` 管。
pub const ANDROID_PERMISSIONS: [&str; 2] = [
    "android.permission.RECORD_AUDIO",
    "android.permission.MODIFY_AUDIO_SETTINGS",
];

/// macOS 签名产物必须携带的两个麦克风 entitlement 键。
///
/// **两个，不是一个。** `device.microphone` 是沙箱内的麦克风访问，
/// `device.audio-input` 是 hardened runtime 下的音频输入豁免；缺后者时**只有公证后的
/// 生产构建**会失败（`tauri#8314`），`tauri dev` 与本地未签名构建全绿。
pub const MACOS_ENTITLEMENTS: [&str; 2] = [
    "com.apple.security.device.microphone",
    "com.apple.security.device.audio-input",
];

/// macOS 与 iOS 的 `Info.plist` 必须携带的用途说明键。缺它时进程在触达输入设备的瞬间
/// 被系统直接终止，不是返回错误。
pub const MICROPHONE_USAGE_DESCRIPTION_KEY: &str = "NSMicrophoneUsageDescription";

/// 授权状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    /// 已授权，可以采集。
    Granted,
    /// 用户明确拒绝。再次申请不会弹窗，只能引导去系统设置。
    Denied,
    /// 尚未询问过。可以发起一次申请。
    Undetermined,
    /// 被设备管理策略或家长控制禁用，用户自己改不了。
    Restricted,
}

/// 谁有能力发起这个平台的授权。记录它是为了让「Rust 侧调用一下就行」这个错误认识
/// 无法在代码里成立。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// 没有系统级门禁，打开设备即可。
    OpenDevice,
    /// 进程首次触达输入设备时由系统弹窗，Rust 侧无需额外调用。
    SystemPromptOnFirstUse,
    /// 只能由 Android framework 发起，`cpal`/AAudio 无能为力。
    AndroidFramework,
    /// 必须先激活 `AVAudioSession` 并申请录音授权，`cpal` 不做这件事。
    AudioSessionActivation,
}

impl Gate {
    /// 本平台的授权是否**必须**经由原生外壳代码发起。
    #[must_use]
    pub const fn needs_shell_code(self) -> bool {
        matches!(self, Self::AndroidFramework | Self::AudioSessionActivation)
    }
}

/// 一次权限查询的结果。字段刻意冗长：调用方需要向用户解释「为什么不能录」，
/// 一个裸 bool 做不到。
#[derive(Debug, Clone)]
pub struct MicPermission {
    /// 平台。
    pub platform: Platform,
    /// 状态。
    pub state: PermissionState,
    /// 授权点。
    pub gate: Gate,
    /// 该平台需要的权限或 entitlement 名，用于诊断与文档。
    pub required: Vec<String>,
    /// 面向开发者的补充说明，可为空。
    pub detail: String,
}

impl MicPermission {
    /// 本平台在编译期已知的授权点与必需权限。状态由调用方（外壳）填。
    #[must_use]
    pub fn contract(platform: Platform) -> (Gate, Vec<String>) {
        match platform {
            Platform::Linux => (Gate::OpenDevice, Vec::new()),
            Platform::Windows => (Gate::SystemPromptOnFirstUse, Vec::new()),
            Platform::MacOs => (
                Gate::SystemPromptOnFirstUse,
                MACOS_ENTITLEMENTS
                    .iter()
                    .map(ToString::to_string)
                    .chain(std::iter::once(MICROPHONE_USAGE_DESCRIPTION_KEY.to_owned()))
                    .collect(),
            ),
            Platform::Android => (
                Gate::AndroidFramework,
                ANDROID_PERMISSIONS
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            Platform::Ios => (
                Gate::AudioSessionActivation,
                vec![MICROPHONE_USAGE_DESCRIPTION_KEY.to_owned()],
            ),
        }
    }

    /// 构造一个报告，`gate` 与 `required` 从 [`Self::contract`] 取，防止调用方写错。
    #[must_use]
    pub fn new(platform: Platform, state: PermissionState, detail: impl Into<String>) -> Self {
        let (gate, required) = Self::contract(platform);
        Self {
            platform,
            state,
            gate,
            required,
            detail: detail.into(),
        }
    }
}

/// 语音不可用的原因。每一项都对应一句给用户看的解释。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradeReason {
    /// 本二进制没编译语音能力。
    FeatureDisabled,
    /// 系统版本低于底线。
    SystemTooOld,
    /// 用户拒绝了麦克风授权。
    PermissionDenied,
    /// 由管理策略禁用。
    PermissionRestricted,
    /// 还没申请过授权。
    PermissionUndetermined,
    /// 没有可用的输入设备。
    NoInputDevice,
    /// 语音模型不可用：没下载、下载失败、校验不过，或许可不允许加载。
    ///
    /// 与采集类原因分开的理由同 [`Self::DeviceBusy`]：下一步动作不在同一个地方。
    /// 这一条要联网跑 `yunjian models fetch`，与麦克风、设备、系统版本都无关，
    /// 归到 [`Self::CaptureFailed`] 会把用户指去检查麦克风。
    ModelUnavailable,
    /// 设备存在但被其他程序独占。
    ///
    /// 与 [`Self::CaptureFailed`] 分开，因为下一步动作完全不同：这一条是「关掉占用它的
    /// 程序」，而 `CaptureFailed` 是「原因未知，换个设备试试」。把两者合并会让界面在
    /// 一半的情形下给出错误的引导。
    DeviceBusy,
    /// 采集成功但识别拒绝了这一次：识别器报错，或整段录音里一次都没检测到开口。
    ///
    /// 与 [`Self::CaptureFailed`] 分开是产品要求：录到了声音而识别不接受，下一步是
    /// 「再念一次、离麦克风近一点」；采集本身失败的下一步是「检查设备」。归成一条会
    /// 在用户明明说了话的那一半情形里把他指去查硬件。
    RecognitionRejected,
    /// 采集本身失败，且原因不在上面任何一条里。
    CaptureFailed,
}

/// 当前该走哪条练习路径。**永远有一条可走**——这就是本模块存在的意义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Practice {
    /// 语音练习可用。
    Voice,
    /// 降级到打字练习，并携带解释。
    Typed {
        /// 降级原因。
        reason: DegradeReason,
        /// 面向用户的中文解释，必须说清「为什么」与「怎么恢复」。
        message: String,
    },
}

impl Practice {
    /// 是否降级了。
    #[must_use]
    pub const fn is_typed(&self) -> bool {
        matches!(self, Self::Typed { .. })
    }

    /// 降级原因；未降级时为 `None`。
    #[must_use]
    pub const fn reason(&self) -> Option<DegradeReason> {
        match self {
            Self::Voice => None,
            Self::Typed { reason, .. } => Some(*reason),
        }
    }
}

/// 由降级原因生成用户可见的解释。
///
/// 每一句都要回答两件事：为什么不能录、怎么恢复。只说「麦克风不可用」等于把用户
/// 留在原地。
#[must_use]
pub fn explain(reason: DegradeReason, platform: Option<Platform>) -> String {
    let where_to_go = match platform {
        Some(Platform::MacOs) => "「系统设置 → 隐私与安全性 → 麦克风」",
        Some(Platform::Windows) => "「设置 → 隐私和安全性 → 麦克风」",
        Some(Platform::Android) => "「设置 → 应用 → 云笺 → 权限」",
        Some(Platform::Ios) => "「设置 → 云笺 → 麦克风」",
        Some(Platform::Linux) | None => "系统的声音设置",
    };
    match reason {
        DegradeReason::FeatureDisabled => {
            "本版本未编译语音能力，已切换到打字练习。打字练习的评分与语音练习共用同一个内核，功能完整。".to_owned()
        }
        DegradeReason::SystemTooOld => {
            let floor = platform.map_or_else(
                || "所需版本".to_owned(),
                |p| crate::platform::floor_of(p).minimum.to_owned(),
            );
            format!(
                "当前系统版本低于语音功能所需的 {floor}，已切换到打字练习。升级系统即可启用朗读与语音默写，在此之前其余功能不受影响。"
            )
        }
        DegradeReason::PermissionDenied => format!(
            "麦克风授权被拒绝，已切换到打字练习。想用语音默写请到{where_to_go}允许云笺访问麦克风，然后重新进入练习。"
        ),
        DegradeReason::PermissionRestricted => format!(
            "麦克风被设备管理策略禁用，应用无法自行申请，已切换到打字练习。需要管理员在{where_to_go}放开后才能使用语音默写。"
        ),
        DegradeReason::PermissionUndetermined => {
            "还没有获得麦克风授权，已先进入打字练习。点「开始语音默写」时会弹出授权请求，同意后即可切换。".to_owned()
        }
        DegradeReason::NoInputDevice => format!(
            "没有检测到可用的麦克风，已切换到打字练习。接入麦克风或在{where_to_go}选择输入设备后可重试。"
        ),
        DegradeReason::DeviceBusy => format!(
            "麦克风正被其他程序占用，已切换到打字练习。关闭正在录音或通话的程序后重试，也可以在{where_to_go}换一个输入设备。"
        ),
        DegradeReason::RecognitionRejected => {
            "这一次录音没有被识别接受，已切换到打字练习。已完成的部分进度保留着；想继续语音跟读，请回到刚才那一句重念一次，离麦克风近一些、说得响一点。".to_owned()
        }
        DegradeReason::CaptureFailed => format!(
            "麦克风打开失败，已切换到打字练习。可能是设备被其他程序独占；关闭占用程序或在{where_to_go}换一个输入设备后重试。"
        ),
        DegradeReason::ModelUnavailable => {
            "语音模型尚未就绪，已切换到打字练习。联网后运行 `yunjian models fetch <模型名>` 下载并校验即可启用；`yunjian models list` 能看到每个模型的许可与缓存状态。".to_owned()
        }
    }
}

/// 只按权限状态判定练习路径，**不看是否编译了语音能力**。
///
/// 两件事刻意分开：特性开关的判定属于 [`crate::practice`]。合在一起会让权限逻辑只能在
/// 开了 `voice` 的构建里被测到，而 `voice` 拉进的是几十 MB 原生库和 GPL-3.0 的
/// espeak-ng——于是 CI 上跑得最多的那种构建反而验证不了降级链。
#[must_use]
pub fn decide(permission: &MicPermission) -> Practice {
    let reason = match permission.state {
        PermissionState::Granted => return Practice::Voice,
        PermissionState::Denied => DegradeReason::PermissionDenied,
        PermissionState::Restricted => DegradeReason::PermissionRestricted,
        PermissionState::Undetermined => DegradeReason::PermissionUndetermined,
    };
    Practice::Typed {
        reason,
        message: explain(reason, Some(permission.platform)),
    }
}

/// 直接由一个降级原因构造降级结果，供采集失败等非权限路径使用。
#[must_use]
pub fn degrade(reason: DegradeReason, platform: Option<Platform>) -> Practice {
    Practice::Typed {
        reason,
        message: explain(reason, platform),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ANDROID_PERMISSIONS, DegradeReason, Gate, MACOS_ENTITLEMENTS,
        MICROPHONE_USAGE_DESCRIPTION_KEY, MicPermission, PermissionState, Practice, decide,
        degrade, explain,
    };
    use crate::platform::Platform;

    #[test]
    fn android_requires_both_permissions() {
        let (gate, required) = MicPermission::contract(Platform::Android);
        assert_eq!(gate, Gate::AndroidFramework);
        for name in ANDROID_PERMISSIONS {
            assert!(
                required.iter().any(|r| r == name),
                "Android 契约缺 {name}；只申请 RECORD_AUDIO 是 tauri#10846 那个坑"
            );
        }
        assert_eq!(required.len(), 2, "不多不少两个");
    }

    #[test]
    fn macos_requires_both_entitlements_and_the_usage_description() {
        let (_, required) = MicPermission::contract(Platform::MacOs);
        for key in MACOS_ENTITLEMENTS {
            assert!(required.iter().any(|r| r == key), "macOS 契约缺 {key}");
        }
        assert!(
            required
                .iter()
                .any(|r| r == MICROPHONE_USAGE_DESCRIPTION_KEY)
        );
    }

    #[test]
    fn mobile_gates_need_shell_code_desktop_gates_do_not() {
        assert!(
            MicPermission::contract(Platform::Android)
                .0
                .needs_shell_code()
        );
        assert!(MicPermission::contract(Platform::Ios).0.needs_shell_code());
        assert!(
            !MicPermission::contract(Platform::Linux)
                .0
                .needs_shell_code()
        );
        assert!(
            !MicPermission::contract(Platform::MacOs)
                .0
                .needs_shell_code()
        );
        assert!(
            !MicPermission::contract(Platform::Windows)
                .0
                .needs_shell_code()
        );
    }

    #[test]
    fn denied_permission_degrades_to_typed_practice_with_an_explanation() {
        for platform in Platform::ALL {
            let report = MicPermission::new(platform, PermissionState::Denied, "");
            let practice = decide(&report);
            assert!(practice.is_typed(), "被拒必须降级，不能崩也不能静默");
            assert_eq!(practice.reason(), Some(DegradeReason::PermissionDenied));
            let Practice::Typed { message, .. } = practice else {
                unreachable!()
            };
            assert!(
                message.contains("打字练习"),
                "解释要告诉用户还能做什么：{message}"
            );
        }
    }

    #[test]
    fn restricted_and_undetermined_also_degrade_but_with_distinct_reasons() {
        let restricted = decide(&MicPermission::new(
            Platform::Android,
            PermissionState::Restricted,
            "",
        ));
        let undetermined = decide(&MicPermission::new(
            Platform::Android,
            PermissionState::Undetermined,
            "",
        ));
        assert_eq!(
            restricted.reason(),
            Some(DegradeReason::PermissionRestricted)
        );
        assert_eq!(
            undetermined.reason(),
            Some(DegradeReason::PermissionUndetermined)
        );
        assert_ne!(
            restricted, undetermined,
            "「管理策略禁用」与「还没问过」的引导完全不同，不能合并成一条"
        );
    }

    #[test]
    fn granted_permission_allows_voice_regardless_of_the_feature_flag() {
        let report = MicPermission::new(Platform::Linux, PermissionState::Granted, "");
        assert_eq!(
            decide(&report),
            Practice::Voice,
            "decide 只回答权限问题；特性开关由 crate::practice 判"
        );
    }

    #[test]
    fn the_crate_entry_point_short_circuits_on_the_feature_flag() {
        let report = MicPermission::new(Platform::Linux, PermissionState::Granted, "");
        let practice = crate::practice(&report);
        if crate::is_available() {
            assert_eq!(practice, Practice::Voice);
        } else {
            assert_eq!(
                practice.reason(),
                Some(DegradeReason::FeatureDisabled),
                "没编译语音能力时权限状态没有意义"
            );
        }
    }

    #[test]
    fn every_degrade_reason_has_a_message_naming_both_cause_and_recovery() {
        let reasons = [
            DegradeReason::FeatureDisabled,
            DegradeReason::SystemTooOld,
            DegradeReason::PermissionDenied,
            DegradeReason::PermissionRestricted,
            DegradeReason::PermissionUndetermined,
            DegradeReason::NoInputDevice,
            DegradeReason::DeviceBusy,
            DegradeReason::CaptureFailed,
        ];
        for reason in reasons {
            for platform in Platform::ALL {
                let text = explain(reason, Some(platform));
                assert!(
                    text.len() > 20,
                    "{reason:?} 在 {platform:?} 上解释过短：{text}"
                );
                assert!(
                    text.contains("打字练习"),
                    "{reason:?} 必须告知降级去向：{text}"
                );
            }
            assert!(
                !explain(reason, None).is_empty(),
                "{reason:?} 在未知平台上也要有解释"
            );
        }
    }

    #[test]
    fn system_too_old_message_quotes_the_platform_floor() {
        let text = explain(DegradeReason::SystemTooOld, Some(Platform::Android));
        assert!(text.contains("26"), "Android 的解释要给出 API 26：{text}");
        let text = explain(DegradeReason::SystemTooOld, Some(Platform::MacOs));
        assert!(text.contains("14.2"), "macOS 的解释要给出 14.2：{text}");
    }

    #[test]
    fn device_busy_and_capture_failed_give_different_instructions() {
        let busy = explain(DegradeReason::DeviceBusy, Some(Platform::Linux));
        let failed = explain(DegradeReason::CaptureFailed, Some(Platform::Linux));
        assert_ne!(
            busy, failed,
            "「被占用」与「原因未知」的下一步动作不同，合并成一条会让界面给出错误引导"
        );
        assert!(
            busy.contains("占用"),
            "被占用的解释必须点名占用这件事：{busy}"
        );
    }

    #[test]
    fn capture_failure_degrades_without_a_permission_report() {
        let practice = degrade(DegradeReason::CaptureFailed, Some(Platform::Linux));
        assert!(practice.is_typed());
        assert_eq!(practice.reason(), Some(DegradeReason::CaptureFailed));
    }
}
