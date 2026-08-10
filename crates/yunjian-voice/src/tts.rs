//! 离线合成。冒烟阶段用 Kitten nano（权重 Apache-2.0），只为证明构建与链接通路成立。
//!
//! 真正的节奏控制不在这里：sherpa-onnx 既无 SSML，其 `silence_scale` 也已报损
//! （上游 #2043），所以逐音步合成加 Rust 侧静音拼接是 todo 52 的事，本模块只提供
//! 单次合成这一块砖。

use std::path::Path;

use sherpa_rs::tts::{KittenTts, KittenTtsConfig};

use crate::VoiceError;

const KITTEN_FILES: &[&str] = &["model.fp16.onnx", "voices.bin", "tokens.txt"];

pub struct Synthesized {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct Synthesizer {
    inner: KittenTts,
}

impl Synthesizer {
    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失的模型文件。
    ///
    /// # Panics
    ///
    /// `KittenTts::new` 的上游签名返回 `Self` 而非 `Result`，模型损坏时它自己 panic。
    /// 前置的文件存在性检查把最常见的一类失败拦在 panic 之前。
    pub fn new(model_dir: &Path) -> Result<Self, VoiceError> {
        VoiceError::require_files(model_dir, KITTEN_FILES)?;

        let joined = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
        let config = KittenTtsConfig {
            model: joined("model.fp16.onnx"),
            voices: joined("voices.bin"),
            tokens: joined("tokens.txt"),
            data_dir: joined("espeak-ng-data"),
            length_scale: 1.0,
            ..Default::default()
        };

        Ok(Self {
            inner: KittenTts::new(config),
        })
    }

    /// # Errors
    ///
    /// `Backend` 转述 sherpa-onnx 的合成失败。
    pub fn synthesize(&mut self, text: &str, speaker: i32) -> Result<Synthesized, VoiceError> {
        let audio = self
            .inner
            .create(text, speaker, 1.0)
            .map_err(|e| VoiceError::Backend(e.to_string()))?;
        Ok(Synthesized {
            samples: audio.samples,
            sample_rate: audio.sample_rate,
        })
    }
}

/// # Errors
///
/// `AudioWrite` 转述 WAV 编码或落盘失败。
pub fn write_wav(path: &Path, audio: &Synthesized) -> Result<(), VoiceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| VoiceError::AudioWrite {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;
    }
    sherpa_rs::write_audio_file(&path.to_string_lossy(), &audio.samples, audio.sample_rate).map_err(
        |e| VoiceError::AudioWrite {
            path: path.to_path_buf(),
            source: e.to_string().into(),
        },
    )
}
