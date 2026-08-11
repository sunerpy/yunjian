//! 离线合成。
//!
//! 真正的节奏控制不在这里：sherpa-onnx 既无 SSML，其 `silence_scale` 也已报损
//! （上游 #2043），所以逐音步合成加 Rust 侧静音拼接是 todo 52 的事，本模块只提供
//! 单次合成这一块砖。
//!
//! 支持三个引擎，由目录内容自动判定，调用方不必知道自己拿到的是哪一个：
//!
//! | 引擎     | 判据                                     | 本项目用途                     |
//! | -------- | ---------------------------------------- | ------------------------------ |
//! | Kitten   | `model.fp16.onnx` + `voices.bin`         | 英文冒烟（todo 44）            |
//! | Kokoro   | `voices.bin` + `model.onnx`              | 中文声音之二（Apache-2.0）     |
//! | VITS     | `model.onnx` + `lexicon.txt`             | 中文声音之一（MeloTTS，MIT）   |
//!
//! 判据顺序不可交换：Kitten 与 Kokoro 都有 `voices.bin`，区分靠 Kitten 的 fp16 权重名。

use std::path::Path;

use sherpa_rs::tts::{
    KittenTts, KittenTtsConfig, KokoroTts, KokoroTtsConfig, VitsTts, VitsTtsConfig,
};

use crate::VoiceError;

/// 合成引擎。报告与日志里需要它，因为「同一段文本换个引擎」是 CER 实测的一个自变量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Kitten,
    Kokoro,
    Vits,
}

impl Engine {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kitten => "kitten",
            Self::Kokoro => "kokoro",
            Self::Vits => "vits",
        }
    }
}

pub struct Synthesized {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

enum Backend {
    Kitten(Box<KittenTts>),
    Kokoro(Box<KokoroTts>),
    Vits(Box<VitsTts>),
}

pub struct Synthesizer {
    inner: Backend,
    engine: Engine,
}

impl Synthesizer {
    /// 按目录内容判定引擎并构造。
    ///
    /// # Errors
    ///
    /// `ModelMissing` 指出第一个缺失的模型文件；目录不像任何已知引擎时返回 `Backend`。
    ///
    /// # Panics
    ///
    /// 上游三个 `*Tts::new` 的签名都返回 `Self` 而非 `Result`，模型损坏时它自己 panic。
    /// 前置的文件存在性检查把最常见的一类失败拦在 panic 之前。
    pub fn new(model_dir: &Path) -> Result<Self, VoiceError> {
        let engine = detect(model_dir)?;
        let joined = |name: &str| model_dir.join(name).to_string_lossy().into_owned();
        let optional = |name: &str| {
            if model_dir.join(name).exists() {
                joined(name)
            } else {
                String::new()
            }
        };

        let inner = match engine {
            Engine::Kitten => {
                VoiceError::require_files(
                    model_dir,
                    &["model.fp16.onnx", "voices.bin", "tokens.txt"],
                )?;
                Backend::Kitten(Box::new(KittenTts::new(KittenTtsConfig {
                    model: joined("model.fp16.onnx"),
                    voices: joined("voices.bin"),
                    tokens: joined("tokens.txt"),
                    data_dir: optional("espeak-ng-data"),
                    length_scale: 1.0,
                    ..Default::default()
                })))
            }
            Engine::Kokoro => {
                VoiceError::require_files(
                    model_dir,
                    &["model.onnx", "voices.bin", "tokens.txt", "lexicon-zh.txt"],
                )?;
                // 词典按逗号拼接是 sherpa-onnx 的既定约定（`--kokoro-lexicon` 同形）。
                // 中英双词典都给上：诗题里出现拉丁字母时不至于整句发音失败。
                let lexicon = format!(
                    "{},{}",
                    optional("lexicon-us-en.txt"),
                    joined("lexicon-zh.txt")
                );
                Backend::Kokoro(Box::new(KokoroTts::new(KokoroTtsConfig {
                    model: joined("model.onnx"),
                    voices: joined("voices.bin"),
                    tokens: joined("tokens.txt"),
                    data_dir: optional("espeak-ng-data"),
                    dict_dir: optional("dict"),
                    lexicon,
                    length_scale: 1.0,
                    // **`lang` 必须留空**。它取的是 espeak-ng 的 voice 名，官方帮助
                    // 里给的例子只有 en / es / fr / hi / it / pt-br，没有中文；填 `"zh"`
                    // 会让 sherpa-onnx 抛 `Failed to set eSpeak-ng voice`，而那是一个
                    // **C++ 异常**——Rust 接不住它，进程直接 abort（实测信息：
                    // `fatal runtime error: Rust cannot catch foreign exceptions`），
                    // 连错误处理的机会都没有。中文读音走上面的 `lexicon`，官方帮助
                    // 也明说「留空则须提供 --kokoro-lexicon」。
                    lang: String::new(),
                    ..Default::default()
                })))
            }
            Engine::Vits => {
                VoiceError::require_files(model_dir, &["model.onnx", "tokens.txt", "lexicon.txt"])?;
                Backend::Vits(Box::new(VitsTts::new(VitsTtsConfig {
                    model: joined("model.onnx"),
                    lexicon: joined("lexicon.txt"),
                    tokens: joined("tokens.txt"),
                    // MeloTTS 中文包不带 espeak-ng-data，靠 `dict/`（jieba）分词。
                    // 两者都传空串而不是不存在的路径：sherpa-onnx 对空串是「不用」，
                    // 对错误路径则是硬失败。
                    dict_dir: optional("dict"),
                    data_dir: optional("espeak-ng-data"),
                    length_scale: 1.0,
                    noise_scale: 0.667,
                    noise_scale_w: 0.8,
                    ..Default::default()
                })))
            }
        };

        Ok(Self { inner, engine })
    }

    #[must_use]
    pub const fn engine(&self) -> Engine {
        self.engine
    }

    /// 以正常语速合成。
    ///
    /// # Errors
    ///
    /// `Backend` 转述 sherpa-onnx 的合成失败。
    pub fn synthesize(&mut self, text: &str, speaker: i32) -> Result<Synthesized, VoiceError> {
        self.synthesize_at(text, speaker, 1.0)
    }

    /// 指定语速合成。`speed` 大于 1 更快。
    ///
    /// 这是引擎内部的时长控制，与 `augment::time_stretch` 的后处理不同：前者重新
    /// 生成声学特征，后者只重排已有波形。CER 实测用后者，因为增强的意义是「同一段
    /// 录音在信道上的变化」，不是「另录一遍」。
    ///
    /// # Errors
    ///
    /// `Backend` 转述 sherpa-onnx 的合成失败。
    pub fn synthesize_at(
        &mut self,
        text: &str,
        speaker: i32,
        speed: f32,
    ) -> Result<Synthesized, VoiceError> {
        let audio = match &mut self.inner {
            Backend::Kitten(t) => t.create(text, speaker, speed),
            Backend::Kokoro(t) => t.create(text, speaker, speed),
            Backend::Vits(t) => t.create(text, speaker, speed),
        }
        .map_err(|e| VoiceError::Backend(e.to_string()))?;
        Ok(Synthesized {
            samples: audio.samples,
            sample_rate: audio.sample_rate,
        })
    }
}

fn detect(model_dir: &Path) -> Result<Engine, VoiceError> {
    if !model_dir.is_dir() {
        return Err(VoiceError::ModelMissing {
            path: model_dir.to_path_buf(),
        });
    }
    let has = |name: &str| model_dir.join(name).exists();
    if has("model.fp16.onnx") && has("voices.bin") {
        return Ok(Engine::Kitten);
    }
    if has("voices.bin") && has("model.onnx") {
        return Ok(Engine::Kokoro);
    }
    if has("model.onnx") && has("lexicon.txt") {
        return Ok(Engine::Vits);
    }
    Err(VoiceError::Backend(format!(
        "{} 不像 Kitten / Kokoro / VITS 任何一种模型目录：\
         Kitten 需要 model.fp16.onnx + voices.bin，Kokoro 需要 model.onnx + voices.bin，\
         VITS 需要 model.onnx + lexicon.txt",
        model_dir.display()
    )))
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

#[cfg(test)]
mod tests {
    use super::{Engine, detect};

    fn scratch(name: &str, files: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yunjian-tts-detect-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录可创建");
        for f in files {
            std::fs::write(dir.join(f), b"x").expect("可写入");
        }
        dir
    }

    /// Kitten 与 Kokoro 都有 `voices.bin`，判据顺序错了就会把 Kitten 当成 Kokoro，
    /// 而那种错误只会在合成出静音时才暴露。
    #[test]
    fn kitten_is_not_mistaken_for_kokoro() {
        let dir = scratch("kitten", &["model.fp16.onnx", "voices.bin", "tokens.txt"]);
        assert_eq!(detect(&dir).expect("应判定"), Engine::Kitten);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kokoro_and_vits_are_distinguished_by_voices_bin() {
        let kokoro = scratch("kokoro", &["model.onnx", "voices.bin", "lexicon-zh.txt"]);
        assert_eq!(detect(&kokoro).expect("应判定"), Engine::Kokoro);
        let vits = scratch("vits", &["model.onnx", "lexicon.txt", "tokens.txt"]);
        assert_eq!(detect(&vits).expect("应判定"), Engine::Vits);
        let _ = std::fs::remove_dir_all(&kokoro);
        let _ = std::fs::remove_dir_all(&vits);
    }

    #[test]
    fn unknown_layout_names_all_three_expectations() {
        let dir = scratch("unknown", &["README.md"]);
        let text = detect(&dir).expect_err("应报错").to_string();
        assert!(
            text.contains("Kitten") && text.contains("Kokoro") && text.contains("VITS"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
