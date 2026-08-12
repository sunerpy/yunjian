//! 节奏的测试。
//!
//! **一条都不需要模型。** 合成抽象在 [`FootSynthesizer`] 之后，测试注入一个产出正弦的假
//! 合成器，于是「音步边界有没有留出 120 毫秒」这个问题在一台没有 sherpa-onnx、没有声卡、
//! 没有权重的机器上就能被回答——而这正是本任务真正要证明的东西：节奏来自拼接。

use std::time::Duration;

use super::{
    Foot, FootAudio, FootSynthesizer, Line, Prosody, SILENCE_FLOOR_DBFS, Segmentation, dbfs_to_rms,
    foot_widths, gap_between, segment_ci, segment_metrical, silent_spans, splice,
};
use crate::lexicon::{CiTuneRhythm, RhythmSource};

/// 假合成器：每个字给 60 毫秒满幅正弦。
///
/// 用正弦而不是常量 1.0，是因为静音判定走 RMS：常量信号无法暴露「逐样本比较绝对值会把
/// 每个过零点误判成静音」这个错误，而正弦可以。
struct Sine {
    sample_rate: u32,
    per_char: Duration,
    calls: Vec<String>,
}

impl Sine {
    fn new() -> Self {
        Self {
            sample_rate: 22_050,
            per_char: Duration::from_millis(60),
            calls: Vec::new(),
        }
    }
}

impl FootSynthesizer for Sine {
    fn synthesize_foot(&mut self, text: &str) -> Result<FootAudio, crate::VoiceError> {
        self.calls.push(text.to_owned());
        let chars = text.chars().count().max(1) as u32;
        let total = self.per_char.as_millis() as u32 * chars;
        let count = (self.sample_rate * total / 1_000) as usize;
        let step = 2.0 * std::f32::consts::PI * 440.0 / self.sample_rate as f32;
        Ok(FootAudio {
            samples: (0..count).map(|i| (i as f32 * step).sin() * 0.8).collect(),
            sample_rate: self.sample_rate,
        })
    }
}

/// 整行一次合成的合成器：它**不**在内部插任何静音，用来证明「不拼接就没有节奏」。
struct WholeLine(Sine);

impl FootSynthesizer for WholeLine {
    fn synthesize_foot(&mut self, text: &str) -> Result<FootAudio, crate::VoiceError> {
        self.0.synthesize_foot(text)
    }
}

// ---------------------------------------------------------------- 音步切分

#[test]
fn five_syllable_lines_split_two_three() {
    assert_eq!(foot_widths(5), vec![2, 3]);
}

#[test]
fn seven_syllable_lines_split_two_two_three() {
    assert_eq!(foot_widths(7), vec![2, 2, 3]);
}

/// 六言不切成二二二：那是把七言的直觉外推到没有依据的地方，宁可整句一个音步。
#[test]
fn other_lengths_are_not_split_on_a_guess() {
    assert_eq!(foot_widths(6), vec![6]);
    assert_eq!(foot_widths(4), vec![4]);
    assert_eq!(foot_widths(0), Vec::<usize>::new());
}

#[test]
fn a_seven_syllable_poem_yields_three_feet_per_line() {
    let seg = segment_metrical([
        "远上寒山石径斜",
        "白云生处有人家",
        "停车坐爱枫林晚",
        "霜叶红于二月花",
    ]);
    assert_eq!(seg.lines.len(), 4);
    assert_eq!(seg.foot_count(), 12);
    assert_eq!(
        seg.lines[0]
            .feet
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["远上", "寒山", "石径斜"]
    );
    assert!(
        seg.lines
            .iter()
            .all(|l| l.source == RhythmSource::CharCount)
    );
}

#[test]
fn a_five_syllable_poem_yields_two_feet_per_line() {
    let seg = segment_metrical(["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"]);
    assert_eq!(seg.foot_count(), 8);
    assert_eq!(
        seg.lines[3]
            .feet
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["低头", "思故乡"]
    );
}

/// 标点不算字：算进去会让七言变成八言，音步边界整体错位一个字。
#[test]
fn punctuation_does_not_count_toward_the_syllable_count() {
    let seg = segment_metrical(["远上寒山石径斜，"]);
    assert_eq!(seg.lines[0].feet.len(), 3);
}

#[test]
fn empty_lines_are_dropped() {
    let seg = segment_metrical(["床前明月光", "", "   ", "疑是地上霜"]);
    assert_eq!(seg.lines.len(), 2);
}

// ---------------------------------------------------------------- 词的切分

fn nian_nu_jiao(pattern: Vec<usize>, source: RhythmSource) -> CiTuneRhythm {
    CiTuneRhythm {
        tune: "念奴娇".to_owned(),
        pattern,
        source,
        evidence: "《全宋词》实测，n=135；据 chinese-poetry 锁定版转录本".to_owned(),
    }
}

/// **验收条目**：词牌不在句式表里，就按标点切分并报 `punctuation`，而不是静默宣称词谱权威。
#[test]
fn a_tune_absent_from_the_table_falls_back_to_punctuation() {
    let seg = segment_ci(&["大江东去", "浪淘尽", "千古风流人物"], None);
    assert_eq!(seg.lines.len(), 3);
    assert!(
        seg.lines
            .iter()
            .all(|line| line.source == RhythmSource::Punctuation),
        "缺词谱时必须报 punctuation"
    );
    assert!(seg.any_punctuation_fallback());
    assert_eq!(
        seg.lines
            .iter()
            .map(|line| line.feet.len())
            .collect::<Vec<_>>(),
        vec![1, 1, 1],
        "标点回落下每句就是一个音步，不再按字数细切"
    );
}

#[test]
fn a_matching_pattern_is_used_and_reports_its_source() {
    let rhythm = nian_nu_jiao(vec![4, 3, 6], RhythmSource::CorpusModal);
    let seg = segment_ci(&["大江东去", "浪淘尽", "千古风流人物"], Some(&rhythm));
    assert!(
        seg.lines
            .iter()
            .all(|line| line.source == RhythmSource::CorpusModal),
        "句数对得上就该用句式表，并如实报来源"
    );
    assert!(!seg.any_punctuation_fallback());
}

/// 句数不符说明这一首与表里那支词牌的体式不同（同名异体在词里极常见），此时用词谱会把
/// 停顿放错，所以必须退化——**实测里念奴娇的众数占比只有 43%**，这条守卫不是保守。
#[test]
fn a_clause_count_mismatch_degrades_instead_of_misplacing_pauses() {
    let rhythm = nian_nu_jiao(vec![4, 5, 4, 7, 6], RhythmSource::CorpusModal);
    let seg = segment_ci(&["大江东去", "浪淘尽", "千古风流人物"], Some(&rhythm));
    assert!(
        seg.lines
            .iter()
            .all(|line| line.source == RhythmSource::Punctuation),
        "句数不符时必须退化到 punctuation"
    );
}

/// 词里的五言句与诗的五言句同为二三：句长与实际字数一致时，句内仍按字数切。
#[test]
fn a_five_syllable_clause_in_a_ci_splits_two_three() {
    let rhythm = CiTuneRhythm {
        tune: "水调歌头".to_owned(),
        pattern: vec![5, 5],
        source: RhythmSource::CorpusModal,
        evidence: "《全宋词》实测，n=263；据 chinese-poetry 锁定版转录本".to_owned(),
    };
    let seg = segment_ci(&["明月几时有", "把酒问青天"], Some(&rhythm));
    assert_eq!(
        seg.lines[0]
            .feet
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["明月", "几时有"]
    );
}

// ---------------------------------------------------------------- 静音判定

#[test]
fn dbfs_converts_to_a_linear_threshold() {
    assert!((dbfs_to_rms(0.0) - 1.0).abs() < 1e-6);
    assert!(dbfs_to_rms(SILENCE_FLOOR_DBFS) < 0.006);
}

#[test]
fn all_zero_audio_is_one_silent_span() {
    let spans = silent_spans(&[0.0; 16_000], 16_000, SILENCE_FLOOR_DBFS);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].duration(16_000), Duration::from_secs(1));
}

/// 这条守的是「逐样本比绝对值」这个错法：正弦每个周期过零两次，逐样本判定会得到几千段
/// 长度为 1 的假静音，而 RMS 窗口判定得到零段。
#[test]
fn a_full_scale_sine_contains_no_silence() {
    let step = 2.0 * std::f32::consts::PI * 440.0 / 16_000.0;
    let wave: Vec<f32> = (0..16_000).map(|i| (i as f32 * step).sin() * 0.8).collect();
    assert!(
        silent_spans(&wave, 16_000, SILENCE_FLOOR_DBFS).is_empty(),
        "满幅正弦不该含静音"
    );
}

// ---------------------------------------------------------------- 拼接与时间戳

/// **验收条目**：七言的每个 二二三 音步边界至少 120 ms，行边界至少 400 ms。
///
/// 两个阈值取自配置（`[voice.prosody]`），断言的是「不短于配置值」，所以把 120 调成 150
/// 之后这条测试照样有效——测试的结构不随调参而改，这正是它们是配置项而非常量的理由。
#[test]
fn spliced_silence_meets_the_configured_pauses() {
    let prosody = Prosody::CLASSICAL;
    let seg = segment_metrical(["远上寒山石径斜", "白云生处有人家"]);
    let mut sine = Sine::new();
    let reading = splice(&mut sine, &seg, prosody).expect("拼接应成功");

    assert_eq!(reading.marks.len(), 6, "两行七言应有 6 个音步");

    // 行内边界：0-1、1-2 与 3-4、4-5。
    for (first, second) in [(0, 1), (1, 2), (3, 4), (4, 5)] {
        let gap = gap_between(&reading, first, second)
            .unwrap_or_else(|| panic!("音步 {first}→{second} 之间没有静音"));
        assert!(
            gap >= prosody.foot_pause(),
            "音步 {first}→{second} 只停了 {gap:?}，短于配置的 {:?}",
            prosody.foot_pause()
        );
    }

    // 行边界：2-3。
    let line_gap = gap_between(&reading, 2, 3).expect("行边界应有静音");
    assert!(
        line_gap >= prosody.line_pause(),
        "行边界只停了 {line_gap:?}，短于配置的 {:?}",
        prosody.line_pause()
    );
    assert!(
        line_gap > prosody.foot_pause(),
        "行边界不该和音步边界一样长，否则听不出行"
    );
}

/// **验收条目**：返回的时间戳数量等于音步数。
///
/// 这是 karaoke 高亮的硬契约，也是「逐段合成顺带给了我们时间戳」这句话的落点——少一个
/// 就会从那里起一路错位到句末。
#[test]
fn one_timestamp_per_foot() {
    for lines in [
        vec!["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"],
        vec!["远上寒山石径斜", "白云生处有人家"],
    ] {
        let seg = segment_metrical(lines.iter().copied());
        let mut sine = Sine::new();
        let reading = splice(&mut sine, &seg, Prosody::CLASSICAL).expect("拼接应成功");
        assert_eq!(reading.marks.len(), seg.foot_count());
    }
}

/// 时间戳必须由样本数算出，不能由播放墙钟测量：标记的样本区间要能对回缓冲区本身。
#[test]
fn timestamps_index_the_actual_buffer() {
    let seg = segment_metrical(["床前明月光"]);
    let mut sine = Sine::new();
    let reading = splice(&mut sine, &seg, Prosody::CLASSICAL).expect("拼接应成功");
    for mark in &reading.marks {
        assert!(mark.start_sample < mark.end_sample);
        assert!(mark.end_sample <= reading.samples.len());
        assert!(mark.end(reading.sample_rate) <= reading.duration());
    }
    assert!(
        reading.marks[0].start_sample == 0,
        "首音步应从第 0 个样本起，前面不该有静音"
    );
}

#[test]
fn each_foot_is_synthesized_separately() {
    let seg = segment_metrical(["远上寒山石径斜"]);
    let mut sine = Sine::new();
    splice(&mut sine, &seg, Prosody::CLASSICAL).expect("拼接应成功");
    assert_eq!(
        sine.calls,
        vec!["远上".to_owned(), "寒山".to_owned(), "石径斜".to_owned()],
        "必须逐音步调用合成器，而不是把整行喂进去"
    );
}

/// 这条是失败场景的常驻版本：**把整行当作一个音步合成，边界静音断言就不成立**。
///
/// 它证明拼接正是产出节奏的机制——不是「拼接是实现节奏的一种办法」，而是没有拼接就没有
/// 任何边界静音，因为引擎既无 SSML、`silence_scale` 也已报损。
#[test]
fn a_single_whole_line_synthesis_has_no_boundary_silence() {
    let whole = Segmentation {
        lines: vec![Line {
            feet: vec![Foot {
                text: "远上寒山石径斜".to_owned(),
                line: 0,
                index_in_line: 0,
            }],
            source: RhythmSource::CharCount,
        }],
    };
    let mut synth = WholeLine(Sine::new());
    let reading = splice(&mut synth, &whole, Prosody::CLASSICAL).expect("拼接应成功");
    assert_eq!(reading.marks.len(), 1, "整行一次合成只有一个音步");
    assert!(
        silent_spans(&reading.samples, reading.sample_rate, SILENCE_FLOOR_DBFS).is_empty(),
        "整行一次合成不该含任何静音；含了说明测试的假合成器自己插了静音，断言就失去意义"
    );
}

#[test]
fn zero_pauses_produce_a_gapless_buffer() {
    let seg = segment_metrical(["床前明月光"]);
    let mut sine = Sine::new();
    let reading = splice(
        &mut sine,
        &seg,
        Prosody {
            foot_pause_ms: 0,
            line_pause_ms: 0,
        },
    )
    .expect("拼接应成功");
    assert_eq!(reading.marks[0].end_sample, reading.marks[1].start_sample);
}

/// 采样率前后不一致不能默默拼上：那会变调。
#[test]
fn a_sample_rate_change_mid_reading_is_rejected() {
    struct Drifting(u32);
    impl FootSynthesizer for Drifting {
        fn synthesize_foot(&mut self, _text: &str) -> Result<FootAudio, crate::VoiceError> {
            self.0 += 1_000;
            Ok(FootAudio {
                samples: vec![0.5; 100],
                sample_rate: self.0,
            })
        }
    }
    let seg = segment_metrical(["床前明月光"]);
    let error =
        splice(&mut Drifting(16_000), &seg, Prosody::CLASSICAL).expect_err("采样率漂移应被拒绝");
    assert!(error.to_string().contains("采样率"), "{error}");
}

#[test]
fn default_prosody_matches_the_documented_values() {
    assert_eq!(Prosody::default().foot_pause_ms, 120);
    assert_eq!(Prosody::default().line_pause_ms, 400);
}
