//! 语音跟读会话的编排：示范 → 复诵 → 节奏观察 → 用户自选等级。
//!
//! **判定层不带特性开关。** 状态机、节奏连贯度的算术、五条失败路径各自的原因码、
//! 一次性评级票据都是纯逻辑，把它们放在 `#[cfg(feature = "voice")]` 后面会让一台没有
//! 模型、没有声卡的机器无法验证它们，而这几件事恰恰是最需要被守住的。真实合成藏在
//! [`Demonstrator`] 之后，真实采集与识别藏在 [`Listener`] 之后。
//!
//! # 2026-08-11 裁决对本模块的约束
//!
//! 文言 ASR 的字错率实测 77.01%（`docs/reports/asr-cer.json`，且那是 TTS 合成音的乐观
//! 上界）。裁决因此把 v1 语音定为**跟读**，落到本模块上是四条硬约束：
//!
//! 1. **没有机器自动评级。** 会话末尾产出 [`SessionScore`]，等级由**用户自选**，经
//!    [`GradeTicket`] 提交。票据按值消耗，所以「提交两次」在类型层面无从表达。
//! 2. **播放与录音绝不重叠。** [`SessionStage::Demonstrating`] 与
//!    [`SessionStage::Listening`] 是两个互斥状态，且只能经「示范已结束」这一次转移相连。
//!    重叠会让识别器完美听见扬声器里自己的示范音，从而得到一个虚假的满覆盖。
//! 3. **节奏连贯度只由三项信号算出**：语音活动段起始间隔的方差、长停顿计数、
//!    总时长与示范音期望时长之比。入口是 [`RhythmInputs`]，它只有这三个私有字段，
//!    于是第四种信号无处可进。
//! 4. **不报告也不暗示读音判定。** 本模块的公开产出里没有任何一项表示读音是否标准；
//!    `tests/no_pronunciation_scoring.rs` 逐字段守着这一点。
//!
//! # 为什么起始时刻取自能量门控而不是识别器
//!
//! 识别器只暴露 token 的 start 时间而没有 stop（sherpa-onnx #985），强制对齐上游明确
//! 不做（#3536）。即便只用 start，77% 字错率下 token 序列本身也是噪声。能量门控给出的
//! 语音活动段起点不依赖任何转写是否正确，因此它是唯一站得住的来源。
//!
//! ```
//! use yunjian_voice::session::COHERENCE_LABEL;
//! assert_eq!(COHERENCE_LABEL, "节奏连贯度");
//! ```

use std::time::Duration;

use yunjian_core::VoiceSessionConfig;
use yunjian_core::operation::{OperationHandle, OperationReporter, start_operation};
use yunjian_recite::{FsrsGrade, RelativeRhythm, ReviewState, Scheduler, VoicePracticeFeedback};

use crate::VoiceError;
use crate::audio::AudioError;
use crate::models::ModelError;
use crate::permission::{DegradeReason, Practice, explain};
use crate::prosody::FootMark;
use crate::recognize::{Prompt, RecognitionPlan, SpeechGateConfig, StuckConfig};

/// 界面上这个指标唯一允许的名字。
///
/// **它不是读音质量。** 名字写死在这里而不是留给各端各写一遍，是因为「流畅度」这个词
/// 会被读成「说得标不标准」，而本项目不做也不宣称做读音判定。
pub const COHERENCE_LABEL: &str = "节奏连贯度";

// ---------------------------------------------------------------------------
// 节奏连贯度：三项信号，密封入口
// ---------------------------------------------------------------------------

/// 一次复诵的语音活动时序。**全部来自能量门控，与转写无关。**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechTimeline {
    /// 每一段语音活动的起始时刻，毫秒。
    pub onsets_ms: Vec<u64>,
    /// 长停顿次数。
    pub long_pause_count: usize,
    /// 这一次复诵的总时长，毫秒。
    pub total_ms: u64,
    /// 是否检测到开口。
    pub spoke: bool,
}

impl SpeechTimeline {
    /// 从一次识别的汇总取出时序。
    #[must_use]
    pub fn from_outcome(outcome: &crate::recognize::RecognitionOutcome) -> Self {
        Self {
            onsets_ms: outcome.onsets_ms.clone(),
            long_pause_count: outcome.long_pause_count,
            total_ms: outcome.total_ms,
            spoke: outcome.spoke,
        }
    }

    /// 合并多行复诵：起始时刻按各行开始时刻平移后首尾相接。
    ///
    /// 平移而不是各行独立，是因为节奏连贯度关心的是整首的节奏，而各行的时刻都以本行
    /// 开头为零点；直接拼接会让行首那一段的间隔变成一个负数量级的假值。
    #[must_use]
    pub fn concat(takes: &[Self]) -> Self {
        let mut merged = Self::default();
        let mut offset = 0u64;
        for take in takes {
            merged
                .onsets_ms
                .extend(take.onsets_ms.iter().map(|at| at.saturating_add(offset)));
            merged.long_pause_count += take.long_pause_count;
            merged.spoke |= take.spoke;
            offset = offset.saturating_add(take.total_ms);
        }
        merged.total_ms = offset;
        merged
    }
}

/// 节奏连贯度的三项输入，也是它的**唯一**输入。
///
/// 字段私有且只有三个：起始间隔的方差、长停顿计数、时长比。**任何基于转写的量都不得
/// 从这里进来**——想让第四种信号参与，唯一的办法是改这个结构体的定义，而那一改会同时
/// 打翻 `tests/no_pronunciation_scoring.rs` 的穷尽字段清单。这是「让错误无从表达」而
/// 不是「事后检查错误」。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RhythmInputs {
    gap_variance_ms2: f64,
    long_pause_count: usize,
    duration_ratio: f64,
    _seal: (),
}

impl RhythmInputs {
    /// 直接给三项数值，供合成时序的单元测试使用。
    #[must_use]
    pub fn from_parts(gap_variance_ms2: f64, long_pause_count: usize, duration_ratio: f64) -> Self {
        Self {
            gap_variance_ms2: gap_variance_ms2.max(0.0),
            long_pause_count,
            duration_ratio: duration_ratio.max(0.0),
            _seal: (),
        }
    }

    /// 从时序与示范音的期望时长求出三项。
    ///
    /// 期望时长为 0（没有示范音）时时长比取 1.0：那意味着没有比较基准，而不是「快了
    /// 无穷倍」。
    #[must_use]
    pub fn from_timeline(timeline: &SpeechTimeline, expected_ms: u64) -> Self {
        Self::from_parts(
            gap_variance_ms2(&timeline.onsets_ms),
            timeline.long_pause_count,
            duration_ratio(timeline.total_ms, expected_ms),
        )
    }

    /// 起始间隔的总体方差，毫秒平方。
    #[must_use]
    pub const fn gap_variance_ms2(&self) -> f64 {
        self.gap_variance_ms2
    }

    /// 长停顿次数。
    #[must_use]
    pub const fn long_pause_count(&self) -> usize {
        self.long_pause_count
    }

    /// 总时长与期望时长之比。
    #[must_use]
    pub const fn duration_ratio(&self) -> f64 {
        self.duration_ratio
    }
}

/// 相邻起始时刻之差的总体方差，毫秒平方。少于两段语音活动时无间隔可言，取 0。
#[must_use]
pub fn gap_variance_ms2(onsets_ms: &[u64]) -> f64 {
    if onsets_ms.len() < 2 {
        return 0.0;
    }
    let gaps: Vec<f64> = onsets_ms
        .windows(2)
        .map(|pair| {
            let earlier = pair[0].min(pair[1]);
            let later = pair[0].max(pair[1]);
            (later - earlier) as f64
        })
        .collect();
    let count = gaps.len() as f64;
    let mean = gaps.iter().sum::<f64>() / count;
    gaps.iter().map(|gap| (gap - mean).powi(2)).sum::<f64>() / count
}

/// 总时长与期望时长之比；期望为 0 时取 1.0。
#[must_use]
pub fn duration_ratio(total_ms: u64, expected_ms: u64) -> f64 {
    if expected_ms == 0 {
        return 1.0;
    }
    total_ms as f64 / expected_ms as f64
}

/// 节奏连贯度，落在 `[0.0, 1.0]`。**不是读音评分。**
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Coherence(f64);

impl Coherence {
    /// 数值。
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// 界面标签，恒为 [`COHERENCE_LABEL`]。
    #[must_use]
    pub const fn label(self) -> &'static str {
        COHERENCE_LABEL
    }
}

/// 由三项输入求节奏连贯度。**签名就是契约**：它只能看见 [`RhythmInputs`]。
///
/// 三项各归一到 `(0.0, 1.0]` 后相乘。归一用 `s / (s + x)` 这种形式，因为它在 `x = 0`
/// 时取 1、在 `x = s` 时取 0.5、随 `x` 单调递减且永不到 0——这三条正是这里需要的性质：
/// 完全匀速得满分、偏离到尺度值得半分、再差也不会把整体压成 0 而丢掉另两项的信息。
///
/// - 匀速度：`scale / (scale + 方差)`，尺度取 `gap_variance_scale_ms2`。
/// - 连贯度：`容忍 / (容忍 + 长停顿次数)`，尺度取 `long_pause_tolerance`。
/// - 速度：`容忍 / (容忍 + |时长比 - 1|)`，尺度取 `duration_ratio_tolerance`。
#[must_use]
pub fn coherence(inputs: &RhythmInputs, config: &VoiceSessionConfig) -> Coherence {
    let steadiness = decay(inputs.gap_variance_ms2, config.gap_variance_scale_ms2);
    let continuity = decay(inputs.long_pause_count as f64, config.long_pause_tolerance);
    let pacing = decay(
        (inputs.duration_ratio - 1.0).abs(),
        config.duration_ratio_tolerance,
    );
    Coherence((steadiness * continuity * pacing).clamp(0.0, 1.0))
}

fn decay(observed: f64, scale: f64) -> f64 {
    if scale <= 0.0 {
        return if observed > 0.0 { 0.0 } else { 1.0 };
    }
    scale / (scale + observed.max(0.0))
}

/// 由时长比判定相对示范音的快慢。
#[must_use]
pub fn relative_rhythm(inputs: &RhythmInputs, config: &VoiceSessionConfig) -> RelativeRhythm {
    let band = config.similar_band.max(0.0);
    if inputs.duration_ratio > 1.0 + band {
        return RelativeRhythm::Slower;
    }
    if inputs.duration_ratio < 1.0 - band {
        return RelativeRhythm::Faster;
    }
    RelativeRhythm::Similar
}

// ---------------------------------------------------------------------------
// 会话产出与一次性评级票据
// ---------------------------------------------------------------------------

/// 一次跟读会话的产出。
///
/// **刻意不实现 `Clone`。** [`Self::into_ticket`] 按值消耗它，于是一次会话最多铸出一枚
/// 评级票据；能克隆就等于能铸两枚，「恰好提交一次」也就退回成一条约定。
#[derive(Debug, PartialEq)]
pub struct SessionScore {
    /// 是否开口、停顿次数、相对节奏三项观察，与打字评分刻意不可互换。
    pub feedback: VoicePracticeFeedback,
    /// 节奏连贯度。
    pub coherence: Coherence,
    /// 连贯度的三项原始输入，便于界面解释这个数是怎么来的。
    pub inputs: RhythmInputs,
    /// 实际复诵了几行。
    pub lines_attempted: usize,
    /// 全会话发出的卡顿提示次数。
    pub prompt_count: usize,
}

impl SessionScore {
    /// 铸出这次会话的评级票据，并消耗自身。
    #[must_use]
    pub fn into_ticket(self, stable_id: impl Into<String>) -> GradeTicket {
        GradeTicket {
            stable_id: stable_id.into(),
        }
    }
}

/// 用户自选 FSRS 等级的一次性提交票据。
///
/// **按值消耗，因此「提交两次」在类型层面无从表达**——第二次调用是 use-after-move，编译
/// 期就被拒。`tests/ui/grade_ticket_cannot_be_submitted_twice.rs` 把这一条钉住。
#[derive(Debug)]
pub struct GradeTicket {
    stable_id: String,
}

impl GradeTicket {
    /// 这张票据对应的作品标识。
    #[must_use]
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    /// 提交用户自己选的等级。
    ///
    /// 等级由调用方从 [`FsrsGrade::ALL`] 里让用户挑，**不由本模块从任何观察推出**：
    /// 77% 字错率下任何自动映射都是在掷硬币。
    ///
    /// # Errors
    ///
    /// 转述排程器的写入失败。
    pub fn submit(
        self,
        scheduler: &mut Scheduler,
        grade: FsrsGrade,
    ) -> yunjian_core::Result<ReviewState> {
        scheduler.review(&self.stable_id, grade)
    }
}

// ---------------------------------------------------------------------------
// 五条失败路径
// ---------------------------------------------------------------------------

/// 语音不可用时改走打字练习的落点，携带**这一条**失败独有的原因码与已完成的进度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedFallback {
    /// 原因码。五条失败路径各不相同。
    pub reason: DegradeReason,
    /// 面向用户的中文解释。
    pub message: String,
    /// 掉线之前已经复诵完的行数。**不清零**——一次设备故障不该让用户重头再来。
    pub completed_lines: usize,
}

impl TypedFallback {
    /// 由原因码与已完成进度构造。
    #[must_use]
    pub fn new(
        reason: DegradeReason,
        platform: Option<crate::platform::Platform>,
        completed_lines: usize,
    ) -> Self {
        Self {
            reason,
            message: explain(reason, platform),
            completed_lines,
        }
    }

    /// 由音频失败构造：无权限、无设备、系统过低、设备被占用都从这里进。
    #[must_use]
    pub fn from_audio(error: &AudioError, completed_lines: usize) -> Self {
        Self::new(error.degrade_reason(), error.platform(), completed_lines)
    }

    /// 由模型失败构造。
    #[must_use]
    pub fn from_model(error: &ModelError, completed_lines: usize) -> Self {
        Self::new(error.degrade_reason(), None, completed_lines)
    }

    /// 由识别失败构造：录到了声音而识别不接受，与采集失败刻意分开。
    #[must_use]
    pub fn recognition_rejected(completed_lines: usize) -> Self {
        Self::new(DegradeReason::RecognitionRejected, None, completed_lines)
    }

    /// 转成通用的练习路径判定。
    #[must_use]
    pub fn practice(&self) -> Practice {
        Practice::Typed {
            reason: self.reason,
            message: self.message.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// 状态机
// ---------------------------------------------------------------------------

/// 会话当前处在哪一步。
///
/// [`Self::Demonstrating`] 与 [`Self::Listening`] 是两个状态而不是两个布尔位，
/// 于是「边播边录」无从表达——那种重叠会让识别器听见扬声器里自己的示范音，得到一个
/// 虚假的满覆盖。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStage {
    /// 还没开始。
    Idle,
    /// 正在播放第 `line` 行的示范音。**此时不录音。**
    Demonstrating {
        /// 行序号，从 0 起。
        line: usize,
    },
    /// 正在录第 `line` 行的复诵。**此时不播放。**
    Listening {
        /// 行序号，从 0 起。
        line: usize,
    },
    /// 全部行已复诵完，等用户自选等级。
    AwaitingGrade,
    /// 已降级到打字练习。
    Degraded {
        /// 降级落点。
        fallback: TypedFallback,
    },
}

impl SessionStage {
    /// 此刻是否在放示范音。
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        matches!(self, Self::Demonstrating { .. })
    }

    /// 此刻是否在录音。
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        matches!(self, Self::Listening { .. })
    }

    /// 是否已经终结（等待评级或已降级）。
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::AwaitingGrade | Self::Degraded { .. })
    }
}

/// 一首作品切成的逐行跟读脚本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionScript {
    lines: Vec<String>,
}

impl SessionScript {
    /// 按标点切句。全是标点或空串时返回 `None`：没有一行可跟读的会话不该被开出来。
    #[must_use]
    pub fn from_poem(text: &str) -> Option<Self> {
        let lines: Vec<String> = text
            .split(|c: char| !c.is_alphabetic())
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        (!lines.is_empty()).then_some(Self { lines })
    }

    /// 逐行文本。
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// 行数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// 是否没有任何一行。恒为 `false`，因为构造点拒绝空脚本。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 示范与聆听的抽象
// ---------------------------------------------------------------------------

/// 一行示范音的播放结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Demonstration {
    /// 逐音步时间戳，供 karaoke 高亮；来自拼接而不是强制对齐。
    pub marks: Vec<FootMark>,
    /// 这一行示范音的时长，毫秒。它就是复诵时长比的分母。
    pub duration_ms: u64,
}

/// 逐行示范。真实实现放 TTS 合成加播放，测试用假实现。
///
/// **`demonstrate` 只在播放真正结束后返回**，这是「播放与录音不重叠」的实现侧保证：
/// 会话在它返回之前不会进入 [`SessionStage::Listening`]。
pub trait Demonstrator {
    /// 合成并播完一行。
    ///
    /// # Errors
    ///
    /// 转述合成或播放的失败。
    fn demonstrate(&mut self, line: &str) -> Result<Demonstration, VoiceError>;
}

/// 一行复诵的观察结果。**不含任何转写。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTake {
    /// 语音活动时序。
    pub timeline: SpeechTimeline,
    /// 这一行里发出的卡顿提示。
    pub prompts: Vec<Prompt>,
}

/// 逐行聆听。真实实现驱动 [`crate::recognize::start_recognition`]，测试用假实现。
pub trait Listener {
    /// 录并观察一行复诵。
    ///
    /// # Errors
    ///
    /// 转述采集或识别的失败。
    fn listen(&mut self, line: &str, plan: &RecognitionPlan) -> Result<LineTake, VoiceError>;
}

// ---------------------------------------------------------------------------
// 事件流
// ---------------------------------------------------------------------------

/// 会话进度快照。可合并，丢掉中间值不损失信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProgress {
    /// 当前处在哪一步。
    pub stage: SessionStage,
    /// 已经复诵完的行数。
    pub completed_lines: usize,
    /// 总行数。
    pub total_lines: usize,
}

/// 会话流上不可丢弃的增量结果。
#[derive(Debug, PartialEq)]
pub enum SessionItem {
    /// 一行示范音已播完，附逐音步时间戳。
    Demonstrated {
        /// 行序号。
        line: usize,
        /// 时间戳与时长。
        demonstration: Demonstration,
    },
    /// 一次卡顿提示。
    Prompt(Prompt),
    /// 一行复诵的观察结果。
    LineObserved {
        /// 行序号。
        line: usize,
        /// 观察结果。
        take: LineTake,
    },
    /// 会话产出。**用户据此自选等级**，本模块不代他选。
    Report(SessionScore),
    /// 降级到打字练习。
    Fallback(TypedFallback),
}

/// 一次跟读会话的编排参数。
#[derive(Debug, Clone)]
pub struct SessionPlan {
    /// 逐行脚本。
    pub script: SessionScript,
    /// 是否先播示范音。关掉即纯复诵，时长比随之失去基准并退为 1.0。
    pub demonstrate: bool,
    /// 卡顿判定参数。
    pub stuck: StuckConfig,
    /// 能量门控参数。
    pub gate: SpeechGateConfig,
    /// 节奏连贯度参数。
    pub session: VoiceSessionConfig,
}

impl SessionPlan {
    /// 以默认阈值编排一次带示范的跟读。
    #[must_use]
    pub fn guided(script: SessionScript, session: VoiceSessionConfig) -> Self {
        Self {
            script,
            demonstrate: true,
            stuck: StuckConfig::default(),
            gate: SpeechGateConfig {
                long_pause_ms: session.long_pause_ms,
                ..SpeechGateConfig::default()
            },
            session,
        }
    }
}

/// 跑一次跟读会话，事件走全工作区统一的长操作协议。
///
/// 用 [`start_operation`] 而不是自造 sink，是为了让取消、背压与资源释放与语料派生、
/// AI 流式、流式识别那三条长任务共用同一份语义。
///
/// 循环严格是「播一行 → 播完 → 录一行 → 录完」。示范与聆听之间没有并发，因为
/// [`Demonstrator::demonstrate`] 返回即代表播放结束，而 [`SessionStage`] 不存在同时
/// 播放与录音的取值。
pub fn start_session<D, L>(
    mut demonstrator: D,
    mut listener: L,
    plan: SessionPlan,
) -> OperationHandle<SessionProgress, SessionItem>
where
    D: Demonstrator + Send + 'static,
    L: Listener + Send + 'static,
{
    start_operation(move |reporter: OperationReporter<_, _>| {
        let total_lines = plan.script.len();
        let mut takes: Vec<SpeechTimeline> = Vec::new();
        let mut expected_ms = 0u64;
        let mut prompt_count = 0usize;

        for (line, text) in plan.script.lines().iter().enumerate() {
            if reporter.is_cancelled() || reporter.is_closed() {
                return Ok(());
            }

            if plan.demonstrate {
                report_stage(
                    &reporter,
                    SessionStage::Demonstrating { line },
                    line,
                    total_lines,
                );
                match demonstrator.demonstrate(text) {
                    Ok(demonstration) => {
                        expected_ms = expected_ms.saturating_add(demonstration.duration_ms);
                        if !reporter.item(SessionItem::Demonstrated {
                            line,
                            demonstration,
                        }) {
                            return Ok(());
                        }
                    }
                    Err(error) => {
                        return degrade_and_stop(
                            &reporter,
                            fallback_for(&error, line),
                            line,
                            total_lines,
                        );
                    }
                }
            }

            report_stage(
                &reporter,
                SessionStage::Listening { line },
                line,
                total_lines,
            );
            let recognition = RecognitionPlan {
                reference: text.clone(),
                confirmed_chars: 0,
                stuck: plan.stuck,
                gate: plan.gate,
                diagnostics: false,
            };
            let take = match listener.listen(text, &recognition) {
                Ok(take) => take,
                Err(error) => {
                    return degrade_and_stop(
                        &reporter,
                        fallback_for(&error, line),
                        line,
                        total_lines,
                    );
                }
            };
            for prompt in take.prompts.clone() {
                prompt_count += 1;
                if !reporter.item(SessionItem::Prompt(prompt)) {
                    return Ok(());
                }
            }
            takes.push(take.timeline.clone());
            if !reporter.item(SessionItem::LineObserved { line, take }) {
                return Ok(());
            }
        }

        let timeline = SpeechTimeline::concat(&takes);
        let inputs = RhythmInputs::from_timeline(&timeline, expected_ms);
        let score = SessionScore {
            feedback: VoicePracticeFeedback::new(
                timeline.spoke,
                timeline.long_pause_count,
                relative_rhythm(&inputs, &plan.session),
            ),
            coherence: coherence(&inputs, &plan.session),
            inputs,
            lines_attempted: takes.len(),
            prompt_count,
        };
        report_stage(
            &reporter,
            SessionStage::AwaitingGrade,
            takes.len(),
            total_lines,
        );
        reporter.item(SessionItem::Report(score));
        Ok(())
    })
}

fn report_stage(
    reporter: &OperationReporter<SessionProgress, SessionItem>,
    stage: SessionStage,
    completed_lines: usize,
    total_lines: usize,
) {
    reporter.progress(SessionProgress {
        stage,
        completed_lines,
        total_lines,
    });
}

fn degrade_and_stop(
    reporter: &OperationReporter<SessionProgress, SessionItem>,
    fallback: TypedFallback,
    completed_lines: usize,
    total_lines: usize,
) -> Result<(), String> {
    report_stage(
        reporter,
        SessionStage::Degraded {
            fallback: fallback.clone(),
        },
        completed_lines,
        total_lines,
    );
    reporter.item(SessionItem::Fallback(fallback));
    Ok(())
}

/// 把一个语音失败映射成打字降级落点，保留已完成的行数。
///
/// [`VoiceError::ModelMissing`] 走模型原因码，其余走 [`VoiceError::degrade_reason`]；
/// 「设备中途掉线」因此落在采集类原因上，而不是被当成识别被拒。
#[must_use]
pub fn fallback_for(error: &VoiceError, completed_lines: usize) -> TypedFallback {
    TypedFallback::new(error.degrade_reason(), None, completed_lines)
}

/// 示范音期望时长的换算，供调用方由 [`crate::prosody::Reading`] 得到 [`Demonstration`]。
#[must_use]
pub fn duration_to_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
