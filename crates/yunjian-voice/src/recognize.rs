//! 流式识别的判定层：双路解码的类型隔离、卡顿判定、双路成本策略与事件驱动。
//!
//! **判定层不带特性开关。** 双路解码的类型边界、卡顿判定的时序算术、Prompt 的生成、
//! 识别器配置的取值都是纯逻辑，把它们放在 `#[cfg(feature = "voice")]` 后面会让一台
//! 没有模型、没有麦克风的机器无法验证它们，而这几件事恰恰是最需要被守住的。真实推理
//! 藏在 [`DualDecode`] 之后，实现在 [`crate::asr::streaming`]。
//!
//! # 为什么必须双路解码
//!
//! 把诗文本本身当 hotwords 去偏置识别器，会让它吐出用户根本没说的字。那既会虚高任何
//! 基于转写的指标，又会**掩盖漏读**——用户跳过一整句，偏置解码仍可能把那句补全。因此
//! 两路解码在类型上不可互换：
//!
//! - **无偏置**一路包成 [`UnbiasedAsrHyp`]，它携带一枚 [`DecodeWitness`]，而那枚见证
//!   **只能**由本文件里唯一的构造点签发，且只在配置确实没有 hotwords 时签发。
//! - **偏置**一路是 [`yunjian_recite::BiasedHyp`]，复用背诵内核里那个已经被 `trybuild`
//!   守住「不得进入打字评分」的类型，而不是在这里另造一个同名类型。
//!
//! # 2026-08-11 裁决对本模块的约束
//!
//! 文言 ASR 的 CER 实测 77.01%（`docs/reports/asr-cer.json`，TTS 合成音的乐观上界）。
//! 因此裁决把 v1 语音定为**跟读**：反馈只含「是否开口 / 停顿 / 相对节奏」，FSRS 等级由
//! **用户自选**。落到本模块上是三条硬约束：
//!
//! 1. **两路假设都只是诊断信号**，谁都不能进入评分。无偏置那一路只能证明「没有 hotword
//!    偏置」，证明不了「转写可靠」；偏置那一路在 77% CER 下面向用户只会制造错误反馈。
//!    所以 [`PartialHypothesis`] 默认不发出，要显式打开 [`RecognitionPlan::diagnostics`]。
//! 2. **卡顿判定不看识别假设**。触发条件是「尚未开口」或「语音之后持续静音」，纯由能量
//!    门控得出；[`StuckDetector::advance`] 只接受会话游标，刻意不接受任何假设文本。
//! 3. **不做逐字时间戳依赖，也不做 forced alignment**。上游明确不做后者（sherpa-onnx
//!    #3536），且只暴露 token 的 start 时间而没有 stop（#985）。

use std::time::Duration;

use yunjian_core::operation::{OperationHandle, OperationReporter, start_operation};
use yunjian_recite::BiasedHyp;

use crate::VoiceError;

/// 参考产品 `vad_speech_tail` 的默认值：语音之后持续这么久的静音就认为卡住了。
pub const DEFAULT_TRAILING_SILENCE_MS: u32 = 2000;

/// 一开始就没开口时等多久再提示。与尾静音同值，理由是两者对用户是同一种体验
/// （「我卡住了」），没有把它们分开调的依据。
pub const DEFAULT_NO_SPEECH_MS: u32 = 2000;

/// 判定静音的电平门槛，与朗读侧共用一个尺度。
pub const SPEECH_FLOOR_DBFS: f32 = crate::prosody::SILENCE_FLOOR_DBFS;

/// 双路解码在参考设备上的实时率上限。超过它就降为单路无偏置解码，**而不是丢帧**。
pub const REALTIME_BUDGET: f32 = 1.0;

// ---------------------------------------------------------------------------
// 类型隔离
// ---------------------------------------------------------------------------

/// 「这段转写来自没有 hotwords 的解码」的见证。
///
/// **构造点全工作区唯一，就在本文件**，且不接受调用方的自我声明：它从
/// [`OnlineDecodeConfig`] 推出结论，配置里带 hotwords 就签不出来。字段私有且是
/// `()`，所以外部既造不出也仿不出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeWitness {
    _seal: (),
}

impl DecodeWitness {
    /// 唯一构造点。只有配置确实没有 hotwords 时才签发。
    fn new(config: &OnlineDecodeConfig) -> Option<Self> {
        config.hotwords.is_none().then_some(Self { _seal: () })
    }
}

/// 无偏置解码的转写。
///
/// **它不是评分输入。** 2026-08-11 裁决作废了「无偏置即可进入评分」这条语义：见证只能
/// 证明没有 hotword 偏置，证明不了 77% CER 下的转写可靠。它的用途是开发诊断，以及日后
/// KWS spike 通过冻结门槛后可能开放的 `coverage_advisory`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnbiasedAsrHyp {
    text: String,
    witness: DecodeWitness,
}

impl UnbiasedAsrHyp {
    /// 从一路解码的配置与它的输出构造。
    ///
    /// 配置里带 hotwords 时返回 `None`——这正是「偏置输出无法伪装成无偏置输出」的地方。
    #[must_use]
    pub fn from_pass(config: &OnlineDecodeConfig, text: impl Into<String>) -> Option<Self> {
        DecodeWitness::new(config).map(|witness| Self {
            text: text.into(),
            witness,
        })
    }

    /// 转写文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 无偏置见证。
    #[must_use]
    pub const fn witness(&self) -> DecodeWitness {
        self.witness
    }
}

/// 从偏置一路的配置与它的输出构造展示用假设。
///
/// 配置里**没有** hotwords 时返回 `None`：把无偏置输出标成偏置输出同样是错标，
/// 会让高亮把一段没有对齐依据的文本当成对齐结果。
#[must_use]
pub fn biased_hyp(config: &OnlineDecodeConfig, text: impl Into<String>) -> Option<BiasedHyp> {
    config
        .hotwords
        .is_some()
        .then(|| BiasedHyp::new(text.into()))
}

// ---------------------------------------------------------------------------
// 识别器配置
// ---------------------------------------------------------------------------

/// 逆文本归一化策略。**只有一个取值。**
///
/// 做成枚举而不是 `bool` 是为了让「打开 ITN」这件事在类型层面无从表达：ITN 会把
/// 识别出的数字与单位改写成阿拉伯数字与符号（「三千」→「3000」），而背诵比对的是
/// 诗的原字形，任何改写都直接制造假的替换与漏读。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ItnPolicy {
    /// 关闭。
    #[default]
    Disabled,
}

/// 识别器的建模单元。**只有一个取值。**
///
/// hotwords 要按汉字逐字编码才能匹配诗句，`cjkchar` 是唯一正确的选择；换成 BPE
/// 会让 hotwords 静默失效而不是报错。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelingUnit {
    /// 逐汉字。
    #[default]
    CjkChar,
}

impl ModelingUnit {
    /// 传给 sherpa-onnx 的字面量。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CjkChar => "cjkchar",
        }
    }
}

/// 解码搜索方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DecodingMethod {
    /// 贪心。无偏置一路够用，也最快。
    #[default]
    GreedySearch,
    /// 修改版 beam search。**hotwords 只在这种方式下生效。**
    ModifiedBeamSearch,
}

impl DecodingMethod {
    /// 传给 sherpa-onnx 的字面量。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GreedySearch => "greedy_search",
            Self::ModifiedBeamSearch => "modified_beam_search",
        }
    }
}

/// 端点检测规则。三条都是「尾随静音超过多少秒」的阈值，单位是秒，因为 C API 就是秒。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EndpointRules {
    /// 什么都还没解出来时的尾静音阈值。
    pub min_trailing_silence_before_speech: f32,
    /// 已经解出非空白之后的尾静音阈值。
    pub min_trailing_silence_after_speech: f32,
    /// 单条话语的最大时长。
    pub max_utterance_length: f32,
}

impl Default for EndpointRules {
    fn default() -> Self {
        Self {
            min_trailing_silence_before_speech: 2.4,
            min_trailing_silence_after_speech: 1.2,
            max_utterance_length: 20.0,
        }
    }
}

/// 同音字替换器的数据位置。
///
/// **不是可选的锦上添花**：文言里同音异形极多，识别时纠正能把「一骑」误成「一其」
/// 这类错误在进入任何下游之前消掉。但它需要一份独立的数据包，而那份数据包目前不在
/// `models.toml` 里（许可未核实），因此 [`Self::discover`] 找不到就返回 `None`，
/// 配置里对应字段留空——这是**因缺失而关闭**，不是因偏好而关闭。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomophoneReplacer {
    /// jieba 词典目录。
    pub dict_dir: std::path::PathBuf,
    /// 词典文件。
    pub lexicon: std::path::PathBuf,
    /// 替换规则 FST。
    pub rule_fsts: std::path::PathBuf,
}

impl HomophoneReplacer {
    /// 从模型目录里发现同音字替换器数据；三项缺任何一项即视为不可用。
    #[must_use]
    pub fn discover(model_dir: &std::path::Path) -> Option<Self> {
        let candidate = Self {
            dict_dir: model_dir.join("dict"),
            lexicon: model_dir.join("lexicon.txt"),
            rule_fsts: model_dir.join("replace.fst"),
        };
        (candidate.dict_dir.is_dir()
            && candidate.lexicon.is_file()
            && candidate.rule_fsts.is_file())
        .then_some(candidate)
    }
}

/// 一首诗展开成的 hotwords。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotwords {
    lines: Vec<String>,
}

impl Hotwords {
    /// 按标点切句，每句一条 hotword，句内逐字空格分隔。
    ///
    /// 逐字分隔是 `cjkchar` 建模单元的要求；整句不分隔会被当成一个未知 token 而静默失效。
    /// 全诗一条也不成立：那会把「必须整首连读」变成匹配前提。
    #[must_use]
    pub fn from_poem(text: &str) -> Option<Self> {
        let lines: Vec<String> = text
            .split(|c: char| !c.is_alphabetic())
            .filter(|segment| !segment.is_empty())
            .map(|segment| {
                segment
                    .chars()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        (!lines.is_empty()).then_some(Self { lines })
    }

    /// 传给 sherpa-onnx 的 hotwords 缓冲区内容。
    #[must_use]
    pub fn buffer(&self) -> String {
        let mut buffer = self.lines.join("\n");
        buffer.push('\n');
        buffer
    }

    /// hotword 条数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// 是否没有任何 hotword。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// 一路在线解码的配置。**真实识别器就是从这个结构建出来的**，因此对它的断言不是
/// 对一份平行描述的断言。
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineDecodeConfig {
    /// 解码搜索方式。
    pub decoding_method: DecodingMethod,
    /// beam 宽度，仅 [`DecodingMethod::ModifiedBeamSearch`] 使用。
    pub max_active_paths: i32,
    /// 建模单元。
    pub modeling_unit: ModelingUnit,
    /// hotwords。`None` 即无偏置一路。
    pub hotwords: Option<Hotwords>,
    /// 每个 hotword token 的加分。
    pub hotwords_score: f32,
    /// 端点检测规则。
    pub endpoint: EndpointRules,
    /// 同音字替换器；缺数据即 `None`。
    pub homophone_replacer: Option<HomophoneReplacer>,
    /// onnxruntime 线程数。
    pub num_threads: i32,
}

impl Default for OnlineDecodeConfig {
    fn default() -> Self {
        Self {
            decoding_method: DecodingMethod::GreedySearch,
            max_active_paths: 4,
            modeling_unit: ModelingUnit::CjkChar,
            hotwords: None,
            hotwords_score: 1.5,
            endpoint: EndpointRules::default(),
            homophone_replacer: None,
            num_threads: 1,
        }
    }
}

impl OnlineDecodeConfig {
    /// 无偏置一路。
    #[must_use]
    pub fn unbiased() -> Self {
        Self::default()
    }

    /// 偏置一路：诗文本作 hotwords，逐字建模，并切到 beam search。
    #[must_use]
    pub fn biased(hotwords: Hotwords) -> Self {
        Self {
            decoding_method: DecodingMethod::ModifiedBeamSearch,
            hotwords: Some(hotwords),
            ..Self::default()
        }
    }

    /// ITN 策略。**恒为关闭**，且没有任何字段能改变它。
    #[must_use]
    pub const fn itn(&self) -> ItnPolicy {
        ItnPolicy::Disabled
    }

    /// 是否开启了 ITN。**恒为 `false`**，因为 [`ItnPolicy`] 只有一个取值，
    /// 而 [`Self::rule_fsts`] 恒为空串。
    #[must_use]
    pub const fn itn_enabled(&self) -> bool {
        false
    }

    /// 传给 sherpa-onnx 的 `rule_fsts`。**恒为空串**：那个字段正是 ITN 的入口。
    #[must_use]
    pub const fn rule_fsts(&self) -> &'static str {
        ""
    }

    /// 这一路是否带偏置。
    #[must_use]
    pub const fn is_biased(&self) -> bool {
        self.hotwords.is_some()
    }

    /// 校验配置自洽。
    ///
    /// # Errors
    ///
    /// hotwords 配在贪心解码上时报 [`VoiceError::RecognizerConfig`]——那种组合不会报错，
    /// 只会让偏置静默失效，而静默失效的偏置路径看起来就像「模型不认识这首诗」。
    pub fn validate(&self) -> Result<(), VoiceError> {
        if self.hotwords.is_some() && self.decoding_method != DecodingMethod::ModifiedBeamSearch {
            return Err(VoiceError::RecognizerConfig {
                detail: format!(
                    "hotwords 只在 modified_beam_search 下生效，当前是 {}",
                    self.decoding_method.as_str()
                ),
            });
        }
        if self.itn_enabled() {
            return Err(VoiceError::RecognizerConfig {
                detail: "ITN 必须保持关闭：它会改写数字与单位，制造假的替换与漏读".to_owned(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 能量门控与卡顿判定
// ---------------------------------------------------------------------------

/// 一帧音频被判为什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// 低于门槛。
    Silence,
    /// 高于门槛。
    Speech,
}

/// 能量门控参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechGateConfig {
    /// 判为语音的电平门槛，dBFS。
    pub speech_floor_dbfs: f32,
    /// 计入「一次停顿」的最短静音时长，毫秒。
    pub pause_ms: u32,
}

impl Default for SpeechGateConfig {
    fn default() -> Self {
        Self {
            speech_floor_dbfs: SPEECH_FLOOR_DBFS,
            pause_ms: 300,
        }
    }
}

/// 逐帧能量门控。
///
/// 刻意只吃 RMS 与帧长而不吃采样缓冲：这样时序判定能用合成数字直接测，不需要造音频。
#[derive(Debug, Clone)]
pub struct SpeechGate {
    config: SpeechGateConfig,
    floor_rms: f32,
    elapsed_ms: u64,
    spoke: bool,
    silence_run_ms: u32,
    pause_count: usize,
    counted_current_pause: bool,
}

impl SpeechGate {
    /// 按参数构造。
    #[must_use]
    pub fn new(config: SpeechGateConfig) -> Self {
        Self {
            floor_rms: crate::prosody::dbfs_to_rms(config.speech_floor_dbfs),
            config,
            elapsed_ms: 0,
            spoke: false,
            silence_run_ms: 0,
            pause_count: 0,
            counted_current_pause: false,
        }
    }

    /// 观察一帧的 RMS。
    pub fn observe(&mut self, rms: f32, frame_ms: u32) -> Activity {
        self.elapsed_ms += u64::from(frame_ms);
        if rms >= self.floor_rms {
            self.spoke = true;
            self.silence_run_ms = 0;
            self.counted_current_pause = false;
            return Activity::Speech;
        }
        self.silence_run_ms = self.silence_run_ms.saturating_add(frame_ms);
        if self.spoke && !self.counted_current_pause && self.silence_run_ms >= self.config.pause_ms
        {
            self.pause_count += 1;
            self.counted_current_pause = true;
        }
        Activity::Silence
    }

    /// 观察一段采样。
    pub fn observe_samples(&mut self, samples: &[f32], sample_rate: u32) -> Activity {
        let frame_ms = if sample_rate == 0 {
            0
        } else {
            u32::try_from(samples.len() as u64 * 1000 / u64::from(sample_rate)).unwrap_or(u32::MAX)
        };
        self.observe(crate::rms(samples), frame_ms)
    }

    /// 至今是否检测到过语音。
    #[must_use]
    pub const fn spoke(&self) -> bool {
        self.spoke
    }

    /// 计入的停顿次数。
    #[must_use]
    pub const fn pause_count(&self) -> usize {
        self.pause_count
    }

    /// 当前连续静音时长，毫秒。
    #[must_use]
    pub const fn silence_run_ms(&self) -> u32 {
        self.silence_run_ms
    }

    /// 已观察的总时长，毫秒。
    #[must_use]
    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }
}

/// 卡顿提示的触发原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptReason {
    /// 一直没开口。
    NoSpeechYet,
    /// 开口之后持续静音。
    TrailingSilence,
}

/// 一次卡顿提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// 提示的下几个字。
    pub next_chars: String,
    /// 这些字在参考文本里的起始下标（按字计，不含标点）。
    pub from_index: usize,
    /// 提示发出的时刻，毫秒。
    pub at_ms: u64,
    /// 触发原因。
    pub reason: PromptReason,
}

/// 卡顿判定参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StuckConfig {
    /// 一直没开口时的等待时长，毫秒。
    pub no_speech_ms: u32,
    /// 开口后持续静音的等待时长，毫秒。
    pub trailing_silence_ms: u32,
    /// 一次提示给几个字。
    pub prompt_chars: usize,
}

impl Default for StuckConfig {
    fn default() -> Self {
        Self {
            no_speech_ms: DEFAULT_NO_SPEECH_MS,
            trailing_silence_ms: DEFAULT_TRAILING_SILENCE_MS,
            prompt_chars: 2,
        }
    }
}

/// 卡顿判定器。
///
/// **不接受识别假设。** 触发只看能量门控给出的静音时长，位置只看会话游标——CER 77% 下
/// partial 的 matched prefix 是噪声，用它推进位置会把提示指到错误的字上。
#[derive(Debug, Clone)]
pub struct StuckDetector {
    reference: Vec<char>,
    config: StuckConfig,
    gate: SpeechGate,
    confirmed: usize,
    prompted_at: Option<usize>,
}

impl StuckDetector {
    /// 按参考文本构造。参考文本里的标点与空白不计入字下标。
    #[must_use]
    pub fn new(reference: &str, config: StuckConfig, gate: SpeechGateConfig) -> Self {
        Self {
            reference: reference.chars().filter(|c| c.is_alphabetic()).collect(),
            config,
            gate: SpeechGate::new(gate),
            confirmed: 0,
            prompted_at: None,
        }
    }

    /// 推进会话游标。调用方是会话编排（示范到第几句、用户确认背到第几字），不是识别器。
    pub fn advance(&mut self, confirmed_chars: usize) {
        let confirmed = confirmed_chars.min(self.reference.len());
        if confirmed > self.confirmed {
            self.confirmed = confirmed;
        }
    }

    /// 观察一帧，必要时给出提示。
    ///
    /// 同一个游标位置最多提示一次；游标推进后才可能再次提示。
    pub fn observe(&mut self, rms: f32, frame_ms: u32) -> Option<Prompt> {
        let activity = self.gate.observe(rms, frame_ms);
        if activity == Activity::Speech {
            return None;
        }
        if self.confirmed >= self.reference.len() {
            return None;
        }
        if self.prompted_at == Some(self.confirmed) {
            return None;
        }
        let (threshold, reason) = if self.gate.spoke() {
            (
                self.config.trailing_silence_ms,
                PromptReason::TrailingSilence,
            )
        } else {
            (self.config.no_speech_ms, PromptReason::NoSpeechYet)
        };
        if self.gate.silence_run_ms() < threshold {
            return None;
        }
        self.prompted_at = Some(self.confirmed);
        let end = (self.confirmed + self.config.prompt_chars).min(self.reference.len());
        Some(Prompt {
            next_chars: self.reference[self.confirmed..end].iter().collect(),
            from_index: self.confirmed,
            at_ms: self.gate.elapsed_ms(),
            reason,
        })
    }

    /// 底层能量门控，供会话读取「是否开口 / 停顿次数」。
    #[must_use]
    pub const fn gate(&self) -> &SpeechGate {
        &self.gate
    }
}

// ---------------------------------------------------------------------------
// 双路成本与降级策略
// ---------------------------------------------------------------------------

/// 实时率：解码墙钟时间除以音频时长。小于 1 表示比实时快。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Rtf(f32);

impl Rtf {
    /// 由音频时长与解码墙钟时长求得。音频为零时长时返回 [`f32::INFINITY`]。
    #[must_use]
    pub fn measure(audio: Duration, wall: Duration) -> Self {
        if audio.is_zero() {
            return Self(f32::INFINITY);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "实时率只需 f32 精度，用于与 1.0 比较和写进报告"
        )]
        let value = (wall.as_secs_f64() / audio.as_secs_f64()) as f32;
        Self(value)
    }

    /// 数值。
    #[must_use]
    pub const fn value(self) -> f32 {
        self.0
    }

    /// 是否慢于实时。
    #[must_use]
    pub fn exceeds_realtime(self) -> bool {
        self.0 > REALTIME_BUDGET
    }
}

/// 单路与双路各自的实时率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualDecodeCost {
    /// 只跑无偏置一路。
    pub single: Rtf,
    /// 两路都跑。
    pub dual: Rtf,
}

/// 降级后关掉了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degradation {
    /// 没有降级。
    None,
    /// 关掉了偏置一路，因此没有高亮对齐。
    HighlightingDisabled,
}

/// 本次要跑几路。
///
/// **刻意没有「丢帧」这个取值。** 双路跑不动时正确的取舍是少跑一路，
/// 丢帧会让保留下来的那一路也变得不可信，等于用两个坏结果换一个坏结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodePlan {
    /// 两路都跑，高亮可用。
    Dual,
    /// 只跑无偏置一路，高亮关闭。
    SingleUnbiased {
        /// 关掉了什么。
        degradation: Degradation,
    },
}

impl DecodePlan {
    /// 本计划是否跑偏置一路。
    #[must_use]
    pub const fn runs_biased(self) -> bool {
        matches!(self, Self::Dual)
    }

    /// 高亮是否可用。
    #[must_use]
    pub const fn highlighting(self) -> bool {
        self.runs_biased()
    }
}

/// 按实测成本决定跑几路。
#[must_use]
pub const fn plan_for(cost: DualDecodeCost) -> DecodePlan {
    if cost.dual.0 > REALTIME_BUDGET {
        DecodePlan::SingleUnbiased {
            degradation: Degradation::HighlightingDisabled,
        }
    } else {
        DecodePlan::Dual
    }
}

// ---------------------------------------------------------------------------
// 解码抽象与事件驱动
// ---------------------------------------------------------------------------

/// 一次观察拿到的两路假设。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialHypothesis {
    /// 观察时刻，毫秒。
    pub at_ms: u64,
    /// 无偏置一路。
    pub unbiased: Option<UnbiasedAsrHyp>,
    /// 偏置一路，仅展示与诊断。
    pub biased: Option<BiasedHyp>,
}

/// 双路解码的抽象。真实实现在 `crate::asr::streaming`，测试用假实现。
pub trait DualDecode {
    /// 喂入一段采样并取回当前两路假设。
    ///
    /// # Errors
    ///
    /// 转述底层识别器的失败。
    fn accept(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<PartialHypothesis, VoiceError>;

    /// 输入结束，取回最终两路假设。
    ///
    /// # Errors
    ///
    /// 转述底层识别器的失败。
    fn finish(&mut self) -> Result<PartialHypothesis, VoiceError>;

    /// 至今实测的双路成本。
    fn cost(&self) -> DualDecodeCost;
}

/// 单声道 PCM 输入源。
pub trait PcmSource {
    /// 下一帧；`None` 表示输入结束。
    fn next_frame(&mut self) -> Option<Vec<f32>>;

    /// 采样率。
    fn sample_rate(&self) -> u32;
}

/// 一次识别的编排参数。
#[derive(Debug, Clone)]
pub struct RecognitionPlan {
    /// 参考诗文。
    pub reference: String,
    /// 起始会话游标，按字计。
    pub confirmed_chars: usize,
    /// 卡顿判定参数。
    pub stuck: StuckConfig,
    /// 能量门控参数。
    pub gate: SpeechGateConfig,
    /// 是否发出两路假设。**默认关闭**：77% CER 下把它们送到用户面前只会制造错误反馈。
    pub diagnostics: bool,
}

impl RecognitionPlan {
    /// 以默认阈值编排一次跟读，不发出假设。
    #[must_use]
    pub fn guided(reference: impl Into<String>) -> Self {
        Self {
            reference: reference.into(),
            confirmed_chars: 0,
            stuck: StuckConfig::default(),
            gate: SpeechGateConfig::default(),
            diagnostics: false,
        }
    }
}

/// 识别过程中的进度快照。可合并，丢掉中间值不损失信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognitionProgress {
    /// 已处理时长，毫秒。
    pub elapsed_ms: u64,
    /// 是否检测到开口。
    pub spoke: bool,
    /// 计入的停顿次数。
    pub pause_count: usize,
}

/// 识别结束时的汇总。**不含任何分数**。
#[derive(Debug, Clone, PartialEq)]
pub struct RecognitionOutcome {
    /// 是否检测到开口。
    pub spoke: bool,
    /// 计入的停顿次数。
    pub pause_count: usize,
    /// 总时长，毫秒。
    pub total_ms: u64,
    /// 发出的提示次数。
    pub prompt_count: usize,
    /// 实测双路成本。
    pub cost: DualDecodeCost,
    /// 按成本选定的计划。
    pub plan: DecodePlan,
}

/// 识别流上不可丢弃的增量结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RecognitionItem {
    /// 部分假设。仅在 [`RecognitionPlan::diagnostics`] 打开时发出。
    Partial(PartialHypothesis),
    /// 卡顿提示。
    Prompt(Prompt),
    /// 结束汇总。
    Outcome(RecognitionOutcome),
}

/// 启动一次流式识别，事件走全工作区统一的长操作协议。
///
/// 用 [`start_operation`] 而不是自造 sink，是为了让取消、背压与资源释放与语料派生、
/// AI 流式那两条长任务共用同一份语义。
pub fn start_recognition<S, D>(
    mut source: S,
    mut decoder: D,
    plan: RecognitionPlan,
) -> OperationHandle<RecognitionProgress, RecognitionItem>
where
    S: PcmSource + Send + 'static,
    D: DualDecode + Send + 'static,
{
    start_operation(move |reporter: OperationReporter<_, _>| {
        let sample_rate = source.sample_rate();
        let mut detector = StuckDetector::new(&plan.reference, plan.stuck, plan.gate);
        detector.advance(plan.confirmed_chars);
        let mut prompt_count = 0usize;
        let mut last_partial: Option<String> = None;

        while let Some(frame) = source.next_frame() {
            if reporter.is_cancelled() || reporter.is_closed() {
                return Ok(());
            }
            let frame_ms = frame_ms(frame.len(), sample_rate);
            if let Some(prompt) = detector.observe(crate::rms(&frame), frame_ms) {
                prompt_count += 1;
                if !reporter.item(RecognitionItem::Prompt(prompt)) {
                    return Ok(());
                }
            }
            let hypothesis = decoder
                .accept(&frame, sample_rate)
                .map_err(|e| e.to_string())?;
            let changed = hypothesis
                .unbiased
                .as_ref()
                .map(|hyp| hyp.as_str().to_owned());
            if plan.diagnostics && changed != last_partial && changed.is_some() {
                last_partial = changed;
                if !reporter.item(RecognitionItem::Partial(hypothesis)) {
                    return Ok(());
                }
            }
            reporter.progress(RecognitionProgress {
                elapsed_ms: detector.gate().elapsed_ms(),
                spoke: detector.gate().spoke(),
                pause_count: detector.gate().pause_count(),
            });
        }

        let final_hypothesis = decoder.finish().map_err(|e| e.to_string())?;
        if plan.diagnostics && !reporter.item(RecognitionItem::Partial(final_hypothesis)) {
            return Ok(());
        }
        let cost = decoder.cost();
        let outcome = RecognitionOutcome {
            spoke: detector.gate().spoke(),
            pause_count: detector.gate().pause_count(),
            total_ms: detector.gate().elapsed_ms(),
            prompt_count,
            cost,
            plan: plan_for(cost),
        };
        reporter.item(RecognitionItem::Outcome(outcome));
        Ok(())
    })
}

fn frame_ms(samples: usize, sample_rate: u32) -> u32 {
    if sample_rate == 0 {
        return 0;
    }
    u32::try_from(samples as u64 * 1000 / u64::from(sample_rate)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
