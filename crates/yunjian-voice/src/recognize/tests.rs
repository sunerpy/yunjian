//! 判定层的测试。**不开任何特性也全部执行**，因此 `make ci` 覆盖它们。

use super::*;

const REFERENCE: &str = "床前明月光，疑是地上霜。";

fn loud() -> f32 {
    crate::prosody::dbfs_to_rms(SPEECH_FLOOR_DBFS) * 4.0
}

fn quiet() -> f32 {
    crate::prosody::dbfs_to_rms(SPEECH_FLOOR_DBFS) / 4.0
}

#[test]
fn a_biased_config_cannot_produce_an_unbiased_hypothesis() {
    let hotwords = Hotwords::from_poem(REFERENCE).expect("诗文可展开为 hotwords");
    let biased = OnlineDecodeConfig::biased(hotwords);

    assert!(
        UnbiasedAsrHyp::from_pass(&biased, "床前明月光").is_none(),
        "偏置一路不得签出无偏置见证"
    );
    assert!(UnbiasedAsrHyp::from_pass(&OnlineDecodeConfig::unbiased(), "床前").is_some());
}

#[test]
fn an_unbiased_config_cannot_produce_a_display_hypothesis() {
    let unbiased = OnlineDecodeConfig::unbiased();
    assert!(
        biased_hyp(&unbiased, "床前明月光").is_none(),
        "无偏置输出不得被标成偏置输出，否则高亮会把没有对齐依据的文本当成对齐结果"
    );

    let hotwords = Hotwords::from_poem(REFERENCE).expect("诗文可展开为 hotwords");
    let biased = OnlineDecodeConfig::biased(hotwords);
    assert_eq!(
        biased_hyp(&biased, "床前明月光")
            .expect("偏置一路应产出展示假设")
            .as_str(),
        "床前明月光"
    );
}

/// 无偏置见证的构造点只能在本模块。这条 grep 守卫覆盖整个 `crates/`，因此新增一个
/// crate 也逃不掉；被查找的字面量用 `concat!` 拼出来，否则本文件自己就会命中。
#[test]
fn decode_witness_is_constructed_in_exactly_one_file() {
    let crates = workspace_root().join("crates");
    let hits: Vec<_> = rust_sources(&crates)
        .into_iter()
        .filter(|(_, source)| source.contains(concat!("DecodeWitness", "::new")))
        .map(|(path, _)| path)
        .collect();

    assert_eq!(hits.len(), 1, "构造点必须唯一，实际命中：{hits:?}");
    assert!(
        hits[0].ends_with("recognize.rs"),
        "唯一构造点必须是无偏置解码路径所在的文件：{:?}",
        hits[0]
    );
}

#[test]
fn itn_is_disabled_and_has_no_way_to_be_enabled() {
    for config in [
        OnlineDecodeConfig::unbiased(),
        OnlineDecodeConfig::biased(Hotwords::from_poem(REFERENCE).expect("hotwords")),
    ] {
        assert_eq!(config.itn(), ItnPolicy::Disabled);
        assert!(!config.itn_enabled());
        assert_eq!(config.rule_fsts(), "");
        config.validate().expect("默认配置应自洽");
    }
}

#[test]
fn hotwords_on_greedy_search_is_rejected_instead_of_silently_ignored() {
    let mut config = OnlineDecodeConfig::biased(Hotwords::from_poem(REFERENCE).expect("hotwords"));
    config.decoding_method = DecodingMethod::GreedySearch;

    let error = config.validate().expect_err("应拒绝");
    assert!(
        error.to_string().contains("modified_beam_search"),
        "{error}"
    );
}

#[test]
fn hotwords_are_split_per_character_and_per_line() {
    let hotwords = Hotwords::from_poem(REFERENCE).expect("hotwords");

    assert_eq!(hotwords.len(), 2);
    assert_eq!(hotwords.buffer(), "床 前 明 月 光\n疑 是 地 上 霜\n");
    assert!(Hotwords::from_poem("，。").is_none());
}

#[test]
fn the_modeling_unit_is_cjkchar_because_hotwords_are_per_character() {
    assert_eq!(ModelingUnit::default().as_str(), "cjkchar");
    assert_eq!(
        OnlineDecodeConfig::unbiased().modeling_unit,
        ModelingUnit::CjkChar
    );
}

#[test]
fn the_homophone_replacer_is_off_by_absence_not_by_preference() {
    let dir = temp_dir("hr-absent");
    assert!(HomophoneReplacer::discover(&dir).is_none());

    std::fs::create_dir_all(dir.join("dict")).expect("建 dict 目录");
    std::fs::write(dir.join("lexicon.txt"), "床 chuang2\n").expect("写 lexicon");
    assert!(
        HomophoneReplacer::discover(&dir).is_none(),
        "缺 replace.fst 时不得报告可用"
    );

    std::fs::write(dir.join("replace.fst"), b"\x00").expect("写 fst");
    let found = HomophoneReplacer::discover(&dir).expect("三项齐备时应可用");
    assert_eq!(found.rule_fsts, dir.join("replace.fst"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pause_after_four_characters_prompts_the_fifth_exactly_once() {
    let mut detector = StuckDetector::new(
        REFERENCE,
        StuckConfig::default(),
        SpeechGateConfig::default(),
    );
    detector.advance(4);

    for _ in 0..5 {
        assert!(detector.observe(loud(), 100).is_none(), "说话中不得提示");
    }

    let mut prompts = Vec::new();
    for _ in 0..40 {
        if let Some(prompt) = detector.observe(quiet(), 100) {
            prompts.push(prompt);
        }
    }

    assert_eq!(prompts.len(), 1, "同一游标位置只提示一次：{prompts:?}");
    let prompt = &prompts[0];
    assert_eq!(prompt.from_index, 4);
    assert_eq!(prompt.next_chars.chars().next(), Some('光'));
    assert_eq!(prompt.reason, PromptReason::TrailingSilence);
    assert!(
        prompt.at_ms >= u64::from(DEFAULT_TRAILING_SILENCE_MS),
        "提示不得早于尾静音门槛：{}",
        prompt.at_ms
    );
}

#[test]
fn silence_shorter_than_the_threshold_does_not_prompt() {
    let mut detector = StuckDetector::new(
        REFERENCE,
        StuckConfig::default(),
        SpeechGateConfig::default(),
    );
    detector.advance(4);
    detector.observe(loud(), 100);

    for _ in 0..19 {
        assert!(detector.observe(quiet(), 100).is_none());
    }
    assert!(
        detector.observe(quiet(), 100).is_some(),
        "第 2000 毫秒应提示"
    );
}

#[test]
fn never_speaking_prompts_from_the_session_cursor() {
    let mut detector = StuckDetector::new(
        REFERENCE,
        StuckConfig::default(),
        SpeechGateConfig::default(),
    );

    let mut prompt = None;
    for _ in 0..30 {
        if let Some(found) = detector.observe(quiet(), 100) {
            prompt = Some(found);
            break;
        }
    }
    let prompt = prompt.expect("一直没开口应提示");
    assert_eq!(prompt.reason, PromptReason::NoSpeechYet);
    assert_eq!(prompt.from_index, 0);
    assert_eq!(prompt.next_chars, "床前");
}

#[test]
fn advancing_the_cursor_allows_a_second_prompt() {
    let mut detector = StuckDetector::new(
        REFERENCE,
        StuckConfig::default(),
        SpeechGateConfig::default(),
    );
    detector.advance(4);
    for _ in 0..25 {
        detector.observe(quiet(), 100);
    }
    detector.advance(6);

    let mut second = None;
    for _ in 0..25 {
        if let Some(prompt) = detector.observe(quiet(), 100) {
            second = Some(prompt);
            break;
        }
    }
    assert_eq!(second.expect("游标推进后应可再提示").from_index, 6);
}

#[test]
fn the_gate_counts_pauses_and_records_speech() {
    let mut gate = SpeechGate::new(SpeechGateConfig::default());
    assert!(!gate.spoke());

    gate.observe(loud(), 100);
    assert!(gate.spoke());
    for _ in 0..5 {
        gate.observe(quiet(), 100);
    }
    gate.observe(loud(), 100);
    for _ in 0..5 {
        gate.observe(quiet(), 100);
    }

    assert_eq!(gate.pause_count(), 2);
    assert_eq!(gate.elapsed_ms(), 1200);
}

#[test]
fn the_gate_derives_frame_length_from_the_sample_rate() {
    let mut gate = SpeechGate::new(SpeechGateConfig::default());
    assert_eq!(
        gate.observe_samples(&[0.0; 1600], 16_000),
        Activity::Silence
    );
    assert_eq!(gate.elapsed_ms(), 100);
}

#[test]
fn a_dual_pass_slower_than_realtime_drops_to_a_single_pass_instead_of_dropping_frames() {
    let cost = DualDecodeCost {
        single: Rtf::measure(Duration::from_secs(4), Duration::from_secs(2)),
        dual: Rtf::measure(Duration::from_secs(4), Duration::from_secs(5)),
    };

    assert!(!cost.single.exceeds_realtime());
    assert!(cost.dual.exceeds_realtime());
    let plan = plan_for(cost);
    assert_eq!(
        plan,
        DecodePlan::SingleUnbiased {
            degradation: Degradation::HighlightingDisabled,
        }
    );
    assert!(!plan.runs_biased());
    assert!(!plan.highlighting());
}

#[test]
fn a_dual_pass_within_budget_keeps_highlighting() {
    let cost = DualDecodeCost {
        single: Rtf::measure(Duration::from_secs(4), Duration::from_millis(800)),
        dual: Rtf::measure(Duration::from_secs(4), Duration::from_millis(2400)),
    };

    assert_eq!(plan_for(cost), DecodePlan::Dual);
    assert!(plan_for(cost).highlighting());
    assert!((cost.dual.value() - 0.6).abs() < 1e-6);
}

#[test]
fn a_zero_length_audio_never_looks_faster_than_realtime() {
    assert!(Rtf::measure(Duration::ZERO, Duration::from_millis(1)).exceeds_realtime());
}

// ---------------------------------------------------------------------------
// 事件驱动
// ---------------------------------------------------------------------------

struct FrameSource {
    frames: std::vec::IntoIter<Vec<f32>>,
    sample_rate: u32,
}

impl FrameSource {
    fn new(frames: Vec<Vec<f32>>, sample_rate: u32) -> Self {
        Self {
            frames: frames.into_iter(),
            sample_rate,
        }
    }
}

impl PcmSource for FrameSource {
    fn next_frame(&mut self) -> Option<Vec<f32>> {
        self.frames.next()
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// 假解码器。它逐帧多吐一个字，因此「话语结束前是否发出过部分假设」可判定。
struct FakeDecoder {
    reference: Vec<char>,
    emitted: usize,
    unbiased: OnlineDecodeConfig,
    biased: OnlineDecodeConfig,
    cost: DualDecodeCost,
}

impl FakeDecoder {
    fn new(reference: &str, cost: DualDecodeCost) -> Self {
        let hotwords = Hotwords::from_poem(reference).expect("hotwords");
        Self {
            reference: reference.chars().filter(|c| c.is_alphabetic()).collect(),
            emitted: 0,
            unbiased: OnlineDecodeConfig::unbiased(),
            biased: OnlineDecodeConfig::biased(hotwords),
            cost,
        }
    }

    fn snapshot(&self, at_ms: u64) -> PartialHypothesis {
        let text: String = self.reference[..self.emitted].iter().collect();
        PartialHypothesis {
            at_ms,
            unbiased: UnbiasedAsrHyp::from_pass(&self.unbiased, text.clone()),
            biased: biased_hyp(&self.biased, text),
        }
    }
}

impl DualDecode for FakeDecoder {
    fn accept(
        &mut self,
        _samples: &[f32],
        _sample_rate: u32,
    ) -> Result<PartialHypothesis, VoiceError> {
        self.emitted = (self.emitted + 1).min(self.reference.len());
        Ok(self.snapshot(0))
    }

    fn finish(&mut self) -> Result<PartialHypothesis, VoiceError> {
        Ok(self.snapshot(0))
    }

    fn cost(&self) -> DualDecodeCost {
        self.cost
    }
}

fn cheap_cost() -> DualDecodeCost {
    DualDecodeCost {
        single: Rtf::measure(Duration::from_secs(1), Duration::from_millis(100)),
        dual: Rtf::measure(Duration::from_secs(1), Duration::from_millis(300)),
    }
}

fn drain(
    handle: &OperationHandle<RecognitionProgress, RecognitionItem>,
) -> Vec<yunjian_core::operation::Event<RecognitionProgress, RecognitionItem>> {
    let mut events = Vec::new();
    while let Some(event) = yunjian_core::operation::next_event(handle, 5_000) {
        let terminal = event.is_terminal();
        events.push(event);
        if terminal {
            break;
        }
    }
    events
}

#[test]
fn a_partial_hypothesis_is_emitted_before_the_utterance_ends() {
    let frames = vec![vec![loud(); 1600]; 6];
    let mut plan = RecognitionPlan::guided(REFERENCE);
    plan.diagnostics = true;
    let handle = start_recognition(
        FrameSource::new(frames, 16_000),
        FakeDecoder::new(REFERENCE, cheap_cost()),
        plan,
    );

    let events = drain(&handle);
    let mut first_partial = None;
    let mut outcome_position = None;
    for (position, event) in events.iter().enumerate() {
        match event {
            yunjian_core::operation::Event::Item(RecognitionItem::Partial(hypothesis)) => {
                if first_partial.is_none() {
                    first_partial = Some((position, hypothesis.clone()));
                }
            }
            yunjian_core::operation::Event::Item(RecognitionItem::Outcome(_)) => {
                outcome_position = Some(position);
            }
            _ => {}
        }
    }

    let (partial_position, partial) = first_partial.expect("应在结束前发出部分假设");
    let outcome_position = outcome_position.expect("应有结束汇总");
    assert!(
        partial_position < outcome_position,
        "部分假设必须早于结束汇总：{partial_position} vs {outcome_position}"
    );
    assert_eq!(partial.unbiased.expect("无偏置一路").as_str(), "床");
    assert!(partial.biased.is_some(), "偏置一路应可用于展示");
}

#[test]
fn diagnostics_are_off_by_default_so_no_hypothesis_reaches_the_caller() {
    let frames = vec![vec![loud(); 1600]; 4];
    let handle = start_recognition(
        FrameSource::new(frames, 16_000),
        FakeDecoder::new(REFERENCE, cheap_cost()),
        RecognitionPlan::guided(REFERENCE),
    );

    let partials = drain(&handle)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                yunjian_core::operation::Event::Item(RecognitionItem::Partial(_))
            )
        })
        .count();
    assert_eq!(partials, 0, "77% CER 下假设默认不面向调用方");
}

#[test]
fn the_stream_reports_both_measured_rtf_values_in_its_outcome() {
    let frames = vec![vec![loud(); 1600]; 3];
    let cost = DualDecodeCost {
        single: Rtf::measure(Duration::from_secs(2), Duration::from_millis(500)),
        dual: Rtf::measure(Duration::from_secs(2), Duration::from_millis(1500)),
    };
    let handle = start_recognition(
        FrameSource::new(frames, 16_000),
        FakeDecoder::new(REFERENCE, cost),
        RecognitionPlan::guided(REFERENCE),
    );

    let outcome = drain(&handle)
        .into_iter()
        .find_map(|event| match event {
            yunjian_core::operation::Event::Item(RecognitionItem::Outcome(outcome)) => {
                Some(outcome)
            }
            _ => None,
        })
        .expect("应有结束汇总");

    assert!((outcome.cost.single.value() - 0.25).abs() < 1e-6);
    assert!((outcome.cost.dual.value() - 0.75).abs() < 1e-6);
    assert_eq!(outcome.plan, DecodePlan::Dual);
    assert!(outcome.spoke);
}

#[test]
fn a_silent_take_emits_one_prompt_and_reports_no_speech() {
    let frames = vec![vec![quiet(); 1600]; 30];
    let handle = start_recognition(
        FrameSource::new(frames, 16_000),
        FakeDecoder::new(REFERENCE, cheap_cost()),
        RecognitionPlan::guided(REFERENCE),
    );

    let events = drain(&handle);
    let prompts: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            yunjian_core::operation::Event::Item(RecognitionItem::Prompt(prompt)) => Some(prompt),
            _ => None,
        })
        .collect();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].reason, PromptReason::NoSpeechYet);

    let outcome = events
        .into_iter()
        .find_map(|event| match event {
            yunjian_core::operation::Event::Item(RecognitionItem::Outcome(outcome)) => {
                Some(outcome)
            }
            _ => None,
        })
        .expect("应有结束汇总");
    assert!(!outcome.spoke);
    assert_eq!(outcome.prompt_count, 1);
}

#[test]
fn cancelling_stops_the_stream_without_an_outcome() {
    let frames = vec![vec![loud(); 1600]; 4096];
    let handle = start_recognition(
        FrameSource::new(frames, 16_000),
        FakeDecoder::new(REFERENCE, cheap_cost()),
        RecognitionPlan::guided(REFERENCE),
    );
    yunjian_core::operation::cancel(&handle);

    let events = drain(&handle);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, yunjian_core::operation::Event::Cancelled)),
        "取消后应收到 Cancelled"
    );
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("定位 workspace root")
        .to_path_buf()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yunjian-recognize-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn rust_sources(root: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    if !root.exists() {
        return Vec::new();
    }
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).expect("读取源码目录") {
                pending.push(entry.expect("读取源码条目").path());
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push((
                path.clone(),
                std::fs::read_to_string(&path).expect("读取 Rust 源码"),
            ));
        }
    }
    sources
}
