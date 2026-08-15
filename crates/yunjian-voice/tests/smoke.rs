//! Wave 0 冒烟：证明 sherpa-onnx 的原生库真的被链接进可执行文件，并且能跑出
//! 一次真实识别与一次真实合成。
//!
//! 断言选 RMS 而不是长度：长度对得上但全零的缓冲区能骗过任何长度断言，
//! 而那正是链接成功、推理却没跑起来时的典型产物。
//!
//! # 模型缺失时的行为
//!
//! 两条冒烟各挂 `cfg_attr(not(<模型>_present), ignore = ...)`，条件由 `build.rs` 在构建期
//! 按模型目录存在性供给。**这不是「缺模型就算过」**：`ignore` 会在测试输出里留下一行
//! `ignored` 与理由，而运行期判目录再 `return` 会让 harness 打印 `ok`——后者才是
//! 「没跑」伪装成「通过」。两条依赖不同模型（合成用 Kitten、识别用 whisper-tiny），
//! 所以门控也是两个独立的 cfg，缺一个不会把另一个一起静默掉。

#![cfg(feature = "voice")]

use std::path::{Path, PathBuf};

use yunjian_voice::{asr, rms, tts};

const ASR_MODEL: &str = "sherpa-onnx-whisper-tiny";
const TTS_MODEL: &str = "kitten-nano-en-v0_2-fp16";
/// 合成语音的 RMS 实测在 0.05 量级；1e-3 足以排除静音又不至于因音量波动误报。
const SILENCE_FLOOR: f32 = 1e-3;

/// 断言保留是刻意的：调用方已被构建期 cfg 门控，走到这里说明 cfg 说「在」，
/// 那么目录不在就是环境在编译与执行之间被改坏了，此时必须变红而不是跳过。
fn model_dir(name: &str) -> PathBuf {
    let dir = asr::model_root().join(name);
    assert!(
        dir.is_dir(),
        "缺少模型目录 {}，但构建期探测认为它存在——环境在编译与执行之间变了。\n\
         按 docs/VOICE-BUILD.zh.md「冒烟模型」一节下载，或用 YUNJIAN_MODEL_DIR 指向已有目录。",
        dir.display()
    );
    dir
}

/// 刻意不用 `sherpa_rs::read_audio_file`：它硬断言 16 kHz，读不了 Kitten 输出的 24 kHz，
/// 也就无法用来校验「写出去的东西读回来还是有声音的」。
fn read_wav(path: &Path) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path).expect("WAV 可打开");
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.expect("样点可解码"))
            .collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| f32::from(s.expect("样点可解码")) / f32::from(i16::MAX))
            .collect(),
    };
    (samples, spec.sample_rate)
}

fn out_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("voice-smoke");
    std::fs::create_dir_all(&dir).expect("测试输出目录可创建");
    dir
}

#[test]
#[cfg_attr(
    not(kitten_model_present),
    ignore = "缺少 models/cache/kitten-nano-en-v0_2-fp16：本用例真跑合成，缺权重无法执行；按 docs/VOICE-BUILD.zh.md「冒烟模型」一节下载，或用 YUNJIAN_MODEL_DIR 指向已有缓存后重跑"
)]
fn smoke_synthesis_writes_non_silent_wav() {
    let mut synth = tts::Synthesizer::new(&model_dir(TTS_MODEL)).expect("合成器可构造");
    let audio = synth
        .synthesize("Bright moonlight before my bed.", 2)
        .expect("合成成功");

    assert!(
        audio.sample_rate >= 8000,
        "采样率异常：{}",
        audio.sample_rate
    );
    assert!(!audio.samples.is_empty(), "合成结果为空");
    let level = rms(&audio.samples);
    assert!(level > SILENCE_FLOOR, "合成结果是静音：RMS={level}");

    let wav = out_dir().join("synthesized.wav");
    tts::write_wav(&wav, &audio).expect("WAV 可写出");

    let (readback, rate) = read_wav(&wav);
    assert_eq!(rate, audio.sample_rate, "回读采样率与写入不一致");
    let readback_level = rms(&readback);
    assert!(
        readback_level > SILENCE_FLOOR,
        "回读音频是静音：RMS={readback_level}"
    );
}

/// 随包 WAV 的产生方式，留在仓库里以说明来源：它是本项目用 Apache-2.0 的
/// Kitten nano 合成出来的自有音频，因此可以随仓库分发；不引入任何第三方录音的许可问题。
/// 重新生成：`cargo test -p yunjian-voice --features voice -- --ignored regenerate`
#[test]
#[ignore = "只在需要重新生成随包 WAV 时手动运行"]
fn regenerate_bundled_fixture() {
    let mut synth = tts::Synthesizer::new(&model_dir(TTS_MODEL)).expect("合成器可构造");
    let audio = synth
        .synthesize("The bright moon shines before my bed.", 2)
        .expect("合成成功");

    let resampled = resample_linear(&audio.samples, audio.sample_rate, 16_000);
    let target = tts::Synthesized {
        samples: resampled,
        sample_rate: 16_000,
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bundled-16k.wav");
    tts::write_wav(&path, &target).expect("fixture 可写出");
}

/// 线性插值重采样。只用于生成 fixture，精度足够；生产路径的重采样是 todo 52 的事。
fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            let frac = (pos - pos.floor()) as f32;
            samples[lo] * (1.0 - frac) + samples[hi] * frac
        })
        .collect()
}

#[test]
#[cfg_attr(
    not(whisper_tiny_model_present),
    ignore = "缺少 models/cache/sherpa-onnx-whisper-tiny：本用例真跑识别，缺权重无法执行；按 docs/VOICE-BUILD.zh.md「冒烟模型」一节下载，或用 YUNJIAN_MODEL_DIR 指向已有缓存后重跑"
)]
fn smoke_recognition_of_bundled_wav_yields_text() {
    let wav = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bundled-16k.wav");
    assert!(wav.is_file(), "随仓库携带的 WAV 缺失：{}", wav.display());

    let (samples, rate) = read_wav(&wav);
    assert_eq!(rate, 16_000, "随包 WAV 必须是 16 kHz");
    assert!(rms(&samples) > SILENCE_FLOOR, "随包 WAV 是静音，识别无意义");

    let mut recognizer = asr::Recognizer::new(&model_dir(ASR_MODEL)).expect("识别器可构造");
    let text = recognizer.transcribe_wav(&wav).expect("识别成功");

    assert!(!text.trim().is_empty(), "识别结果为空");
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("moon"),
        "识别结果与随包音频内容不符：{text}"
    );
}
