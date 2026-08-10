//! 云笺语音 crate。
//!
//! 朗读节奏来自逐音步合成加 Rust 侧静音拼接，因而自带时间戳，无需强制
//! 对齐。采集走 `rodio`/`cpal`，不经由 WebView。
//!
//! 整个原生推理能力挂在 `voice` cargo 特性后面。不开该特性时本 crate 仍可编译，
//! 且**不链接 onnxruntime**，词典与默写功能因此不会被原生依赖拖垮。
//! 构建方式、按平台前置条件与许可清单见 `docs/VOICE-BUILD.zh.md`。

mod error;

pub use error::VoiceError;

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
