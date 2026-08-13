//! 离线识别。
//!
//! 模型族固定为 Whisper：todo 45 逐模型核实许可后，只有 Whisper 权重能确认为 MIT
//! （`openai/whisper` 的 LICENSE 明写「code and model weights are released under the
//! MIT License」）。FunASR 系（SenseVoice / Paraformer）走的是自家的
//! *FunASR Model Open Source License v1.1*，不是 MIT 也不是 Apache-2.0；
//! 流式 zipformer 包在上游没有任何 LICENSE 声明。两者都因此出局，理由记在
//! `models/DENYLIST.md`。
//!
//! 目录布局不写死尺寸：`sherpa-onnx-whisper-tiny` 里叫 `tiny-encoder.onnx`，
//! `-small` 里叫 `small-encoder.onnx`。前缀由目录内容推断，因此换尺寸不必改代码。

use std::path::{Path, PathBuf};

use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};

/// 双路流式解码。判定层在 [`crate::recognize`]，这里只有直调 C API 的那一小块。
pub mod streaming;

use crate::VoiceError;

/// 权重精度。int8 量化版与 fp32 版在同一个发布包里并存，前者体积约为三分之一、
/// CPU 上快数倍，是桌面端按需下载的现实选择；CER 实测两者都要跑，所以精度是参数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Precision {
    /// `<前缀>-encoder.onnx`。
    #[default]
    Float32,
    /// `<前缀>-encoder.int8.onnx`。
    Int8,
}

impl Precision {
    fn suffix(self) -> &'static str {
        match self {
            Self::Float32 => ".onnx",
            Self::Int8 => ".int8.onnx",
        }
    }

    /// 报告与日志里的稳定标识。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Float32 => "fp32",
            Self::Int8 => "int8",
        }
    }
}

/// 识别器构造参数。
///
/// `language` 必填而非默认 `"zh"`：Whisper 的 language 传错不会报错，只会安静地
/// 输出另一种语言的转写，而那种失败在 CER 数字里看起来只像「模型很差」。
#[derive(Debug, Clone)]
pub struct RecognizerOptions {
    /// Whisper 的语言代码，如 `"zh"` / `"en"`。
    pub language: String,
    pub precision: Precision,
    /// onnxruntime 线程数。多进程并行测量时应设为 1，避免线程互相抢核。
    pub num_threads: i32,
}

impl Default for RecognizerOptions {
    fn default() -> Self {
        Self {
            language: "en".to_owned(),
            precision: Precision::Float32,
            num_threads: 1,
        }
    }
}

pub struct Recognizer {
    inner: WhisperRecognizer,
}

impl Recognizer {
    /// 以默认参数（英文、fp32）构造。
    ///
    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失的模型文件；`Backend` 转述 sherpa-onnx 的构造失败。
    pub fn new(model_dir: &Path) -> Result<Self, VoiceError> {
        Self::open(model_dir, &RecognizerOptions::default())
    }

    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失的模型文件；`Backend` 转述 sherpa-onnx 的构造失败，
    /// 或目录里找不到唯一的 Whisper 编码器。
    pub fn open(model_dir: &Path, opts: &RecognizerOptions) -> Result<Self, VoiceError> {
        let prefix = whisper_prefix(model_dir)?;
        let suffix = opts.precision.suffix();
        let files = [
            format!("{prefix}-encoder{suffix}"),
            format!("{prefix}-decoder{suffix}"),
            format!("{prefix}-tokens.txt"),
        ];
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        VoiceError::require_files(model_dir, &refs)?;

        let joined = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
        let config = WhisperConfig {
            encoder: joined(&files[0]),
            decoder: joined(&files[1]),
            tokens: joined(&files[2]),
            language: opts.language.clone(),
            num_threads: Some(opts.num_threads),
            ..Default::default()
        };

        let inner =
            WhisperRecognizer::new(config).map_err(|e| VoiceError::Backend(e.to_string()))?;
        Ok(Self { inner })
    }

    /// # Errors
    ///
    /// `AudioRead` 表示 WAV 无法解码；`ModelMissing` 表示路径不存在。
    pub fn transcribe_wav(&mut self, wav: &Path) -> Result<String, VoiceError> {
        if !wav.is_file() {
            return Err(VoiceError::ModelMissing {
                path: wav.to_path_buf(),
            });
        }
        let path = wav.to_string_lossy();
        let (samples, sample_rate) =
            sherpa_rs::read_audio_file(&path).map_err(|e| VoiceError::AudioRead {
                path: wav.to_path_buf(),
                source: e.to_string().into(),
            })?;
        Ok(self.transcribe(&samples, sample_rate))
    }

    /// 直接识别内存里的单声道 `f32` 采样。
    ///
    /// CER 实测要对同一段音频做多种增强，落盘再读回只是无谓的 I/O；更要紧的是
    /// `sherpa_rs::read_audio_file` 硬断言 16 kHz，走文件就无法喂 8 kHz 重采样变体。
    pub fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> String {
        self.inner.transcribe(sample_rate, samples).text
    }
}

/// 从目录内容推断 Whisper 文件前缀（`tiny` / `base` / `small` / …）。
///
/// 只认 `*-encoder.onnx`：int8 变体叫 `*-encoder.int8.onnx`，不会重复命中，
/// 因此同时放着两种精度的官方发布包也能推出唯一前缀。
fn whisper_prefix(model_dir: &Path) -> Result<String, VoiceError> {
    if !model_dir.is_dir() {
        return Err(VoiceError::ModelMissing {
            path: model_dir.to_path_buf(),
        });
    }
    let mut found: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(model_dir).map_err(|e| VoiceError::AudioRead {
        path: model_dir.to_path_buf(),
        source: Box::new(e),
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(prefix) = name.strip_suffix("-encoder.onnx") {
            found.push(prefix.to_owned());
        }
    }
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(VoiceError::Backend(format!(
            "{} 里没有 `*-encoder.onnx`，不像 sherpa-onnx 的 Whisper 模型目录",
            model_dir.display()
        ))),
        _ => Err(VoiceError::Backend(format!(
            "{} 里有多个 `*-encoder.onnx`（{}），无法判定用哪个",
            model_dir.display(),
            found.join(" / ")
        ))),
    }
}

/// 模型根目录。[`crate::models::cache_root`] 的别名。
///
/// **刻意只是转发，不再自己解析一遍。** 按需下载会把权重放进
/// [`crate::models::model_dir`]，识别器必须从同一个地方找它们；两份各自算路径的实现
/// 迟早会漂移，而漂移的症状是「下载说成功了但识别器报模型缺失」。
#[must_use]
pub fn model_root() -> PathBuf {
    crate::models::cache_root()
}

#[cfg(test)]
mod tests {
    use super::{Precision, whisper_prefix};

    #[test]
    fn precision_suffix_distinguishes_int8_from_float() {
        assert_eq!(Precision::Float32.suffix(), ".onnx");
        assert_eq!(Precision::Int8.suffix(), ".int8.onnx");
        assert_eq!(Precision::default(), Precision::Float32);
    }

    /// 同一目录里既有 fp32 又有 int8 时仍须推出唯一前缀——官方发布包正是这样打包的，
    /// 早期实现用 `contains("-encoder")` 匹配就会在这里判定「多个编码器」而失败。
    #[test]
    fn prefix_detection_ignores_the_int8_variant() {
        let dir = std::env::temp_dir().join("yunjian-asr-prefix-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        for name in [
            "small-encoder.onnx",
            "small-encoder.int8.onnx",
            "small-decoder.onnx",
            "small-decoder.int8.onnx",
            "small-tokens.txt",
        ] {
            std::fs::write(dir.join(name), b"x").expect("可写入");
        }
        assert_eq!(whisper_prefix(&dir).expect("应推出前缀"), "small");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefix_detection_reports_a_non_whisper_directory() {
        let dir = std::env::temp_dir().join("yunjian-asr-prefix-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        let err = whisper_prefix(&dir).expect_err("应报错");
        let text = err.to_string();
        assert!(text.contains("encoder.onnx"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
