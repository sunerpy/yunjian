//! 五平台的系统最低版本，以及每条底线的来源。
//!
//! 这张表是**产品要求**，不是建议：低于底线的系统拿不到语音，但必须仍然拿到完整的
//! 打字练习产品。因此每一条都带 [`Floor::reason`]，让「为什么是这个数字」可被审查，
//! 而不是靠记忆。`docs/PLATFORM-REQUIREMENTS.zh.md` 由测试钉住与本表一致。

/// 云笺需要区分的五个平台。桌面三个、移动两个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Platform {
    /// Linux 桌面（ALSA 后端）。
    Linux,
    /// Windows 桌面（WASAPI 后端）。
    Windows,
    /// macOS 桌面（CoreAudio 后端）。
    MacOs,
    /// Android（AAudio 后端）。
    Android,
    /// iOS（CoreAudio + `AVAudioSession`）。
    Ios,
}

impl Platform {
    /// 全部五个平台，顺序与文档表格一致。
    pub const ALL: [Self; 5] = [
        Self::Linux,
        Self::Windows,
        Self::MacOs,
        Self::Android,
        Self::Ios,
    ];

    /// 文档与报错里使用的中文名。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Android => "Android",
            Self::Ios => "iOS",
        }
    }

    /// 当前编译目标对应的平台；不在这五个之内时为 `None`。
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "android") {
            Some(Self::Android)
        } else if cfg!(target_os = "ios") {
            Some(Self::Ios)
        } else {
            None
        }
    }
}

/// 一个平台的语音功能系统底线。
#[derive(Debug, Clone, Copy)]
pub struct Floor {
    /// 平台。
    pub platform: Platform,
    /// 面向用户的最低版本字符串，与文档表格逐字一致。
    pub minimum: &'static str,
    /// 底线的来源。必须是可核对的事实，不是「保险起见」。
    pub reason: &'static str,
}

/// Android 的 `minSdkVersion`。**26 而不是 Tauri 默认的 24**：`cpal` 的 Android 后端
/// 通过 `ndk` crate 的 `api-level-26` 特性绑定 AAudio，两个候选 `cpal` 版本
/// （`rodio` 内部的 0.17.3 与曾被固定的 0.18.1）都是这一条，无从回避。
pub const ANDROID_MIN_SDK: u32 = 26;

/// macOS 的最低版本。见 [`FLOORS`] 里该条的 `reason`。
pub const MACOS_MINIMUM: &str = "14.2";

/// 五平台底线表。任何一条改动都必须同时改 `docs/PLATFORM-REQUIREMENTS.zh.md`，
/// 否则 `platform_floors_match_the_document` 会失败。
pub const FLOORS: [Floor; 5] = [
    Floor {
        platform: Platform::Linux,
        minimum: "glibc 2.31",
        reason: "ALSA 后端需要 libasound2；2.31 是 Ubuntu 20.04 的 glibc，也是发布矩阵里最旧的构建宿主",
    },
    Floor {
        platform: Platform::Windows,
        minimum: "Windows 10 1809",
        reason: "WASAPI 采集本身在 Vista 就有，但 Tauri v2 要求 WebView2，而 WebView2 的常青分发不再支持 1809 之前的版本",
    },
    Floor {
        platform: Platform::MacOs,
        minimum: MACOS_MINIMUM,
        reason: "cpal 的 macOS 后端在 loopback 路径无条件引用 macOS 14.2 才引入的 AudioHardwareCreateProcessTap 与 CATapDescription，且未做弱链接",
    },
    Floor {
        platform: Platform::Android,
        minimum: "8.0（API 26）",
        reason: "cpal 的 Android 后端经 ndk crate 的 api-level-26 特性绑定 AAudio，高于 Tauri 文档写的 API 24",
    },
    Floor {
        platform: Platform::Ios,
        minimum: "14.0",
        reason: "Tauri v2 的 iOS 部署目标默认 14.0；AVAudioSession 采集路径本身更早就有，因此底线由 Tauri 决定而非音频栈",
    },
];

/// 查表。
#[must_use]
pub fn floor_of(platform: Platform) -> Floor {
    let mut i = 0;
    while i < FLOORS.len() {
        if FLOORS[i].platform as u8 == platform as u8 {
            return FLOORS[i];
        }
        i += 1;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::{ANDROID_MIN_SDK, FLOORS, Platform, floor_of};

    #[test]
    fn every_platform_has_exactly_one_floor() {
        for platform in Platform::ALL {
            let matches = FLOORS
                .iter()
                .filter(|f| f.platform as u8 == platform as u8)
                .count();
            assert_eq!(matches, 1, "{} 应恰好一条底线", platform.label());
        }
    }

    #[test]
    fn every_floor_states_a_concrete_version_and_a_reason() {
        for floor in FLOORS {
            assert!(
                !floor.minimum.is_empty(),
                "{} 的最低版本不得为空",
                floor.platform.label()
            );
            assert!(
                floor.minimum.chars().any(char::is_numeric),
                "{} 的最低版本必须含具体数字，不能是「较新版本」这类空话：{}",
                floor.platform.label(),
                floor.minimum
            );
            assert!(
                floor.reason.len() > 20,
                "{} 的底线必须说明来源",
                floor.platform.label()
            );
        }
    }

    #[test]
    fn android_floor_is_26_not_tauri_default_24() {
        assert_eq!(
            ANDROID_MIN_SDK, 26,
            "cpal 的 AAudio 绑定要求 API 26；降到 Tauri 默认的 24 会在设备上崩"
        );
        assert!(floor_of(Platform::Android).minimum.contains("26"));
    }

    #[test]
    fn lookup_returns_the_requested_platform() {
        for platform in Platform::ALL {
            assert_eq!(floor_of(platform).platform, platform);
        }
    }
}
