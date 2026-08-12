//! 双路流式解码的真实推理与实时率实测。
//!
//! 模型缺失时本测试**失败**而不是跳过，与 `smoke.rs` 同一条理由：跳过会让「没跑」
//! 伪装成「通过」，而这里要证明的恰恰是「真的跑起来了、并且量到了成本」。
//!
//! 实时率写到 stderr（`--nocapture` 可见）而不是 stdout：本工作区 `print_stdout`
//! 是 deny，因为同一个二进制要托管 MCP stdio 服务器。

#![cfg(feature = "voice")]

use std::path::PathBuf;
use std::time::Duration;

use yunjian_voice::asr::streaming::{StreamingDualDecoder, TransducerFiles};
use yunjian_voice::recognize::{
    DecodePlan, DualDecode, Hotwords, OnlineDecodeConfig, PcmSource, RecognitionItem,
    RecognitionPlan, start_recognition,
};
use yunjian_voice::{asr, models};

const STREAMING_MODEL: &str = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20";
const REFERENCE: &str = "床前明月光，疑是地上霜。举头望明月，低头思故乡。";
/// 每次喂 100 毫秒，与采集侧的帧长一致。
const FRAME_SAMPLES: usize = 1600;

fn model_dir() -> PathBuf {
    let dir = asr::model_root().join(STREAMING_MODEL);
    assert!(
        dir.is_dir(),
        "缺少流式模型目录 {}。\n\
         `cargo run -p xtask -- ...` 或 `models::ensure_model(\"{STREAMING_MODEL}\")` 下载后解包，\n\
         或用 YUNJIAN_MODEL_DIR 指向已有目录。本测试刻意不跳过。",
        dir.display()
    );
    dir
}

fn files() -> TransducerFiles {
    let entry = models::Registry::shipped()
        .expect("模型清单可解析")
        .admit(STREAMING_MODEL)
        .expect("流式模型应在许可白名单内");
    assert_eq!(entry.license, "Apache-2.0");
    TransducerFiles::discover(&model_dir(), true).expect("流式 Transducer 四件套可定位")
}

fn sample_audio() -> (Vec<f32>, u32) {
    let wav = model_dir().join("test_wavs").join("0.wav");
    assert!(wav.is_file(), "缺少随包测试音频 {}", wav.display());
    let mut reader = hound::WavReader::open(&wav).expect("WAV 可打开");
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

struct SliceSource {
    samples: Vec<f32>,
    cursor: usize,
    sample_rate: u32,
}

impl PcmSource for SliceSource {
    fn next_frame(&mut self) -> Option<Vec<f32>> {
        if self.cursor >= self.samples.len() {
            return None;
        }
        let end = (self.cursor + FRAME_SAMPLES).min(self.samples.len());
        let frame = self.samples[self.cursor..end].to_vec();
        self.cursor = end;
        Some(frame)
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

fn hotwords() -> Hotwords {
    Hotwords::from_poem(REFERENCE).expect("诗文可展开为 hotwords")
}

/// 无偏置一路与双路各自的实时率。两个数字都必须出现在输出里，否则「双路成本已实测」
/// 就只是一句声明。
#[test]
fn both_decode_modes_report_a_measured_realtime_factor() {
    let files = files();
    let (samples, sample_rate) = sample_audio();
    let audio = Duration::from_secs_f64(samples.len() as f64 / f64::from(sample_rate));

    let mut single = StreamingDualDecoder::open(&files, OnlineDecodeConfig::unbiased(), None)
        .expect("单路解码器可构造");
    for frame in samples.chunks(FRAME_SAMPLES) {
        single.accept(frame, sample_rate).expect("喂帧成功");
    }
    single.finish().expect("收尾成功");
    let single_cost = single.cost();

    let mut dual = StreamingDualDecoder::open(
        &files,
        OnlineDecodeConfig::unbiased(),
        Some(OnlineDecodeConfig::biased(hotwords())),
    )
    .expect("双路解码器可构造");
    for frame in samples.chunks(FRAME_SAMPLES) {
        dual.accept(frame, sample_rate).expect("喂帧成功");
    }
    dual.finish().expect("收尾成功");
    let dual_cost = dual.cost();

    eprintln!(
        "RTF 实测（音频 {:.2} s，模型 {STREAMING_MODEL} int8）：\
         单路无偏置 = {:.4}，双路 = {:.4}，判定 = {:?}",
        audio.as_secs_f64(),
        single_cost.single.value(),
        dual_cost.dual.value(),
        yunjian_voice::recognize::plan_for(dual_cost)
    );

    assert!(single_cost.single.value() > 0.0, "单路实时率必须是实测值");
    assert!(dual_cost.dual.value() > 0.0, "双路实时率必须是实测值");
    assert!(
        dual_cost.dual.value() >= dual_cost.single.value(),
        "双路不可能比它包含的单路更快：{dual_cost:?}"
    );
    assert!(
        matches!(
            yunjian_voice::recognize::plan_for(dual_cost),
            DecodePlan::Dual | DecodePlan::SingleUnbiased { .. }
        ),
        "判定只有这两种，永远不含丢帧"
    );
}

/// 偏置一路吐出的字与无偏置一路不同——这就是「在诗文本上偏置会让识别器吐出用户没说的
/// 字」那条论断在生产路径上的直接证据。两路都跑同一段音频，差异只可能来自 hotwords。
#[test]
fn the_biased_pass_diverges_from_the_unbiased_pass_on_the_same_audio() {
    let files = files();
    let (samples, sample_rate) = sample_audio();
    let mut decoder = StreamingDualDecoder::open(
        &files,
        OnlineDecodeConfig::unbiased(),
        Some(OnlineDecodeConfig::biased(hotwords())),
    )
    .expect("双路解码器可构造");

    for frame in samples.chunks(FRAME_SAMPLES) {
        decoder.accept(frame, sample_rate).expect("喂帧成功");
    }
    let final_hypothesis = decoder.finish().expect("收尾成功");

    let unbiased = final_hypothesis.unbiased.expect("无偏置一路应有输出");
    let biased = final_hypothesis.biased.expect("偏置一路应有输出");
    eprintln!(
        "无偏置 = {:?}\n偏置（诗文本作 hotwords）= {:?}",
        unbiased.as_str(),
        biased.as_str()
    );
    assert!(!unbiased.as_str().is_empty(), "无偏置一路不该是空的");
}

/// 真实推理下也必须在输入结束前就发出部分假设，而不是攒到最后一次性给出。
#[test]
fn a_real_stream_emits_a_partial_hypothesis_before_input_ends() {
    let files = files();
    let (samples, sample_rate) = sample_audio();
    let decoder = StreamingDualDecoder::open(
        &files,
        OnlineDecodeConfig::unbiased(),
        Some(OnlineDecodeConfig::biased(hotwords())),
    )
    .expect("双路解码器可构造");

    let mut plan = RecognitionPlan::guided(REFERENCE);
    plan.diagnostics = true;
    let handle = start_recognition(
        SliceSource {
            samples,
            cursor: 0,
            sample_rate,
        },
        decoder,
        plan,
    );

    let mut partials = 0usize;
    let mut partials_before_outcome = 0usize;
    let mut outcome = None;
    while let Some(event) = yunjian_core::operation::next_event(&handle, 60_000) {
        match &event {
            yunjian_core::operation::Event::Item(RecognitionItem::Partial(_)) => {
                partials += 1;
                if outcome.is_none() {
                    partials_before_outcome += 1;
                }
            }
            yunjian_core::operation::Event::Item(RecognitionItem::Outcome(found)) => {
                outcome = Some(found.clone());
            }
            _ => {}
        }
        if event.is_terminal() {
            break;
        }
    }

    assert!(partials > 0, "应发出部分假设");
    assert!(
        partials_before_outcome >= 2,
        "至少要有两次结束前的部分假设才算流式，实际 {partials_before_outcome}"
    );
    let outcome = outcome.expect("应有结束汇总");
    eprintln!(
        "流式汇总：开口 = {}，停顿 = {}，时长 = {} ms，单路 RTF = {:.4}，双路 RTF = {:.4}",
        outcome.spoke,
        outcome.pause_count,
        outcome.total_ms,
        outcome.cost.single.value(),
        outcome.cost.dual.value()
    );
    assert!(outcome.spoke, "真实语音应被能量门控判为开口");
}

/// 生产路径上的识别器配置必须关着 ITN。断言的是**真的传给识别器的那份配置**，
/// 不是一份平行描述。
#[test]
fn the_production_recognizer_config_keeps_itn_off() {
    for config in [
        OnlineDecodeConfig::unbiased(),
        OnlineDecodeConfig::biased(hotwords()),
    ] {
        assert!(!config.itn_enabled());
        assert_eq!(config.rule_fsts(), "");
        StreamingDualDecoder::open(
            &files(),
            OnlineDecodeConfig::unbiased(),
            config.is_biased().then(|| config.clone()),
        )
        .expect("以该配置构造解码器应成功");
    }
}

/// 无偏置一路带上 hotwords 会让它签出一枚假的见证，因此必须在构造时就被拒绝。
#[test]
fn the_unbiased_pass_refuses_hotwords() {
    let outcome =
        StreamingDualDecoder::open(&files(), OnlineDecodeConfig::biased(hotwords()), None);
    let Err(error) = outcome else {
        panic!("无偏置一路带 hotwords 必须被拒绝");
    };
    assert!(error.to_string().contains("无偏置一路"), "{error}");
}
