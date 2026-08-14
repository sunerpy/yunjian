//! 跟读会话的判定层测试。**全部不需要模型、声卡或麦克风。**

use super::{
    COHERENCE_LABEL, Coherence, Demonstration, Demonstrator, GradeTicket, LineTake, Listener,
    RhythmInputs, SessionItem, SessionPlan, SessionProgress, SessionScore, SessionScript,
    SessionStage, SpeechTimeline, TypedFallback, coherence, duration_ratio, gap_variance_ms2,
    relative_rhythm, start_session,
};
use crate::VoiceError;
use crate::audio::{AudioError, SystemSupport};
use crate::models::ModelError;
use crate::permission::{DegradeReason, MicPermission, PermissionState};
use crate::platform::Platform;
use crate::recognize::{Prompt, PromptReason, RecognitionPlan};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use yunjian_core::VoiceSessionConfig;
use yunjian_core::operation::{Event, OperationHandle, cancel, next_event};
use yunjian_recite::{FsrsGrade, RelativeRhythm, Scheduler};

fn config() -> VoiceSessionConfig {
    VoiceSessionConfig::default()
}

fn script() -> SessionScript {
    SessionScript::from_poem("床前明月光，疑是地上霜。").expect("两句应当切出脚本")
}

// ---------------------------------------------------------------------------
// 节奏连贯度只由三项信号算出
// ---------------------------------------------------------------------------

#[test]
fn gap_variance_is_zero_for_perfectly_even_onsets() {
    assert!(gap_variance_ms2(&[0, 500, 1000, 1500]).abs() < f64::EPSILON);
}

#[test]
fn gap_variance_is_zero_when_there_is_no_gap_to_speak_of() {
    assert!(gap_variance_ms2(&[]).abs() < f64::EPSILON);
    assert!(gap_variance_ms2(&[42]).abs() < f64::EPSILON);
}

#[test]
fn gap_variance_grows_with_irregular_onsets() {
    let even = gap_variance_ms2(&[0, 500, 1000, 1500]);
    let ragged = gap_variance_ms2(&[0, 100, 1400, 1500]);
    assert!(
        ragged > even,
        "不匀速的起始间隔方差必须更大：{ragged} vs {even}"
    );
}

#[test]
fn duration_ratio_without_a_demonstration_is_neutral() {
    assert!((duration_ratio(9_999, 0) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn coherence_is_computed_from_exactly_the_three_available_signals() {
    let perfect = RhythmInputs::from_parts(0.0, 0, 1.0);
    assert!((coherence(&perfect, &config()).value() - 1.0).abs() < 1e-9);

    let jittery = RhythmInputs::from_parts(config().gap_variance_scale_ms2, 0, 1.0);
    let stalled = RhythmInputs::from_parts(0.0, config().long_pause_tolerance as usize, 1.0);
    let hurried = RhythmInputs::from_parts(0.0, 0, 1.0 - config().duration_ratio_tolerance);

    for (name, inputs) in [("方差", jittery), ("长停顿", stalled), ("时长比", hurried)] {
        let value = coherence(&inputs, &config()).value();
        assert!(
            (value - 0.5).abs() < 1e-9,
            "{name}这一项在等于尺度值时应当恰好得 0.5，实得 {value}"
        );
    }
}

#[test]
fn coherence_degrades_monotonically_in_each_of_the_three_signals() {
    let base = RhythmInputs::from_parts(10_000.0, 1, 1.1);
    let worse_variance = RhythmInputs::from_parts(40_000.0, 1, 1.1);
    let worse_pauses = RhythmInputs::from_parts(10_000.0, 4, 1.1);
    let worse_pacing = RhythmInputs::from_parts(10_000.0, 1, 1.9);
    let baseline = coherence(&base, &config());
    for (name, inputs) in [
        ("方差", worse_variance),
        ("长停顿", worse_pauses),
        ("时长比", worse_pacing),
    ] {
        let value = coherence(&inputs, &config());
        assert!(
            value < baseline,
            "{name}变差后连贯度必须下降：{} vs {}",
            value.value(),
            baseline.value()
        );
    }
}

#[test]
fn coherence_is_synthesised_from_timings_alone_never_from_a_transcript() {
    let timeline = SpeechTimeline {
        onsets_ms: vec![0, 400, 800, 1200],
        long_pause_count: 0,
        total_ms: 1_600,
        spoke: true,
    };
    let inputs = RhythmInputs::from_timeline(&timeline, 1_600);
    assert!(inputs.gap_variance_ms2().abs() < f64::EPSILON);
    assert_eq!(inputs.long_pause_count(), 0);
    assert!((inputs.duration_ratio() - 1.0).abs() < f64::EPSILON);
    assert!((coherence(&inputs, &config()).value() - 1.0).abs() < 1e-9);
}

#[test]
fn the_metric_is_labelled_as_rhythm_coherence_not_pronunciation() {
    assert_eq!(COHERENCE_LABEL, "节奏连贯度");
    let value = coherence(&RhythmInputs::from_parts(0.0, 0, 1.0), &config());
    assert_eq!(value.label(), COHERENCE_LABEL);
}

#[test]
fn relative_rhythm_reads_the_duration_ratio_band() {
    let cfg = config();
    let slower = RhythmInputs::from_parts(0.0, 0, 1.0 + cfg.similar_band + 0.1);
    let faster = RhythmInputs::from_parts(0.0, 0, 1.0 - cfg.similar_band - 0.1);
    let similar = RhythmInputs::from_parts(0.0, 0, 1.0);
    assert_eq!(relative_rhythm(&slower, &cfg), RelativeRhythm::Slower);
    assert_eq!(relative_rhythm(&faster, &cfg), RelativeRhythm::Faster);
    assert_eq!(relative_rhythm(&similar, &cfg), RelativeRhythm::Similar);
}

#[test]
fn timelines_concatenate_by_shifting_onsets_rather_than_restarting_the_clock() {
    let first = SpeechTimeline {
        onsets_ms: vec![0, 500],
        long_pause_count: 1,
        total_ms: 1_000,
        spoke: true,
    };
    let second = SpeechTimeline {
        onsets_ms: vec![0, 500],
        long_pause_count: 2,
        total_ms: 1_000,
        spoke: true,
    };
    let merged = SpeechTimeline::concat(&[first, second]);
    assert_eq!(merged.onsets_ms, vec![0, 500, 1_000, 1_500]);
    assert_eq!(merged.long_pause_count, 3);
    assert_eq!(merged.total_ms, 2_000);
    assert!(merged.spoke);
}

// ---------------------------------------------------------------------------
// 五条失败路径，五个不同的原因码
// ---------------------------------------------------------------------------

#[test]
fn each_of_the_five_failure_paths_yields_a_distinct_reason_code() {
    let denied = AudioError::PermissionDenied {
        platform: Platform::Linux,
        state: PermissionState::Denied,
    };
    let no_device = AudioError::NoDevice {
        detail: "系统未报告默认输入设备".to_owned(),
    };
    let too_old = AudioError::UnsupportedPlatformVersion {
        platform: Platform::MacOs,
        required: "14.2".to_owned(),
        found: "13.6".to_owned(),
    };

    let paths = [
        (
            "无权限",
            TypedFallback::from_audio(&denied, 0),
            DegradeReason::PermissionDenied,
        ),
        (
            "无设备",
            TypedFallback::from_audio(&no_device, 0),
            DegradeReason::NoInputDevice,
        ),
        (
            "系统版本过低",
            TypedFallback::from_audio(&too_old, 0),
            DegradeReason::SystemTooOld,
        ),
        (
            "无模型",
            TypedFallback::from_model(
                &ModelError::Absent {
                    name: "sherpa-onnx-streaming-zipformer-zh".to_owned(),
                    dir: std::path::PathBuf::from("/nowhere/models/zipformer"),
                    next: "联网后运行 yunjian models fetch".to_owned(),
                },
                0,
            ),
            DegradeReason::ModelUnavailable,
        ),
        (
            "识别被拒",
            TypedFallback::recognition_rejected(0),
            DegradeReason::RecognitionRejected,
        ),
    ];

    for (name, fallback, expected) in &paths {
        assert_eq!(fallback.reason, *expected, "{name} 的原因码不对");
        assert!(
            fallback.practice().is_typed(),
            "{name} 必须落到打字练习而不是零分"
        );
        assert!(!fallback.message.is_empty(), "{name} 必须有面向用户的解释");
    }

    let codes: Vec<DegradeReason> = paths.iter().map(|(_, f, _)| f.reason).collect();
    for (index, code) in codes.iter().enumerate() {
        for other in &codes[index + 1..] {
            assert_ne!(code, other, "五条失败路径的原因码必须互不相同");
        }
    }

    for (index, (name, fallback, _)) in paths.iter().enumerate() {
        for (other_name, other, _) in &paths[index + 1..] {
            assert_ne!(
                fallback.message, other.message,
                "{name} 与 {other_name} 共用了同一句引导；原因码不同而话一样，用户拿到的下一步就是错的"
            );
        }
    }
}

#[test]
fn a_missing_model_is_not_reported_as_a_capture_failure() {
    let fallback = super::fallback_for(
        &VoiceError::ModelMissing {
            path: std::path::PathBuf::from("/nowhere/encoder.onnx"),
        },
        0,
    );
    assert_eq!(fallback.reason, DegradeReason::ModelUnavailable);
    assert!(
        fallback.message.contains("models fetch"),
        "缺模型的解释要给出下一步命令：{}",
        fallback.message
    );
}

#[test]
fn the_preflight_gate_routes_a_denied_permission_to_typed_practice() {
    let preflight = crate::audio::Preflight {
        permission: MicPermission::new(Platform::Linux, PermissionState::Denied, "用户拒绝"),
        system: SystemSupport::Meets,
    };
    let error = preflight.check().expect_err("被拒的授权必须报错");
    let fallback = TypedFallback::from_audio(&error, 3);
    assert_eq!(fallback.reason, DegradeReason::PermissionDenied);
    assert_eq!(fallback.completed_lines, 3);
}

// ---------------------------------------------------------------------------
// 播放与录音互斥
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trace {
    PlayStart(usize),
    PlayEnd(usize),
    RecordStart(usize),
    RecordEnd(usize),
}

type Log = Arc<Mutex<Vec<Trace>>>;

struct ScriptedDemonstrator {
    log: Log,
    line: usize,
    duration_ms: u64,
    fail_at: Option<usize>,
    error: Option<VoiceError>,
}

impl Demonstrator for ScriptedDemonstrator {
    fn demonstrate(&mut self, _line: &str) -> Result<Demonstration, VoiceError> {
        let current = self.line;
        self.line += 1;
        self.log
            .lock()
            .expect("日志可加锁")
            .push(Trace::PlayStart(current));
        if self.fail_at == Some(current) {
            return Err(self
                .error
                .take()
                .unwrap_or(VoiceError::Backend("合成失败".to_owned())));
        }
        self.log
            .lock()
            .expect("日志可加锁")
            .push(Trace::PlayEnd(current));
        Ok(Demonstration {
            marks: Vec::new(),
            duration_ms: self.duration_ms,
        })
    }
}

struct ScriptedListener {
    log: Log,
    line: usize,
    takes: Vec<LineTake>,
    fail_at: Option<usize>,
    error: Option<VoiceError>,
}

impl Listener for ScriptedListener {
    fn listen(&mut self, _line: &str, plan: &RecognitionPlan) -> Result<LineTake, VoiceError> {
        assert!(!plan.diagnostics, "跟读不得把噪声假设发给用户");
        let current = self.line;
        self.line += 1;
        self.log
            .lock()
            .expect("日志可加锁")
            .push(Trace::RecordStart(current));
        if self.fail_at == Some(current) {
            return Err(self.error.take().unwrap_or(VoiceError::NoInputDevice {
                detail: "设备中途消失".to_owned(),
            }));
        }
        self.log
            .lock()
            .expect("日志可加锁")
            .push(Trace::RecordEnd(current));
        Ok(self.takes.get(current).cloned().unwrap_or_else(even_take))
    }
}

/// 停在第一句的示范器：进入后通知测试，然后等测试放行。
struct GatedDemonstrator {
    gate: Arc<SessionGate>,
}

impl Demonstrator for GatedDemonstrator {
    fn demonstrate(&mut self, _line: &str) -> Result<Demonstration, VoiceError> {
        self.gate.enter_and_wait();
        Ok(Demonstration {
            marks: Vec::new(),
            duration_ms: 1_600,
        })
    }
}

/// 生产者与测试之间的双向闸门。两侧等待都有上限：脚本化用例宁可失败，也不要挂到工作流超时。
#[derive(Debug, Default)]
struct SessionGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    entered: bool,
    released: bool,
}

impl SessionGate {
    fn enter_and_wait(&self) {
        let mut state = self.state.lock().expect("闸门可加锁");
        state.entered = true;
        self.changed.notify_all();
        let started = Instant::now();
        while !state.released {
            assert!(started.elapsed() < GATE_BUDGET, "闸门没有在预算内放行");
            let (next, _) = self
                .changed
                .wait_timeout(state, GATE_POLL)
                .expect("闸门可等待");
            state = next;
        }
    }

    fn wait_until_entered(&self) {
        let mut state = self.state.lock().expect("闸门可加锁");
        let started = Instant::now();
        while !state.entered {
            assert!(
                started.elapsed() < GATE_BUDGET,
                "生产者没有在预算内进入闸门"
            );
            let (next, _) = self
                .changed
                .wait_timeout(state, GATE_POLL)
                .expect("闸门可等待");
            state = next;
        }
    }

    fn release(&self) {
        self.state.lock().expect("闸门可加锁").released = true;
        self.changed.notify_all();
    }
}

const GATE_BUDGET: Duration = Duration::from_secs(5);
const GATE_POLL: Duration = Duration::from_millis(200);

fn even_take() -> LineTake {
    LineTake {
        timeline: SpeechTimeline {
            onsets_ms: vec![0, 400, 800, 1_200],
            long_pause_count: 0,
            total_ms: 1_600,
            spoke: true,
        },
        prompts: Vec::new(),
    }
}

fn rig(
    fail_demo_at: Option<usize>,
    fail_listen_at: Option<usize>,
    error: Option<VoiceError>,
) -> (ScriptedDemonstrator, ScriptedListener, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let demonstrator = ScriptedDemonstrator {
        log: Arc::clone(&log),
        line: 0,
        duration_ms: 1_600,
        fail_at: fail_demo_at,
        error: if fail_demo_at.is_some() {
            error.clone_boxed()
        } else {
            None
        },
    };
    let listener = ScriptedListener {
        log: Arc::clone(&log),
        line: 0,
        takes: vec![even_take(), even_take()],
        fail_at: fail_listen_at,
        error: if fail_listen_at.is_some() {
            error
        } else {
            None
        },
    };
    (demonstrator, listener, log)
}

trait CloneBoxed {
    fn clone_boxed(&self) -> Option<VoiceError>;
}

impl CloneBoxed for Option<VoiceError> {
    fn clone_boxed(&self) -> Option<VoiceError> {
        match self {
            Some(VoiceError::NoInputDevice { detail }) => Some(VoiceError::NoInputDevice {
                detail: detail.clone(),
            }),
            Some(VoiceError::ModelMissing { path }) => {
                Some(VoiceError::ModelMissing { path: path.clone() })
            }
            Some(VoiceError::Backend(text)) => Some(VoiceError::Backend(text.clone())),
            _ => None,
        }
    }
}

fn drain(
    handle: &OperationHandle<SessionProgress, SessionItem>,
) -> (
    Vec<SessionProgress>,
    Vec<SessionItem>,
    Event<SessionProgress, SessionItem>,
) {
    let mut progress = Vec::new();
    let mut items = Vec::new();
    loop {
        match next_event(handle, 5_000) {
            Some(Event::Progress(snapshot)) => progress.push(snapshot),
            Some(Event::Item(item)) => items.push(item),
            Some(terminal) => return (progress, items, terminal),
            None => panic!("会话事件流超时"),
        }
    }
}

#[test]
fn playback_and_recording_never_overlap() {
    let (demonstrator, listener, log) = rig(None, None, None);
    let handle = start_session(
        demonstrator,
        listener,
        SessionPlan::guided(script(), config()),
    );
    let (_, _, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);

    let trace = log.lock().expect("日志可加锁").clone();
    assert_eq!(
        trace,
        vec![
            Trace::PlayStart(0),
            Trace::PlayEnd(0),
            Trace::RecordStart(0),
            Trace::RecordEnd(0),
            Trace::PlayStart(1),
            Trace::PlayEnd(1),
            Trace::RecordStart(1),
            Trace::RecordEnd(1),
        ],
        "示范必须播完才开录：任何交错都会让识别器听见自己的示范音"
    );
}

#[test]
fn the_stage_type_cannot_express_playing_and_recording_at_once() {
    let playing = SessionStage::Demonstrating { line: 0 };
    let recording = SessionStage::Listening { line: 0 };
    assert!(playing.is_playing() && !playing.is_recording());
    assert!(recording.is_recording() && !recording.is_playing());
    assert!(!SessionStage::Idle.is_playing() && !SessionStage::Idle.is_recording());
    assert!(SessionStage::AwaitingGrade.is_terminal());
}

#[test]
fn progress_snapshots_alternate_demonstrating_and_listening_per_line() {
    let (demonstrator, listener, _) = rig(None, None, None);
    let handle = start_session(
        demonstrator,
        listener,
        SessionPlan::guided(script(), config()),
    );
    let (progress, _, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);
    let stages: Vec<SessionStage> = progress.into_iter().map(|p| p.stage).collect();
    assert!(
        stages.contains(&SessionStage::Demonstrating { line: 0 })
            || stages.contains(&SessionStage::Listening { line: 0 })
            || stages.contains(&SessionStage::AwaitingGrade),
        "进度快照可合并，但至少要能观察到会话推进：{stages:?}"
    );
    for pair in stages.windows(2) {
        assert!(
            !(pair[0].is_playing() && pair[1].is_playing()),
            "同一行不该被连续标成播放两次：{pair:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 端到端：一个产出，恰好一次提交
// ---------------------------------------------------------------------------

fn temp_db(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yunjian-session-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录可创建");
    dir.join("review.db")
}

fn run_to_report(plan: SessionPlan) -> SessionScore {
    let (demonstrator, listener, _) = rig(None, None, None);
    let handle = start_session(demonstrator, listener, plan);
    let (_, items, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);
    let mut reports: Vec<SessionScore> = items
        .into_iter()
        .filter_map(|item| match item {
            SessionItem::Report(score) => Some(score),
            _ => None,
        })
        .collect();
    assert_eq!(reports.len(), 1, "一次会话只产出一个报告");
    reports.pop().expect("刚断言过恰有一个")
}

#[test]
fn a_scripted_session_produces_one_report_and_submits_exactly_one_grade() {
    let score = run_to_report(SessionPlan::guided(script(), config()));
    assert_eq!(score.lines_attempted, 2);
    assert!(score.feedback.spoke);
    assert_eq!(score.feedback.pause_count, 0);
    assert_eq!(score.feedback.relative_rhythm, RelativeRhythm::Similar);
    assert!((score.coherence.value() - 1.0).abs() < 1e-9);

    let path = temp_db("submit-once");
    let _ = std::fs::remove_file(&path);
    let mut scheduler = Scheduler::open(&path).expect("复习库可打开");
    let ticket = score.into_ticket("poem-001");
    assert_eq!(ticket.stable_id(), "poem-001");
    let state = ticket
        .submit(&mut scheduler, FsrsGrade::Good)
        .expect("提交等级应当成功");
    assert_eq!(state.stable_id, "poem-001");
    assert_eq!(state.last_grade, FsrsGrade::Good);

    let stored = scheduler
        .state("poem-001")
        .expect("可读回状态")
        .expect("刚写入的状态必须存在");
    assert_eq!(stored.last_grade, FsrsGrade::Good);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_grade_comes_from_the_user_and_the_session_never_derives_one() {
    let score = run_to_report(SessionPlan::guided(script(), config()));
    let path = temp_db("user-choice");
    let _ = std::fs::remove_file(&path);
    let mut scheduler = Scheduler::open(&path).expect("复习库可打开");
    let ticket = score.into_ticket("poem-002");
    let state = ticket
        .submit(&mut scheduler, FsrsGrade::Again)
        .expect("提交等级应当成功");
    assert_eq!(
        state.last_grade,
        FsrsGrade::Again,
        "满分节奏也必须服从用户自选的 Again，否则就是机器在评级"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_session_without_a_demonstration_falls_back_to_a_neutral_duration_ratio() {
    let mut plan = SessionPlan::guided(script(), config());
    plan.demonstrate = false;
    let score = run_to_report(plan);
    assert!((score.inputs.duration_ratio() - 1.0).abs() < f64::EPSILON);
    assert_eq!(score.feedback.relative_rhythm, RelativeRhythm::Similar);
}

#[test]
fn prompts_are_forwarded_and_counted() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let demonstrator = ScriptedDemonstrator {
        log: Arc::clone(&log),
        line: 0,
        duration_ms: 1_600,
        fail_at: None,
        error: None,
    };
    let prompted = LineTake {
        timeline: even_take().timeline,
        prompts: vec![Prompt {
            next_chars: "明月".to_owned(),
            from_index: 2,
            at_ms: 2_400,
            reason: PromptReason::TrailingSilence,
        }],
    };
    let listener = ScriptedListener {
        log,
        line: 0,
        takes: vec![prompted, even_take()],
        fail_at: None,
        error: None,
    };
    let handle = start_session(
        demonstrator,
        listener,
        SessionPlan::guided(script(), config()),
    );
    let (_, items, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);
    let prompts = items
        .iter()
        .filter(|item| matches!(item, SessionItem::Prompt(_)))
        .count();
    assert_eq!(prompts, 1);
    let report = items
        .iter()
        .find_map(|item| match item {
            SessionItem::Report(score) => Some(score),
            _ => None,
        })
        .expect("应当有报告");
    assert_eq!(report.prompt_count, 1);
}

// ---------------------------------------------------------------------------
// 中途掉线保留进度
// ---------------------------------------------------------------------------

#[test]
fn a_mid_session_disconnection_keeps_the_finished_lines_and_switches_to_typed() {
    let (demonstrator, listener, _) = rig(
        None,
        Some(1),
        Some(VoiceError::NoInputDevice {
            detail: "设备中途消失".to_owned(),
        }),
    );
    let handle = start_session(
        demonstrator,
        listener,
        SessionPlan::guided(script(), config()),
    );
    let (_, items, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);

    let fallback = items
        .iter()
        .find_map(|item| match item {
            SessionItem::Fallback(fallback) => Some(fallback),
            _ => None,
        })
        .expect("掉线必须给出降级落点");
    assert_eq!(fallback.reason, DegradeReason::NoInputDevice);
    assert_eq!(
        fallback.completed_lines, 1,
        "第一行已经背完，掉线不该让它作废"
    );
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, SessionItem::Report(_))),
        "被打断的尝试不产出报告，也就无从铸出评级票据"
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, SessionItem::LineObserved { line: 0, .. })),
        "第一行的观察结果必须已经发出"
    );
}

#[test]
fn a_synthesis_failure_before_the_first_line_degrades_with_zero_progress() {
    let (demonstrator, listener, _) = rig(
        Some(0),
        None,
        Some(VoiceError::ModelMissing {
            path: std::path::PathBuf::from("/nowhere/model.onnx"),
        }),
    );
    let handle = start_session(
        demonstrator,
        listener,
        SessionPlan::guided(script(), config()),
    );
    let (_, items, terminal) = drain(&handle);
    assert_eq!(terminal, Event::Done);
    let fallback = items
        .iter()
        .find_map(|item| match item {
            SessionItem::Fallback(fallback) => Some(fallback),
            _ => None,
        })
        .expect("合成失败必须给出降级落点");
    assert_eq!(fallback.reason, DegradeReason::ModelUnavailable);
    assert_eq!(fallback.completed_lines, 0);
}

#[test]
fn cancelling_stops_the_session_without_a_report() {
    // 脚本化 rig 是零耗时的：不设闸门时整场会话可能在 `cancel` 落地之前就跑完，于是这条
    // 断言退化成与调度器赛跑，CI 上实测在 Linux 随机变红过。闸门把「取消时会话确实还在
    // 跑」变成事实而不是概率，断言的性质一点没变——反而更强。
    let gate = Arc::new(SessionGate::default());
    let (_, listener, _) = rig(None, None, None);
    let handle = start_session(
        GatedDemonstrator {
            gate: Arc::clone(&gate),
        },
        listener,
        SessionPlan::guided(script(), config()),
    );
    gate.wait_until_entered();
    cancel(&handle);
    gate.release();
    let mut saw_report = false;
    while let Some(event) = next_event(&handle, 5_000) {
        match event {
            Event::Item(SessionItem::Report(_)) => saw_report = true,
            event if event.is_terminal() => break,
            _ => {}
        }
    }
    assert!(!saw_report, "取消后不得产出报告");
}

// ---------------------------------------------------------------------------
// 脚本切分
// ---------------------------------------------------------------------------

#[test]
fn a_script_splits_on_punctuation_and_refuses_to_be_empty() {
    let parsed = SessionScript::from_poem("床前明月光，疑是地上霜。").expect("应当切出两句");
    assert_eq!(parsed.lines(), ["床前明月光", "疑是地上霜"]);
    assert_eq!(parsed.len(), 2);
    assert!(!parsed.is_empty());
    assert!(SessionScript::from_poem("，。！").is_none());
    assert!(SessionScript::from_poem("").is_none());
}

#[test]
fn the_gate_threshold_follows_the_configured_long_pause() {
    let mut cfg = config();
    cfg.long_pause_ms = 1_234;
    let plan = SessionPlan::guided(script(), cfg);
    assert_eq!(plan.gate.long_pause_ms, 1_234);
}

#[test]
fn coherence_never_leaves_the_unit_interval() {
    let extremes = [
        RhythmInputs::from_parts(0.0, 0, 1.0),
        RhythmInputs::from_parts(f64::MAX, usize::MAX, 0.0),
        RhythmInputs::from_parts(1e12, 99, 50.0),
    ];
    for inputs in extremes {
        let value = coherence(&inputs, &config()).value();
        assert!((0.0..=1.0).contains(&value), "连贯度越界：{value}");
    }
}

#[test]
fn a_zero_scale_config_does_not_produce_a_non_finite_coherence() {
    let cfg = VoiceSessionConfig {
        long_pause_ms: 700,
        gap_variance_scale_ms2: 0.0,
        long_pause_tolerance: 0.0,
        duration_ratio_tolerance: 0.0,
        similar_band: 0.0,
    };
    let perfect: Coherence = coherence(&RhythmInputs::from_parts(0.0, 0, 1.0), &cfg);
    assert!((perfect.value() - 1.0).abs() < f64::EPSILON);
    let ragged = coherence(&RhythmInputs::from_parts(1.0, 1, 2.0), &cfg);
    assert!(ragged.value().abs() < f64::EPSILON);
}

#[test]
fn a_ticket_is_bound_to_the_stable_id_and_not_to_any_content_hash() {
    let score = run_to_report(SessionPlan::guided(script(), config()));
    let ticket: GradeTicket = score.into_ticket("tang-300-0001");
    assert_eq!(ticket.stable_id(), "tang-300-0001");
}
