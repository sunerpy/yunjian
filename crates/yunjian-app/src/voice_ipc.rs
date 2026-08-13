//! 语音跟读会话的桌面端接线：示范、录音、karaoke 高亮、卡顿提示、最终视图、模型下载。
//!
//! # 两条硬约束，各自有落点
//!
//! **一、采集与识别期间不得占用 WebView 主线程。** 四条命令全是 `async`：读语料库与合成
//! 这类阻塞活计进 `spawn_blocking`；长驻的采集与识别走
//! [`yunjian_voice::session::start_session`]，它在自己的线程里跑，命令只在 async 侧排空
//! 事件队列并往 [`Channel`] 转发。分类判据与 `ipc.rs` 的三类命令逐条一致。
//!
//! **二、显示的分数绝不来自偏置假设。** [`VoiceReportOut`] **一个字符串字段都没有**——
//! 它的九个键全部来自能量门控测到的时序（是否开口、长停顿数、起始间隔方差、时长比），
//! 与转写无关。偏置一路的文本只走 [`VoiceItemOut::AsrPartial`]，那一项自带
//! `diagnostics_only: true` 与 [`ASR_PARTIAL_NOTE`]，界面据此把它收进诊断区。
//! `tests` 里有一条注入哨兵串的用例：假聆听器吐出一段可辨识的偏置文本，断言它在报告
//! 载荷里一次都不出现。
//!
//! # 为什么 karaoke 高亮的时间戳来自拼接而不是对齐
//!
//! 上游明确不做强制对齐（sherpa-onnx #3536），且识别器只暴露 token 的 **start** 时间、
//! 没有 stop（#985）。逐音步合成天然知道每一段起止于第几个样本，
//! [`yunjian_voice::prosody::splice`] 把它记成 [`FootMark`]，于是时间戳是拼接的算术结果，
//! 不需要任何对齐。
//!
//! # 音频不从命令返回
//!
//! 示范音经 `ipc.rs` 的 `yunjian-audio` 自定义 URI 协议交付：命令返回一个 URL，WebView
//! 用 `<audio>` 去取。实测把 6.3 MB 的 PCM 当命令返回值序列化会在线上膨胀到 22.5 MB
//! （`learnings.md` 的 todo 64 一节），而一段二十秒的朗读正是这个量级。
//!
//! # 2026-08-11 裁决在本模块的落点
//!
//! 文言 ASR 的字错率实测 77.01%，因此 v1 语音**不做机器自动评分**，FSRS 等级由用户自选。
//! 本模块没有任何一条命令返回等级或建议等级：会话的终点是 [`VoiceItemOut::Report`]，
//! 用户看过之后走既有的 `recite_commit_grade` 自己选一档。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};
use yunjian_core::Config;
use yunjian_core::operation::{Event, OperationReporter, cancel, next_event, start_operation};
use yunjian_recite::RelativeRhythm;
use yunjian_voice::models::FetchProgress;
use yunjian_voice::permission::{DegradeReason, Practice};
use yunjian_voice::prosody::{FootMark, Reading};
use yunjian_voice::recognize::{Prompt, PromptReason, RecognitionPlan};
use yunjian_voice::session::{
    COHERENCE_LABEL, Demonstration, Demonstrator, LineTake, Listener, SessionItem, SessionPlan,
    SessionProgress, SessionScript, SessionStage, TypedFallback, start_session,
};

use crate::ipc::{AppState, IpcResult, blocking, new_operation_id, register_operation};

/// 部分假设那一项**必须**同屏出现的说明。
///
/// 它不是免责声明而是判据：CER 77% 下这段文本与用户实际说出的字没有可靠对应，把它读成
/// 「已经背到这里」会给出错误反馈。所以界面只能把它当诊断显示。
pub const ASR_PARTIAL_NOTE: &str =
    "以下转写仅供诊断：文言识别的字错率实测过高，它既不代表你说了什么，也不参与任何评分。";

/// 会话产出那一段必须同屏出现的说明。
pub const NO_MACHINE_SCORE_NOTE: &str = "语音路径不做机器评分：以下各项全部来自能量门控测到的时序，与识别转写无关；复习等级由你自己选。";

/// [`Demonstration`] 里样本下标使用的时基。
///
/// [`Demonstration`] 只带样本下标与整行时长，**不带采样率**——因为拼接算术本身与采样率
/// 无关。所以本模块与自己的示范器约定：交给会话的 [`FootMark`] 一律已换算到毫秒，
/// 即时基恒为 1000 Hz。凭一个猜出来的采样率去换算是另一条路，而那条路会在换了合成引擎
/// （MeloTTS 44.1 kHz 与 Kokoro 24 kHz 不同）的那一天把整首诗的高亮时刻整体拉错。
pub const MARK_TIMEBASE_HZ: u32 = 1000;

// ---------------------------------------------------------------------------
// 线上形状
// ---------------------------------------------------------------------------

/// 一个音步在示范音里的位置，毫秒。**karaoke 高亮的唯一驱动源。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FootMarkOut {
    /// 所在行序号。
    pub line: usize,
    /// 行内序号。
    pub index_in_line: usize,
    /// 音步文本。
    pub text: String,
    /// 起始时刻，毫秒。
    pub start_ms: u64,
    /// 结束时刻，毫秒。
    pub end_ms: u64,
}

impl FootMarkOut {
    /// 由拼接得到的标记换算成毫秒。
    #[must_use]
    pub fn from_mark(mark: &FootMark, sample_rate: u32) -> Self {
        Self {
            line: mark.line,
            index_in_line: mark.index_in_line,
            text: mark.text.clone(),
            start_ms: samples_to_ms(mark.start_sample, sample_rate),
            end_ms: samples_to_ms(mark.end_sample, sample_rate),
        }
    }
}

fn samples_to_ms(samples: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    samples as u64 * 1000 / u64::from(sample_rate)
}

/// 全部降级原因，供线上串与原因码互查。
///
/// 写成数组而不是靠 `match` 反查，是为了让「新增一个原因却忘了给它线上串」这件事
/// 有一条可执行的判据（见 `tests::every_degrade_reason_has_a_distinct_wire_key`）。
pub const WIRE_DEGRADE_REASONS: [DegradeReason; 10] = [
    DegradeReason::FeatureDisabled,
    DegradeReason::SystemTooOld,
    DegradeReason::PermissionDenied,
    DegradeReason::PermissionRestricted,
    DegradeReason::PermissionUndetermined,
    DegradeReason::NoInputDevice,
    DegradeReason::ModelUnavailable,
    DegradeReason::DeviceBusy,
    DegradeReason::RecognitionRejected,
    DegradeReason::CaptureFailed,
];

/// 降级原因码的线上串。
///
/// **穷尽匹配**，与 `AudioError::degrade_reason` 同一个理由：新增原因时编译器在这里报缺
/// 分支，于是界面不可能收到一个它没有对应文案的原因码。
#[must_use]
pub const fn degrade_reason_key(reason: DegradeReason) -> &'static str {
    match reason {
        DegradeReason::FeatureDisabled => "feature_disabled",
        DegradeReason::SystemTooOld => "system_too_old",
        DegradeReason::PermissionDenied => "permission_denied",
        DegradeReason::PermissionRestricted => "permission_restricted",
        DegradeReason::PermissionUndetermined => "permission_undetermined",
        DegradeReason::NoInputDevice => "no_input_device",
        DegradeReason::ModelUnavailable => "model_unavailable",
        DegradeReason::DeviceBusy => "device_busy",
        DegradeReason::RecognitionRejected => "recognition_rejected",
        DegradeReason::CaptureFailed => "capture_failed",
    }
}

/// 降级到打字练习的落点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedFallbackOut {
    /// 原因码。五条失败路径各不相同。
    pub reason: &'static str,
    /// 面向用户的中文解释，由 `yunjian_voice::permission::explain` 给出。
    pub message: String,
    /// 掉线之前已经复诵完的行数。**不清零。**
    pub completed_lines: usize,
}

impl TypedFallbackOut {
    /// 由会话的降级落点构造。
    #[must_use]
    pub fn from_fallback(fallback: &TypedFallback) -> Self {
        Self {
            reason: degrade_reason_key(fallback.reason),
            message: fallback.message.clone(),
            completed_lines: fallback.completed_lines,
        }
    }

    /// 由原因码与解释直接构造，供尚未开出会话就被挡住的路径使用。
    #[must_use]
    pub fn new(reason: DegradeReason, message: String, completed_lines: usize) -> Self {
        Self {
            reason: degrade_reason_key(reason),
            message,
            completed_lines,
        }
    }
}

/// 会话当前处在哪一步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum VoiceStageOut {
    /// 还没开始。
    Idle,
    /// 正在播第 `line` 行的示范音。**此时不录音。**
    Demonstrating {
        /// 行序号。
        line: usize,
    },
    /// 正在录第 `line` 行的复诵。**此时不播放。**
    Listening {
        /// 行序号。
        line: usize,
    },
    /// 全部行已复诵完，等用户自选等级。
    AwaitingGrade,
    /// 已降级到打字练习。
    Degraded {
        /// 降级落点。
        fallback: TypedFallbackOut,
    },
}

impl VoiceStageOut {
    fn from_stage(stage: &SessionStage) -> Self {
        match stage {
            SessionStage::Idle => Self::Idle,
            SessionStage::Demonstrating { line } => Self::Demonstrating { line: *line },
            SessionStage::Listening { line } => Self::Listening { line: *line },
            SessionStage::AwaitingGrade => Self::AwaitingGrade,
            SessionStage::Degraded { fallback } => Self::Degraded {
                fallback: TypedFallbackOut::from_fallback(fallback),
            },
        }
    }
}

/// 会话进度快照。可合并，丢掉中间值不损失信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceProgressOut {
    /// 当前步骤。
    pub stage: VoiceStageOut,
    /// 已复诵完的行数。
    pub completed_lines: usize,
    /// 总行数。
    pub total_lines: usize,
}

/// 一次流式部分假设。**仅供诊断，不参与任何评分。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AsrPartialOut {
    /// 观察时刻，毫秒。
    pub at_ms: u64,
    /// 无偏置一路的转写。
    pub unbiased: Option<String>,
    /// 偏置一路的转写。已匹配前缀的实时高亮取自它，而它**只能**用来高亮。
    pub biased: Option<String>,
    /// 恒为 `true`。做成字段而不是靠约定，是为了让「把它当用户反馈渲染」这件事在
    /// 载荷上就看得见，而不是等到有人在界面里发现一段假反馈。
    pub diagnostics_only: bool,
    /// 必须同屏出现的说明。
    pub note: &'static str,
}

impl AsrPartialOut {
    /// 由两路转写构造。
    #[must_use]
    pub fn new(at_ms: u64, unbiased: Option<String>, biased: Option<String>) -> Self {
        Self {
            at_ms,
            unbiased,
            biased,
            diagnostics_only: true,
            note: ASR_PARTIAL_NOTE,
        }
    }
}

/// 卡顿提示原因的线上串。
#[must_use]
pub const fn prompt_reason_key(reason: PromptReason) -> &'static str {
    match reason {
        PromptReason::NoSpeechYet => "no_speech_yet",
        PromptReason::TrailingSilence => "trailing_silence",
    }
}

/// 相对节奏的线上串。
#[must_use]
pub const fn relative_rhythm_key(rhythm: RelativeRhythm) -> &'static str {
    match rhythm {
        RelativeRhythm::Slower => "slower",
        RelativeRhythm::Similar => "similar",
        RelativeRhythm::Faster => "faster",
    }
}

/// 一次跟读会话的产出。
///
/// # 这个结构体的**键集**本身就是一道门禁
///
/// 九个键全部是数与布尔，**一个字符串字段都没有**——于是「把转写当分数显示」在这一层
/// 无从表达。`tests::report_payload_keys_are_frozen` 逐键钉住它：任何人加一个
/// `transcript` / `matched_text` / `accuracy` 都会让那条断言变红，而不是等到有人在界面上
/// 看见一个假分数才发现。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceReportOut {
    /// 是否检测到开口。
    pub spoke: bool,
    /// 长停顿次数。
    pub long_pause_count: usize,
    /// 相对示范音的快慢。
    pub relative_rhythm: &'static str,
    /// 节奏连贯度，落在 `[0, 1]`。**不是读音评分。**
    pub coherence: f64,
    /// 这个指标唯一允许的名字，恒为 [`COHERENCE_LABEL`]。
    pub coherence_label: &'static str,
    /// 起始间隔方差，毫秒平方。连贯度三项输入之一。
    pub gap_variance_ms2: f64,
    /// 总时长与示范音期望时长之比。连贯度三项输入之一。
    pub duration_ratio: f64,
    /// 实际复诵了几行。
    pub lines_attempted: usize,
    /// 全会话发出的卡顿提示次数。
    pub prompt_count: usize,
}

/// 会话流上不可丢弃的增量结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum VoiceItemOut {
    /// 一行示范音已播完，附逐音步时间戳。
    Demonstrated {
        /// 行序号。
        line: usize,
        /// 这一行示范音的时长，毫秒。
        duration_ms: u64,
        /// 逐音步时间戳。**长度恒等于这一行的音步数。**
        marks: Vec<FootMarkOut>,
    },
    /// 一次流式部分假设。
    AsrPartial(AsrPartialOut),
    /// 一次卡顿提示。
    Prompt {
        /// 提示的下几个字。
        next_chars: String,
        /// 这些字在参考文本里的起始下标。
        from_index: usize,
        /// 提示发出的时刻，毫秒。
        at_ms: u64,
        /// 触发原因。
        reason: &'static str,
    },
    /// 一行复诵的观察结果。**不含任何转写。**
    LineObserved {
        /// 行序号。
        line: usize,
        /// 是否检测到开口。
        spoke: bool,
        /// 长停顿次数。
        long_pause_count: usize,
        /// 这一行的时长，毫秒。
        total_ms: u64,
        /// 每一段语音活动的起始时刻，毫秒。
        onsets_ms: Vec<u64>,
    },
    /// 会话产出。
    Report(VoiceReportOut),
    /// 降级到打字练习。
    Fallback(TypedFallbackOut),
}

fn session_item(item: &SessionItem) -> VoiceItemOut {
    match item {
        SessionItem::Demonstrated {
            line,
            demonstration,
        } => VoiceItemOut::Demonstrated {
            line: *line,
            duration_ms: demonstration.duration_ms,
            marks: demonstration
                .marks
                .iter()
                .map(|mark| FootMarkOut::from_mark(mark, MARK_TIMEBASE_HZ))
                .collect(),
        },
        SessionItem::Prompt(prompt) => prompt_item(prompt),
        SessionItem::LineObserved { line, take } => VoiceItemOut::LineObserved {
            line: *line,
            spoke: take.timeline.spoke,
            long_pause_count: take.timeline.long_pause_count,
            total_ms: take.timeline.total_ms,
            onsets_ms: take.timeline.onsets_ms.clone(),
        },
        SessionItem::Report(score) => VoiceItemOut::Report(VoiceReportOut {
            spoke: score.feedback.spoke,
            long_pause_count: score.feedback.pause_count,
            relative_rhythm: relative_rhythm_key(score.feedback.relative_rhythm),
            coherence: score.coherence.value(),
            coherence_label: COHERENCE_LABEL,
            gap_variance_ms2: score.inputs.gap_variance_ms2(),
            duration_ratio: score.inputs.duration_ratio(),
            lines_attempted: score.lines_attempted,
            prompt_count: score.prompt_count,
        }),
        SessionItem::Fallback(fallback) => {
            VoiceItemOut::Fallback(TypedFallbackOut::from_fallback(fallback))
        }
    }
}

fn prompt_item(prompt: &Prompt) -> VoiceItemOut {
    VoiceItemOut::Prompt {
        next_chars: prompt.next_chars.clone(),
        from_index: prompt.from_index,
        at_ms: prompt.at_ms,
        reason: prompt_reason_key(prompt.reason),
    }
}

/// 模型下载进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum ModelFetchOut {
    /// 正在下载。`bytes_total == 0` 表示服务端没给长度。
    Downloading {
        /// 已写出的字节数。
        bytes_done: u64,
        /// 预期总字节数；未知时为零。
        bytes_total: u64,
    },
    /// 正在核对已落地归档的摘要。
    Verifying {
        /// 归档字节数。
        bytes: u64,
    },
    /// 摘要已核对通过。
    Verified,
    /// 正在解包。
    Unpacking,
}

impl From<FetchProgress> for ModelFetchOut {
    fn from(progress: FetchProgress) -> Self {
        match progress {
            FetchProgress::Downloading {
                bytes_done,
                bytes_total,
            } => Self::Downloading {
                bytes_done,
                bytes_total,
            },
            FetchProgress::Verifying { bytes } => Self::Verifying { bytes },
            FetchProgress::Verified => Self::Verified,
            FetchProgress::Unpacking => Self::Unpacking,
        }
    }
}

/// 语音在本机可用不可用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceAvailabilityOut {
    /// 可以走语音跟读。
    Voice {
        /// 节奏连贯度的界面标签。
        coherence_label: &'static str,
        /// 会话产出必须同屏出现的说明。
        note: &'static str,
    },
    /// 只能走打字练习，附这一条失败独有的原因。
    Typed {
        /// 原因码。
        reason: &'static str,
        /// 面向用户的中文解释。
        message: String,
    },
}

impl VoiceAvailabilityOut {
    fn from_practice(practice: &Practice) -> Self {
        match practice {
            Practice::Voice => Self::Voice {
                coherence_label: COHERENCE_LABEL,
                note: NO_MACHINE_SCORE_NOTE,
            },
            Practice::Typed { reason, message } => Self::Typed {
                reason: degrade_reason_key(*reason),
                message: message.clone(),
            },
        }
    }
}

/// 一段可播放的示范音，以及驱动 karaoke 高亮的时间戳。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceDemonstrationOut {
    /// 自定义 URI 协议下的音频地址。**音频本体不经命令返回值。**
    pub audio_url: String,
    /// 采样率。
    pub sample_rate: u32,
    /// 总时长，毫秒。
    pub duration_ms: u64,
    /// 逐音步时间戳。**长度恒等于音步数。**
    pub marks: Vec<FootMarkOut>,
}

// ---------------------------------------------------------------------------
// 语音装置
// ---------------------------------------------------------------------------

/// 部分假设的出口。会话的聆听器经它把诊断转写送到 [`Channel`] 上。
pub(crate) type PartialSink = Arc<dyn Fn(AsrPartialOut) + Send + Sync>;

/// 一次会话所需的示范器与聆听器。
pub(crate) struct Coupling {
    /// 逐行示范。返回的 [`Demonstration`] 里标记已换算到 [`MARK_TIMEBASE_HZ`]。
    pub demonstrator: Box<dyn Demonstrator + Send>,
    /// 逐行聆听。
    pub listener: Box<dyn Listener + Send>,
}

/// 语音会话需要从外界拿到的全部东西：语料正文、合成、采集与识别、模型。
///
/// **存在的理由与 `audio::InputDevice`、`prosody::FootSynthesizer` 完全同因**：命令的
/// 外壳必须能在一台没有模型、没有声卡的机器上被真的调用一次，否则「权限被拒会切到打字
/// 模式并带上原因」这条断言只能靠读代码确认，而读代码确认不了运行时行为。
pub(crate) trait VoiceRig: Send + Sync + 'static {
    /// 语音在本机是否可用。
    fn probe(&self, config: &Config) -> Practice;

    /// 一首作品的正文。
    fn body(&self, config: &Config, poem_id: &str) -> IpcResult<String>;

    /// 合成整篇朗读，采样率与标记时基取自合成器本身。
    fn read(&self, config: &Config, body: &str) -> IpcResult<Reading>;

    /// 开一次跟读会话所需的示范器与聆听器。
    fn couple(&self, config: &Config, partials: PartialSink) -> IpcResult<Coupling>;

    /// 取一个模型到本地缓存，边取边报进度，`stop` 为真时中止且不留下半个文件。
    fn fetch_model(
        &self,
        config: &Config,
        name: &str,
        stop: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(ModelFetchOut),
    ) -> Result<PathBuf, TypedFallback>;
}

/// 装上生产装置。
pub(crate) fn production_rig() -> Arc<dyn VoiceRig> {
    Arc::new(rig::ProductionRig)
}

/// 一首作品的正文。**两种生产装置共用它**：正文来自语料库，与有没有编译语音能力无关，
/// 于是不开 `voice` 的构建里「这首诗要跟读哪几句」照样答得出来。
pub(crate) fn corpus_body(config: &Config, poem_id: &str) -> IpcResult<String> {
    let corpus =
        yunjian_core::CorpusHandle::open(&config.corpus).map_err(|error| error.to_string())?;
    let detail = yunjian_core::Yunjian::new(corpus)
        .poem_detail(yunjian_core::PoemDetailRequest {
            poem_id: poem_id.to_owned(),
        })
        .map_err(|error| error.to_string())?;
    Ok(detail.poem.body)
}

#[cfg(not(feature = "voice"))]
#[path = "voice_rig_disabled.rs"]
mod rig;

#[cfg(feature = "voice")]
#[path = "voice_rig_enabled.rs"]
mod rig;

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 语音是否可用。**界面在渲染任何语音控件之前先问这一条。**
#[tauri::command]
pub(crate) async fn voice_availability<R: Runtime>(
    app: AppHandle<R>,
) -> IpcResult<VoiceAvailabilityOut> {
    blocking(app, move |state| {
        let practice = state.voice_rig().probe(&state.config());
        Ok(VoiceAvailabilityOut::from_practice(&practice))
    })
    .await
}

/// 示范按钮的请求。
#[derive(Debug, Deserialize)]
pub(crate) struct VoiceDemonstrateRequest {
    poem_id: String,
}

/// 合成一段示范朗读，返回可播放地址与逐音步时间戳。
#[tauri::command]
pub(crate) async fn voice_demonstrate<R: Runtime>(
    app: AppHandle<R>,
    request: VoiceDemonstrateRequest,
) -> IpcResult<VoiceDemonstrationOut> {
    blocking(app, move |state| {
        let config = state.config();
        let rig = state.voice_rig();
        if let Practice::Typed { message, .. } = rig.probe(&config) {
            return Err(message);
        }
        let body = rig.body(&config, &request.poem_id)?;
        let reading = rig.read(&config, &body)?;
        let marks = reading
            .marks
            .iter()
            .map(|mark| FootMarkOut::from_mark(mark, reading.sample_rate))
            .collect();
        let duration_ms = u64::try_from(reading.duration().as_millis()).unwrap_or(u64::MAX);
        let sample_rate = reading.sample_rate;
        let audio_url = state.put_audio(encode_wav(&reading), "audio/wav");
        Ok(VoiceDemonstrationOut {
            audio_url,
            sample_rate,
            duration_ms,
            marks,
        })
    })
    .await
}

/// 把一段拼接好的朗读编成 16 位单声道 PCM 的 WAV。
///
/// 自己写这 44 字节的头而不是拉一个编码库：WebView 的 `<audio>` 认的最小格式就是它，
/// 而 `yunjian-voice::tts::write_wav` 只写文件、且在 `voice` 特性后面——本模块要在不开
/// 该特性的构建里也能被测到。
#[must_use]
pub fn encode_wav(reading: &Reading) -> Vec<u8> {
    let rate = if reading.sample_rate == 0 {
        MARK_TIMEBASE_HZ
    } else {
        reading.sample_rate
    };
    let data_len = reading.samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &u32::try_from(36 + data_len)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&rate.saturating_mul(2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data_len).unwrap_or(u32::MAX).to_le_bytes());
    for sample in &reading.samples {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "先 clamp 到 [-1, 1] 再乘 i16::MAX，结果必然落在 i16 内"
        )]
        let quantized = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.extend_from_slice(&quantized.to_le_bytes());
    }
    out
}

/// 录音按钮的请求。
#[derive(Debug, Deserialize)]
pub(crate) struct VoiceSessionRequest {
    poem_id: String,
    #[serde(default)]
    demonstrate: bool,
    #[serde(default)]
    operation_id: Option<String>,
}

/// 一次会话跑完之后的落点。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceOutcomeOut {
    /// 正常跑完，附会话产出。
    Reported {
        /// 操作标识，供取消。
        operation_id: String,
        /// 会话产出。
        report: VoiceReportOut,
    },
    /// 中途降级到打字练习。
    Degraded {
        /// 操作标识。
        operation_id: String,
        /// 降级落点。
        fallback: TypedFallbackOut,
    },
}

/// 跑一次语音跟读会话，事件走 [`Channel`]。
///
/// **采集与识别全程在会话自己的线程上**（`start_session` 用 `start_operation`），本命令
/// 只在 async 侧排空事件队列；队列空时 `await` 一次短睡眠而不是自旋，与 `appreciate_poem`
/// 同一形态。
#[tauri::command]
pub(crate) async fn voice_start_session<R: Runtime>(
    app: AppHandle<R>,
    request: VoiceSessionRequest,
    on_event: Channel<Event<VoiceProgressOut, VoiceItemOut>>,
) -> IpcResult<VoiceOutcomeOut> {
    let operation_id = request
        .operation_id
        .clone()
        .unwrap_or_else(new_operation_id);
    let demonstrate = request.demonstrate;
    let poem_id = request.poem_id;
    let partial_channel = on_event.clone();
    let partials: PartialSink = Arc::new(move |partial| {
        let _ = partial_channel.send(Event::Item(VoiceItemOut::AsrPartial(partial)));
    });

    let prepared = blocking(app.clone(), move |state| {
        let config = state.config();
        let rig = state.voice_rig();
        if let Practice::Typed { reason, message } = rig.probe(&config) {
            return Ok(Prepared::Blocked(TypedFallbackOut::new(reason, message, 0)));
        }
        let body = rig.body(&config, &poem_id)?;
        let script =
            SessionScript::from_poem(&body).ok_or_else(|| "这首作品没有可跟读的句子".to_owned())?;
        let coupling = rig.couple(&config, partials)?;
        Ok(Prepared::Ready {
            plan: SessionPlan {
                demonstrate,
                ..SessionPlan::guided(script, config.voice.session)
            },
            coupling,
        })
    })
    .await?;

    let (plan, coupling) = match prepared {
        Prepared::Blocked(fallback) => {
            let _ = on_event.send(Event::Progress(VoiceProgressOut {
                stage: VoiceStageOut::Degraded {
                    fallback: fallback.clone(),
                },
                completed_lines: 0,
                total_lines: 0,
            }));
            let _ = on_event.send(Event::Item(VoiceItemOut::Fallback(fallback.clone())));
            let _ = on_event.send(Event::Done);
            return Ok(VoiceOutcomeOut::Degraded {
                operation_id,
                fallback,
            });
        }
        Prepared::Ready { plan, coupling } => (plan, coupling),
    };

    let handle = Arc::new(start_session(
        BoxedDemonstrator(coupling.demonstrator),
        BoxedListener(coupling.listener),
        plan,
    ));
    let _registration = register_operation(&app, operation_id.clone(), Arc::clone(&handle));

    let mut report = None;
    let mut fallback = None;
    loop {
        if let Some(event) = next_event(&handle, 0) {
            let out = map_session_event(event);
            match &out {
                Event::Item(VoiceItemOut::Report(value)) => report = Some(value.clone()),
                Event::Item(VoiceItemOut::Fallback(value)) => fallback = Some(value.clone()),
                _ => {}
            }
            let terminal = out.is_terminal();
            if let Err(error) = on_event.send(out) {
                cancel(&handle);
                return Err(format!("发送语音会话事件失败：{error}"));
            }
            if terminal {
                break;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    if let Some(fallback) = fallback {
        return Ok(VoiceOutcomeOut::Degraded {
            operation_id,
            fallback,
        });
    }
    let report = report.ok_or_else(|| "语音会话结束但没有产出".to_owned())?;
    Ok(VoiceOutcomeOut::Reported {
        operation_id,
        report,
    })
}

fn map_session_event(
    event: Event<SessionProgress, SessionItem>,
) -> Event<VoiceProgressOut, VoiceItemOut> {
    match event {
        Event::Progress(progress) => Event::Progress(VoiceProgressOut {
            stage: VoiceStageOut::from_stage(&progress.stage),
            completed_lines: progress.completed_lines,
            total_lines: progress.total_lines,
        }),
        Event::Item(item) => Event::Item(session_item(&item)),
        Event::Done => Event::Done,
        Event::Cancelled => Event::Cancelled,
        Event::Failed { message } => Event::Failed { message },
    }
}

enum Prepared {
    Blocked(TypedFallbackOut),
    Ready {
        plan: SessionPlan,
        coupling: Coupling,
    },
}

struct BoxedDemonstrator(Box<dyn Demonstrator + Send>);

impl Demonstrator for BoxedDemonstrator {
    fn demonstrate(&mut self, line: &str) -> Result<Demonstration, yunjian_voice::VoiceError> {
        self.0.demonstrate(line)
    }
}

struct BoxedListener(Box<dyn Listener + Send>);

impl Listener for BoxedListener {
    fn listen(
        &mut self,
        line: &str,
        plan: &RecognitionPlan,
    ) -> Result<LineTake, yunjian_voice::VoiceError> {
        self.0.listen(line, plan)
    }
}

/// 取模型的请求。
#[derive(Debug, Deserialize)]
pub(crate) struct VoiceFetchModelRequest {
    name: String,
    #[serde(default)]
    operation_id: Option<String>,
}

/// 取模型完成后的落点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceModelOutcomeOut {
    /// 已就位。
    Ready {
        /// 操作标识。
        operation_id: String,
        /// 模型名。
        name: String,
        /// 本地目录。
        directory: String,
    },
    /// 取不到，附这一条失败独有的原因。
    Unavailable {
        /// 操作标识。
        operation_id: String,
        /// 降级落点。
        fallback: TypedFallbackOut,
    },
}

/// 按需取一个语音模型，边取边报进度，可经 `cancel_operation` 取消。
///
/// 取消能真的打断一次几百兆的传输，靠的是 `stop` 探针被下压到写入端：模型层的
/// `Transport` 把字节写进一个受探针看守的 `Write`，探针为真时那次 `write` 直接报错，
/// 于是既有的「失败不留文件」路径接手清理。只在两次进度回调之间检查取消是另一条路，
/// 而那条路在服务端一次发来几兆时会拖很久才响应。
#[tauri::command]
pub(crate) async fn voice_fetch_model<R: Runtime>(
    app: AppHandle<R>,
    request: VoiceFetchModelRequest,
    on_event: Channel<Event<ModelFetchOut, Value>>,
) -> IpcResult<VoiceModelOutcomeOut> {
    let operation_id = request
        .operation_id
        .clone()
        .unwrap_or_else(new_operation_id);
    let name = request.name;
    let (config, rig) = {
        let state = app.state::<AppState>();
        (state.config(), state.voice_rig())
    };
    let fetch_name = name.clone();
    let reported: Arc<Mutex<Option<TypedFallbackOut>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&reported);

    let handle = Arc::new(start_operation(
        move |reporter: OperationReporter<ModelFetchOut, Value>| {
            let stop = || reporter.is_cancelled() || reporter.is_closed();
            let outcome = rig.fetch_model(&config, &fetch_name, &stop, &mut |progress| {
                reporter.progress(progress);
            });
            match outcome {
                Ok(directory) => {
                    reporter.item(Value::String(directory.display().to_string()));
                    Ok(())
                }
                Err(fallback) => {
                    let out = TypedFallbackOut::from_fallback(&fallback);
                    let message = out.message.clone();
                    *sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(out);
                    Err(message)
                }
            }
        },
    ));
    let _registration = register_operation(&app, operation_id.clone(), Arc::clone(&handle));

    let mut directory = None;
    let mut failed = false;
    loop {
        if let Some(event) = next_event(&handle, 0) {
            if let Event::Item(Value::String(path)) = &event {
                directory = Some(path.clone());
            }
            failed |= matches!(event, Event::Failed { .. } | Event::Cancelled);
            let cancelled = matches!(event, Event::Cancelled);
            let terminal = event.is_terminal();
            if let Err(error) = on_event.send(event) {
                cancel(&handle);
                return Err(format!("发送模型下载事件失败：{error}"));
            }
            if cancelled {
                let mut stored = reported
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if stored.is_none() {
                    *stored = Some(TypedFallbackOut::new(
                        DegradeReason::ModelUnavailable,
                        "模型下载已取消；语音练习需要它，可稍后重试，或改走打字练习。".to_owned(),
                        0,
                    ));
                }
            }
            if terminal {
                break;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    if failed {
        let fallback = reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_else(|| {
                TypedFallbackOut::new(
                    DegradeReason::ModelUnavailable,
                    "模型下载失败且没有给出原因；已切换到打字练习。".to_owned(),
                    0,
                )
            });
        return Ok(VoiceModelOutcomeOut::Unavailable {
            operation_id,
            fallback,
        });
    }
    let directory = directory.ok_or_else(|| "模型下载结束但没有落地目录".to_owned())?;
    Ok(VoiceModelOutcomeOut::Ready {
        operation_id,
        name,
        directory,
    })
}

#[cfg(test)]
mod tests;
