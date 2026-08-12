//! 云笺语音 crate。
//!
//! 朗读节奏来自逐音步合成加 Rust 侧静音拼接，因而自带时间戳，无需强制
//! 对齐。采集走 `rodio`（内部是 `cpal`），不经由 WebView。
//!
//! 三个特性开关，边界刻意不同：
//!
//! - **无特性**：纯 Rust，无原生依赖。[`permission`] 与 [`platform`] 始终可用，
//!   因为「语音为什么不可用」这个问题在不带语音的构建里同样要回答。
//! - **`capture`**：拉入 `rodio`，提供 [`capture`]。许可为 MIT/Apache-2.0。
//! - **`voice`**：拉入 `sherpa-rs`，提供 [`asr`] 与 [`tts`]，并隐含 `capture`。
//!   **分发物因静态包含 espeak-ng 而须按 GPL-3.0 提供**，详见 `docs/VOICE-BUILD.zh.md`。
//!
//! 采集在 Rust 里做绕开了 WebView，但**没有**绕开操作系统的麦克风授权。各平台的
//! 授权链与系统最低版本见 [`permission`]、[`platform`] 与
//! `docs/PLATFORM-REQUIREMENTS.zh.md`。

mod error;
pub mod permission;
pub mod platform;

pub use error::VoiceError;

#[cfg(feature = "capture")]
pub mod capture;

/// 音频增强。纯信号处理，不依赖 `voice` 特性，因此默认构建也能编译与自测。
pub mod augment;

/// 音频采集与播放。**判定层不带特性开关**——重采样正确性与四种失败各自降级成什么都在
/// 这一层，于是一台没有麦克风的机器仍然能验证它们；只有真实设备那一小块要 `capture`。
pub mod audio;

/// 模型的按需下载、许可门禁与本地缓存。**判定层不带特性开关**——许可与拒绝名单的判定、
/// 摘要校验、原子落地、缺失时的降级信号都在这一层，只有真正的 HTTP 与解压要 `download`。
pub mod models;

/// 破读词表、词谱句式表与朗读覆盖名册。**判定层不带特性开关**——三份 TSV 由 `include_str!`
/// 编译期内联，于是「数据站不站得住」在默认构建里就能回答，不需要模型也不需要语料库。
pub mod lexicon;

/// 朗读节奏：音步切分、静音拼接与逐音步时间戳。**判定层不带特性开关**——切分规则、拼接
/// 算术与静音判定都是纯函数，合成抽象在 [`prosody::FootSynthesizer`] 之后，于是一台没有
/// 模型、没有声卡的机器仍然能验证节奏是对的。
pub mod prosody;

/// 流式识别的双路解码、卡顿判定与事件驱动。**判定层不带特性开关**——类型隔离、
/// 卡顿时序、识别器配置取值与降级策略都是纯逻辑，真实推理藏在
/// [`recognize::DualDecode`] 之后，于是一台没有模型、没有麦克风的机器仍然能验证
/// 「偏置输出进不了评分」「四字后停顿恰好提示一次」这两件最要紧的事。
pub mod recognize;

#[cfg(feature = "voice")]
pub mod asr;
#[cfg(feature = "voice")]
pub mod tts;

/// 本二进制是否编译进了原生语音能力。调用方据此决定降级到默写练习，
/// 而不是在缺失时抛错。
#[must_use]
pub const fn is_available() -> bool {
    cfg!(feature = "voice")
}

/// 链接进来的 sherpa-onnx 原生库版本；未编译语音时为 `None`。
///
/// 这是唯一一个不需要模型就能触达原生库的调用，因此它同时是诊断入口与链接性证据：
/// 只要它返回 `Some`，libsherpa-onnx 与 onnxruntime 就确实被加载了。
#[must_use]
pub fn backend_version() -> Option<String> {
    #[cfg(feature = "voice")]
    {
        // SAFETY: `SherpaOnnxGetVersionStr` 返回指向库内静态字符串的指针，
        // 不需要调用方释放，且在进程生命周期内始终有效。
        let raw = unsafe { sherpa_rs::sherpa_rs_sys::SherpaOnnxGetVersionStr() };
        if raw.is_null() {
            return None;
        }
        // SAFETY: 上一步已排除空指针，且该指针指向 NUL 结尾的 C 字符串。
        let text = unsafe { std::ffi::CStr::from_ptr(raw) };
        Some(text.to_string_lossy().into_owned())
    }
    #[cfg(not(feature = "voice"))]
    {
        None
    }
}

/// 当前该走语音练习还是打字练习。**这是调用方唯一需要的入口。**
///
/// 组合两级判定：先看本二进制有没有编译语音能力（没有的话权限状态无意义），再看权限。
/// 两级分开是因为 [`permission::decide`] 必须能在不开 `voice` 的构建里被测到——
/// 见那个函数的文档。
#[must_use]
pub fn practice(report: &permission::MicPermission) -> permission::Practice {
    if is_available() {
        return permission::decide(report);
    }
    permission::degrade(
        permission::DegradeReason::FeatureDisabled,
        Some(report.platform),
    )
}

/// 采样点均方根。低于阈值即视为静音，是冒烟测试判定「真的产生了声音」的依据：
/// 长度对得上但全是零的缓冲区在别的断言下都能过。
#[must_use]
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "音频电平只需 f32 精度，调用方用它和阈值比较"
    )]
    let level = (sum / samples.len() as f64).sqrt() as f32;
    level
}

#[cfg(test)]
mod tests {
    use super::{is_available, rms};

    #[test]
    fn rms_of_silence_is_zero() {
        assert!(rms(&[0.0; 128]) < f32::EPSILON);
        assert!(rms(&[]) < f32::EPSILON);
    }

    #[test]
    fn rms_of_full_scale_square_wave_is_one() {
        let wave: Vec<f32> = (0..128)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert!((rms(&wave) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn availability_tracks_the_feature_flag() {
        assert_eq!(is_available(), cfg!(feature = "voice"));
    }
}
