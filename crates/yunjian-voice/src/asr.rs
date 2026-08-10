//! 离线识别。冒烟阶段用 Whisper tiny（权重 MIT），只为证明构建与链接通路成立；
//! 实际投产模型由 todo 45 核许可后选定，故模型目录布局在此处保持可替换。

use std::path::{Path, PathBuf};

use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};

use crate::VoiceError;

/// Whisper 模型目录里必须齐备的三个文件。缺一个都构造不出识别器，
/// 而 sherpa-onnx 在缺失时抛的是无路径信息的通用错误。
const WHISPER_FILES: &[&str] = &["tiny-encoder.onnx", "tiny-decoder.onnx", "tiny-tokens.txt"];

pub struct Recognizer {
    inner: WhisperRecognizer,
}

impl Recognizer {
    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失的模型文件；`Backend` 转述 sherpa-onnx 的构造失败。
    pub fn new(model_dir: &Path) -> Result<Self, VoiceError> {
        VoiceError::require_files(model_dir, WHISPER_FILES)?;

        let joined = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
        let config = WhisperConfig {
            encoder: joined("tiny-encoder.onnx"),
            decoder: joined("tiny-decoder.onnx"),
            tokens: joined("tiny-tokens.txt"),
            language: "en".to_owned(),
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
        Ok(self.inner.transcribe(sample_rate, &samples).text)
    }
}

/// 模型根目录。`YUNJIAN_MODEL_DIR` 覆盖，默认为仓库内 `models/cache`——`.gitignore`
/// 已排除该路径，权重因此不会被误提交。
///
/// 默认值在编译期由 `CARGO_MANIFEST_DIR` 推出绝对路径，而不是写相对路径：
/// `cargo test -p yunjian-voice` 的工作目录是 crate 目录而非工作区根，相对路径会找错地方。
/// 按需下载与配置驱动的模型目录是 todo 53 的事，这里只保证 spike 可复现。
#[must_use]
pub fn model_root() -> PathBuf {
    std::env::var_os("YUNJIAN_MODEL_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("models")
                .join("cache")
        },
        PathBuf::from,
    )
}
