//! 离线合成。
//!
//! 节奏本身不在这里：sherpa-onnx 既无 SSML，其 `silence_scale` 也已报损（上游 #2043），
//! 所以音步切分、静音拼接与时间戳都在 [`crate::prosody`]，那一层不带特性开关因而无模型
//! 也能测。本模块提供两块砖：单次合成，以及把 [`Synthesizer`] 接到
//! [`prosody::FootSynthesizer`] 上、并把破读词表注入引擎词典。
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
use crate::lexicon::{PhonemeIndex, Poyin, compile_overrides};
use crate::prosody::{self, FootAudio, FootSynthesizer};

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
                    // 破读覆写词典排在基础词典**之后**：sherpa-onnx 按逗号顺序合并，后面的
                    // 同名词条覆盖前面的，顺序反了覆写就不生效，而那种失效是听不出来的
                    // ——合成照常成功，只是读音仍是现代普通话。
                    lexicon: match model_dir.join(POYIN_LEXICON).exists() {
                        true => format!("{},{}", joined("lexicon.txt"), joined(POYIN_LEXICON)),
                        false => joined("lexicon.txt"),
                    },
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

/// 破读覆写词典在模型目录内的文件名。
///
/// 落在模型目录里而不是别处，是因为音素是**逐模型**的：同一句「石径斜」在 MeloTTS 与 Kokoro
/// 的音系里写法不同，把一个模型的覆写喂给另一个只会得到乱音。放在模型目录内让它随模型
/// 一起被删除，不会留下一份对不上号的残留。
pub const POYIN_LEXICON: &str = "lexicon-poyin.txt";

/// 把破读词表编译成覆写词典并写进模型目录。
///
/// 返回写出的词条数。**在 [`Synthesizer::new`] 之前调用**：引擎在构造时读词典，之后再写
/// 文件不会生效。
///
/// 只支持 VITS（MeloTTS 中文包）：实测它的 `lexicon.txt` 是拼音编码的
/// （`斜 x ie 2 2`），因此覆写音素可以从词典自身借用同音条目而**一个都不用手写**。
/// Kokoro 的 `lexicon-zh.txt` 是 IPA 编码（`斜 ɕ j e ↗`），从拼音到它的音素没有可核对的
/// 路径，所以本函数对它返回 [`VoiceError::Backend`] 而不是写出一份猜出来的词典。
///
/// # Errors
///
/// 目录不是 VITS 模型、基础词典读不到、编译覆写失败（含读音无法表达）、或落盘失败。
pub fn install_poyin_lexicon(model_dir: &Path, poyin: &Poyin) -> Result<usize, VoiceError> {
    if detect(model_dir)? != Engine::Vits {
        return Err(VoiceError::Backend(format!(
            "{} 不是 VITS 模型目录；破读覆写只支持拼音编码的 MeloTTS 词典，\
             Kokoro 的 IPA 词典没有可核对的拼音到音素路径",
            model_dir.display()
        )));
    }
    let base_path = model_dir.join("lexicon.txt");
    let base = std::fs::read_to_string(&base_path).map_err(|e| VoiceError::AudioRead {
        path: base_path.clone(),
        source: Box::new(e),
    })?;
    let index = PhonemeIndex::parse(&base);
    let overrides = compile_overrides(poyin, &index)
        .map_err(|problems| VoiceError::Backend(problems.join("；")))?;
    let mut text = String::new();
    for entry in &overrides {
        text.push_str(&entry.line());
        text.push('\n');
    }
    let target = model_dir.join(POYIN_LEXICON);
    std::fs::write(&target, text).map_err(|e| VoiceError::AudioWrite {
        path: target,
        source: Box::new(e),
    })?;
    tracing::info!(
        model_dir = %model_dir.display(),
        entries = overrides.len(),
        "破读覆写词典已写入"
    );
    Ok(overrides.len())
}

/// 逐音步合成，供 [`prosody::splice`] 驱动。
///
/// 说话人固定取 0、语速固定取 1.0：**音步之间的时长关系由拼接的静音承担**，如果这里再按
/// 音步调语速，两处控制会互相干扰，而其中一处（引擎侧）是不可观测的。
impl FootSynthesizer for Synthesizer {
    fn synthesize_foot(&mut self, text: &str) -> Result<FootAudio, VoiceError> {
        let audio = self.synthesize(text, 0)?;
        Ok(FootAudio {
            samples: audio.samples,
            sample_rate: audio.sample_rate,
        })
    }
}

/// 合成一段带节奏的朗读。
///
/// 这是朗读功能对外的入口：切分交给调用方（近体诗走 [`prosody::segment_metrical`]，词走
/// [`prosody::segment_ci`]），本函数只负责逐音步合成加拼接。
///
/// # Errors
///
/// 转述合成失败与采样率不一致。
pub fn read_aloud(
    synthesizer: &mut Synthesizer,
    segmentation: &prosody::Segmentation,
    prosody: prosody::Prosody,
) -> Result<prosody::Reading, VoiceError> {
    prosody::splice(synthesizer, segmentation, prosody)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Engine, POYIN_LEXICON, Synthesizer, detect, install_poyin_lexicon, read_aloud};
    use crate::lexicon::{PhonemeIndex, Poyin, Syllable, compile_overrides};
    use crate::prosody::{
        Prosody, SILENCE_FLOOR_DBFS, gap_between, segment_metrical, silent_spans,
    };

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

    // 下面三条要么读真实 VITS 词典、要么真跑合成，缺模型时无法执行，因此都挂
    // `cfg_attr(not(melo_model_present), ignore = ...)`。条件由 `build.rs` 在构建期按目录
    // 存在性供给——`ignore` 只接受字面量，运行期判目录再 `return` 会让 harness 打印 `ok`，
    // 那是「没跑」冒充「通过」。挂 `ignore` 则在输出里留下一行带理由的 `ignored`，
    // 而模型在位时照常真跑。理由字面量逐条重复（属性不接受 const），与
    // `crates/yunjian-cli/tests/install_scripts.rs` 门控 POSIX 脚本用例的写法一致。

    /// 随包 MeloTTS 中文包的目录。
    ///
    /// 断言保留是刻意的：调用方已被 `melo_model_present` 门控，走到这里说明 cfg 说「在」，
    /// 那么目录不在就是环境在编译与执行之间被改坏了，此时**必须变红**而不是跳过。
    fn vits_dir() -> PathBuf {
        let dir = crate::models::cache_root().join("vits-melo-tts-zh_en");
        assert!(
            dir.is_dir(),
            "缺少模型目录 {}，但构建期探测认为它存在——环境在编译与执行之间变了。\n\
             跑 `yunjian models fetch vits-melo-tts-zh_en`，或用 YUNJIAN_MODEL_DIR 指向已有目录。",
            dir.display()
        );
        dir
    }

    fn real_index() -> PhonemeIndex {
        let path = vits_dir().join("lexicon.txt");
        PhonemeIndex::parse(&std::fs::read_to_string(&path).expect("基础词典可读"))
    }

    /// **验收条目**：三个黄金破读经词表产出期望拼音。
    ///
    /// 在真实引擎词典上核对：每一条覆写的音素必须与词典里那个目标读音的同音条目逐字节相等。
    /// 这条断言之所以有力，是因为音素**没有一个是手写的**——它们全部借自词典自身，于是
    /// 「产出期望拼音」不是一句声明，而是可以和词典对齐的事实。
    #[test]
    #[cfg_attr(
        not(melo_model_present),
        ignore = "缺少 models/cache/vits-melo-tts-zh_en：本用例读真实 VITS 词典或真跑合成，缺模型无法执行；跑 `yunjian models fetch vits-melo-tts-zh_en` 或用 YUNJIAN_MODEL_DIR 指向已有缓存后重跑"
    )]
    fn the_three_golden_readings_compile_to_the_expected_phonemes() {
        let index = real_index();
        let poyin = Poyin::shipped().expect("随仓破读词表应可解析");
        let overrides = compile_overrides(&poyin, &index).expect("覆写应可编译");

        for (word, character, pinyin) in [
            ("石径斜", '斜', "xiá"),
            ("鬓毛衰", '衰', "cuī"),
            ("一骑", '骑', "jì"),
        ] {
            let entry = overrides
                .iter()
                .find(|entry| entry.word == word)
                .unwrap_or_else(|| panic!("覆写词典缺词条 {word}"));
            let syllable = Syllable::parse(pinyin).expect("拼音应可解析");
            let want = index
                .syllable(&syllable)
                .unwrap_or_else(|| panic!("引擎词典里没有读 {pinyin} 的条目"));
            assert!(
                entry.phonemes.ends_with(want),
                "{word} 里 {character} 的音素应为 {want}（{pinyin}），实得 {:?}",
                entry.phonemes
            );
            let default = index.character(character).expect("该字应在词典里");
            assert_ne!(
                want, default,
                "{character} 的破读音素与默认音素相同，说明这条覆写没有任何效果"
            );
        }
    }

    /// 覆写词典写进模型目录后，VITS 的词典参数必须把它排在基础词典之后。
    #[test]
    #[cfg_attr(
        not(melo_model_present),
        ignore = "缺少 models/cache/vits-melo-tts-zh_en：本用例读真实 VITS 词典或真跑合成，缺模型无法执行；跑 `yunjian models fetch vits-melo-tts-zh_en` 或用 YUNJIAN_MODEL_DIR 指向已有缓存后重跑"
    )]
    fn installing_the_override_lexicon_writes_one_line_per_override() {
        let source = vits_dir();
        let scratch = std::env::temp_dir().join("yunjian-poyin-install");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("临时目录可创建");
        // 只复制判定与编译需要的两个文件：detect 看存在性，install 读基础词典。
        // 不复制 163 MB 的权重，因为本用例不做推理。
        std::fs::copy(source.join("lexicon.txt"), scratch.join("lexicon.txt")).expect("可复制词典");
        std::fs::write(scratch.join("model.onnx"), b"placeholder").expect("可写占位权重");
        std::fs::write(scratch.join("tokens.txt"), b"placeholder").expect("可写占位符号表");

        let poyin = Poyin::shipped().expect("应可解析");
        let written = install_poyin_lexicon(&scratch, &poyin).expect("应可写入");
        assert_eq!(written, poyin.override_count());

        let text = std::fs::read_to_string(scratch.join(POYIN_LEXICON)).expect("覆写词典可读");
        assert_eq!(text.lines().count(), written);
        assert!(text.lines().any(|line| line.starts_with("石径斜 ")));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Kokoro 的词典是 IPA 编码，从拼音到它的音素没有可核对的路径，所以必须报错而不是
    /// 写出一份猜出来的词典。
    #[test]
    fn installing_into_a_non_vits_model_is_refused() {
        let scratch = std::env::temp_dir().join("yunjian-poyin-refuse");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("临时目录可创建");
        for name in ["model.onnx", "voices.bin", "tokens.txt", "lexicon-zh.txt"] {
            std::fs::write(scratch.join(name), b"placeholder").expect("可写占位文件");
        }
        let poyin = Poyin::shipped().expect("应可解析");
        let error = install_poyin_lexicon(&scratch, &poyin).expect_err("非 VITS 目录应被拒绝");
        assert!(error.to_string().contains("VITS"), "{error}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// **验收条目，在真实合成上跑**：七言的每个 二二三 音步边界至少 `foot_pause_ms`、
    /// 行边界至少 `line_pause_ms`，且时间戳数等于音步数。
    ///
    /// 这是整条链路的端到端证明：真实模型逐音步合成，Rust 侧插静音，再用 RMS 把静音量回来。
    #[test]
    #[cfg_attr(
        not(melo_model_present),
        ignore = "缺少 models/cache/vits-melo-tts-zh_en：本用例读真实 VITS 词典或真跑合成，缺模型无法执行；跑 `yunjian models fetch vits-melo-tts-zh_en` 或用 YUNJIAN_MODEL_DIR 指向已有缓存后重跑"
    )]
    fn real_synthesis_has_the_configured_boundary_silence() {
        let mut synthesizer = Synthesizer::new(&vits_dir()).expect("合成器应可构造");
        let prosody = Prosody::CLASSICAL;
        let segmentation = segment_metrical(["远上寒山石径斜", "白云生处有人家"]);
        let reading = read_aloud(&mut synthesizer, &segmentation, prosody).expect("朗读应成功");

        assert_eq!(
            reading.marks.len(),
            segmentation.foot_count(),
            "时间戳数应等于音步数"
        );
        assert_eq!(reading.marks.len(), 6);
        assert!(crate::rms(&reading.samples) > 1e-3, "整段不该是静音");

        for (first, second) in [(0, 1), (1, 2), (3, 4), (4, 5)] {
            let gap = gap_between(&reading, first, second)
                .unwrap_or_else(|| panic!("音步 {first}→{second} 之间没有静音"));
            assert!(
                gap >= prosody.foot_pause(),
                "音步 {first}→{second} 只停了 {gap:?}，短于配置的 {:?}",
                prosody.foot_pause()
            );
        }
        let line_gap = gap_between(&reading, 2, 3).expect("行边界应有静音");
        assert!(
            line_gap >= prosody.line_pause(),
            "行边界只停了 {line_gap:?}，短于配置的 {:?}",
            prosody.line_pause()
        );

        // 失败场景的常驻版本：同一行整句一次合成，边界静音就不存在了。
        let whole = synthesizer
            .synthesize("远上寒山石径斜", 0)
            .expect("整句合成应成功");
        let spans = silent_spans(&whole.samples, whole.sample_rate, SILENCE_FLOOR_DBFS);
        let longest = spans
            .iter()
            .map(|span| span.duration(whole.sample_rate))
            .max()
            .unwrap_or_default();
        assert!(
            longest < prosody.foot_pause(),
            "整句一次合成竟然含 {longest:?} 的静音，不短于配置的音步停顿 {:?}；\
             那意味着节奏不是拼接产出的，本任务的核心论断需要重新验证",
            prosody.foot_pause()
        );
    }
}
