//! 语音接线的断言。
//!
//! # 这些用例为什么必须真的把命令调起来
//!
//! 只用类型断言证明命令返回了一个 `Future`，命令**外壳一行都没执行过**——todo 64 已经
//! 实测到那种做法让覆盖率反而下降（88.40% → 84.05%）。[`tauri::test::mock_builder`] 给出
//! 跑在 `MockRuntime` 上的真 `App`，不需要 GTK 也不需要 WebView2，于是外壳可以按 WebView
//! 驱动它的方式被驱动一次，而这回答了类型检查回答不了的问题：把闭包挪到工作线程之后，
//! `AppHandle` 还能不能解出被管理的状态。
//!
//! # 假装置不是「让界面看起来能用」的替身
//!
//! [`FakeRig`] 不做任何合成、采集或识别，但它**如实遵守协议**：示范音的标记落在
//! [`MARK_TIMEBASE_HZ`] 上、聆听器经 sink 吐出偏置转写、失败时给出具体原因码。所以这些
//! 用例验的是接线，不是内核——内核那一半在 `yunjian-voice` 自己的测试里。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::{App, AppHandle, Manager};
use yunjian_core::Config;
use yunjian_voice::VoiceError;
use yunjian_voice::permission::{DegradeReason, MicPermission, PermissionState, Practice, explain};
use yunjian_voice::platform::Platform;
use yunjian_voice::prosody::{
    FootAudio, FootMark, FootSynthesizer, Prosody, Reading, Segmentation, segment_metrical, splice,
};
use yunjian_voice::recognize::{Prompt, PromptReason, RecognitionPlan};
use yunjian_voice::session::{
    Demonstration, Demonstrator, LineTake, Listener, SpeechTimeline, TypedFallback,
};

use super::{
    ASR_PARTIAL_NOTE, AsrPartialOut, Coupling, MARK_TIMEBASE_HZ, ModelFetchOut, PartialSink,
    VoiceDemonstrateRequest, VoiceFetchModelRequest, VoiceModelOutcomeOut, VoiceOutcomeOut,
    VoiceRig, VoiceSessionRequest, WIRE_DEGRADE_REASONS, degrade_reason_key, voice_availability,
    voice_demonstrate, voice_fetch_model, voice_start_session,
};
use crate::ipc::AppState;

/// 注入的哨兵偏置串。
///
/// 选一串**绝不可能出自真实识别**的字符，于是「它出现在哪个载荷里」是一条确定的判据。
/// 语音路径的偏置假设只允许流到诊断区；它一旦出现在会话产出里，就意味着有人把转写接进了
/// 分数，而那正是 2026-08-11 裁决禁止的事。
const SENTINEL_BIASED: &str = "ZZ-偏置哨兵-QX";

/// 测试用的四句五言。公有领域，正文毫无争议。
const BODY: &str = "床前明月光，疑是地上霜。举头望明月，低头思故乡。";

/// 假聆听器每一行占用的时长。用来证明会话不占 async 运行时。
const LISTEN_DELAY: Duration = Duration::from_millis(200);

// ---------------------------------------------------------------------------
// 假装置
// ---------------------------------------------------------------------------

struct FakeSynth;

impl FootSynthesizer for FakeSynth {
    fn synthesize_foot(&mut self, text: &str) -> Result<FootAudio, VoiceError> {
        Ok(FootAudio {
            samples: vec![0.5; text.chars().count() * 4_000],
            sample_rate: 16_000,
        })
    }
}

fn body_segmentation() -> Segmentation {
    segment_metrical(yunjian_core::derive::split_metrical_lines(BODY))
}

fn splice_body(body: &str) -> Reading {
    splice(
        &mut FakeSynth,
        &segment_metrical(yunjian_core::derive::split_metrical_lines(body)),
        Prosody::CLASSICAL,
    )
    .expect("假合成器的采样率恒定，拼接不会失败")
}

/// 示范器交出去的标记，供断言比对「线上收到的与装置发出的逐项相同」。
type MarkLog = Arc<Mutex<Vec<Vec<FootMark>>>>;

struct FakeDemonstrator {
    emitted: MarkLog,
}

impl Demonstrator for FakeDemonstrator {
    fn demonstrate(&mut self, line: &str) -> Result<Demonstration, VoiceError> {
        let reading = splice(
            &mut FakeSynth,
            &segment_metrical(std::iter::once(line)),
            Prosody::CLASSICAL,
        )?;
        let rate = reading.sample_rate as usize;
        let marks: Vec<FootMark> = reading
            .marks
            .iter()
            .map(|mark| FootMark {
                line: mark.line,
                index_in_line: mark.index_in_line,
                text: mark.text.clone(),
                start_sample: mark.start_sample * 1000 / rate,
                end_sample: mark.end_sample * 1000 / rate,
            })
            .collect();
        self.emitted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(marks.clone());
        Ok(Demonstration {
            marks,
            duration_ms: u64::try_from(reading.duration().as_millis()).unwrap_or(u64::MAX),
        })
    }
}

struct FakeListener {
    partials: PartialSink,
    line: AtomicUsize,
    fail_at: Option<usize>,
}

impl Listener for FakeListener {
    fn listen(&mut self, line: &str, plan: &RecognitionPlan) -> Result<LineTake, VoiceError> {
        let index = self.line.fetch_add(1, Ordering::Relaxed);
        (self.partials)(AsrPartialOut::new(
            120,
            Some(format!("无偏置-{index}")),
            Some(format!("{SENTINEL_BIASED}-{}", plan.reference)),
        ));
        std::thread::sleep(LISTEN_DELAY);
        if self.fail_at == Some(index) {
            return Err(VoiceError::NoInputDevice {
                detail: "假装置注入的设备掉线".to_owned(),
            });
        }
        Ok(LineTake {
            timeline: SpeechTimeline {
                onsets_ms: vec![0, 400, 800],
                long_pause_count: 1,
                total_ms: 1_200,
                spoke: true,
            },
            prompts: vec![Prompt {
                next_chars: line.chars().take(2).collect(),
                from_index: 0,
                at_ms: 900,
                reason: PromptReason::TrailingSilence,
            }],
        })
    }
}

enum FetchBehaviour {
    Succeeds,
    WaitsForCancel,
}

struct FakeRig {
    practice: Practice,
    marks: MarkLog,
    listener_fails_at: Option<usize>,
    fetch: FetchBehaviour,
}

impl FakeRig {
    fn granted() -> Self {
        Self {
            practice: Practice::Voice,
            marks: Arc::default(),
            listener_fails_at: None,
            fetch: FetchBehaviour::Succeeds,
        }
    }

    fn denied() -> Self {
        let permission =
            MicPermission::new(Platform::Linux, PermissionState::Denied, "假装置注入的拒绝");
        Self {
            practice: yunjian_voice::permission::decide(&permission),
            ..Self::granted()
        }
    }
}

impl VoiceRig for FakeRig {
    fn probe(&self, _config: &Config) -> Practice {
        self.practice.clone()
    }

    fn body(&self, _config: &Config, _poem_id: &str) -> crate::ipc::IpcResult<String> {
        Ok(BODY.to_owned())
    }

    fn read(&self, _config: &Config, body: &str) -> crate::ipc::IpcResult<Reading> {
        Ok(splice_body(body))
    }

    fn couple(&self, _config: &Config, partials: PartialSink) -> crate::ipc::IpcResult<Coupling> {
        Ok(Coupling {
            demonstrator: Box::new(FakeDemonstrator {
                emitted: Arc::clone(&self.marks),
            }),
            listener: Box::new(FakeListener {
                partials,
                line: AtomicUsize::new(0),
                fail_at: self.listener_fails_at,
            }),
        })
    }

    fn fetch_model(
        &self,
        _config: &Config,
        name: &str,
        stop: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(ModelFetchOut),
    ) -> Result<std::path::PathBuf, TypedFallback> {
        match self.fetch {
            FetchBehaviour::Succeeds => {
                for done in [0u64, 512, 1_024] {
                    progress(ModelFetchOut::Downloading {
                        bytes_done: done,
                        bytes_total: 1_024,
                    });
                }
                progress(ModelFetchOut::Verifying { bytes: 1_024 });
                progress(ModelFetchOut::Verified);
                progress(ModelFetchOut::Unpacking);
                Ok(std::path::PathBuf::from("/tmp/yunjian-fake-models").join(name))
            }
            FetchBehaviour::WaitsForCancel => {
                let started = Instant::now();
                let mut done = 0u64;
                while !stop() && started.elapsed() < Duration::from_secs(5) {
                    done += 64;
                    progress(ModelFetchOut::Downloading {
                        bytes_done: done,
                        bytes_total: 1_048_576,
                    });
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(TypedFallback::new(
                    DegradeReason::ModelUnavailable,
                    Some(Platform::Linux),
                    0,
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 支架
// ---------------------------------------------------------------------------

fn scratch_config(name: &str) -> Config {
    let root = std::env::temp_dir().join(format!("yunjian-voice-ipc-{name}"));
    let mut config = Config::default();
    config.app.data_dir = root.join("data");
    config.corpus.data_dir = root.join("corpus");
    config.voice.model_dir = root.join("models");
    config
}

fn app_with(name: &str, rig: FakeRig) -> (App<MockRuntime>, MarkLog) {
    let marks = Arc::clone(&rig.marks);
    let app = crate::ipc::configure_builder(mock_builder(), scratch_config(name))
        .build(mock_context(noop_assets()))
        .expect("MockRuntime 下可构建应用");
    app.state::<AppState>().install_voice_rig(Arc::new(rig));
    (app, marks)
}

/// 记录 [`Channel`] 上真正投递出去的 JSON。
///
/// 收成 `Value` 而不是反序列化回 Rust 类型，是刻意的：本文件最要紧的一条断言是
/// 「哨兵串不出现在产出载荷里」，而那要在**线上的字节**上判，不是在一个可能把未知字段
/// 丢掉的反序列化结果上判。
fn recorder<T>() -> (Channel<T>, Arc<Mutex<Vec<Value>>>) {
    let log: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(source) = body else {
            panic!("Channel 载荷必须是 JSON");
        };
        sink.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(serde_json::from_str(&source)?);
        Ok(())
    });
    (channel, log)
}

fn delivered(log: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    log.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn items<'a>(events: &'a [Value], item: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["type"] == "item")
        .map(|event| &event["payload"])
        .filter(|payload| payload["item"] == item)
        .collect()
}

fn run_session(app: &AppHandle<MockRuntime>, demonstrate: bool) -> (VoiceOutcomeOut, Vec<Value>) {
    let (channel, log) = recorder();
    let request = VoiceSessionRequest {
        poem_id: "any".to_owned(),
        demonstrate,
        operation_id: None,
    };
    let outcome =
        tauri::async_runtime::block_on(voice_start_session(app.clone(), request, channel))
            .expect("会话命令不应以 Err 结束：降级也是一种正常落点");
    (outcome, delivered(&log))
}

// ---------------------------------------------------------------------------
// 验收断言
// ---------------------------------------------------------------------------

/// 高亮事件带的时间戳数量必须与合成音步数一致。
///
/// 两向都验：既比对「线上收到的与示范器发出的逐项相同」，也比对「总数等于真实切分的音步
/// 数」。只验后者的话，一个把每行标记都塞成同一份的实现照样能过。
#[test]
fn highlight_marks_match_the_synthesis_segment_count() {
    let (app, emitted) = app_with("marks", FakeRig::granted());
    let (outcome, events) = run_session(&app.handle().clone(), true);
    assert!(matches!(outcome, VoiceOutcomeOut::Reported { .. }));

    let emitted = emitted
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let expected_total = body_segmentation().foot_count();
    assert_eq!(
        emitted.iter().map(Vec::len).sum::<usize>(),
        expected_total,
        "假示范器发出的音步总数应等于真实切分的音步数"
    );

    let demonstrated = items(&events, "demonstrated");
    assert_eq!(
        demonstrated.len(),
        emitted.len(),
        "每一行示范都要有一条高亮事件"
    );
    let delivered_total: usize = demonstrated
        .iter()
        .map(|item| item["marks"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(
        delivered_total, expected_total,
        "线上收到的时间戳数量必须等于合成音步数"
    );

    for (item, line_marks) in demonstrated.iter().zip(emitted.iter()) {
        let marks = item["marks"].as_array().expect("marks 是数组");
        assert_eq!(marks.len(), line_marks.len());
        for (out, source) in marks.iter().zip(line_marks.iter()) {
            assert_eq!(out["text"], Value::from(source.text.clone()));
            assert_eq!(out["start_ms"], Value::from(source.start_sample as u64));
            assert_eq!(out["end_ms"], Value::from(source.end_sample as u64));
            assert!(
                out["end_ms"].as_u64() > out["start_ms"].as_u64(),
                "每个音步都要有正的时长，否则高亮会瞬间跳过"
            );
        }
    }
}

/// ASR 部分假设必须经 [`Channel`] 到达，而不是 Tauri event。
#[test]
fn asr_partials_arrive_over_the_channel() {
    let (app, _) = app_with("partials", FakeRig::granted());
    let (_, events) = run_session(&app.handle().clone(), false);

    let partials = items(&events, "asr_partial");
    assert!(
        !partials.is_empty(),
        "聆听器每一行都吐了一条部分假设，Channel 上必须能看到"
    );
    for partial in &partials {
        assert_eq!(partial["diagnostics_only"], Value::Bool(true));
        assert_eq!(partial["note"], Value::from(ASR_PARTIAL_NOTE));
    }
}

/// 流式路径不得改用 Tauri event 或 `eval`。
#[test]
fn voice_streaming_never_uses_events_or_eval() {
    let production = production_source();
    assert!(
        production.contains("ipc::Channel"),
        "流式数据必须通过 ipc::Channel"
    );
    assert!(
        !production.contains(".emit("),
        "流式路径不得使用 Tauri event"
    );
    assert!(!production.contains(".eval("), "不得通过 eval 传输数据");
}

/// 模拟权限被拒时，界面状态必须切到打字模式**并带上那一条独有的原因**。
#[test]
fn a_denied_microphone_switches_to_typed_practice_with_the_reason() {
    let (app, _) = app_with("denied", FakeRig::denied());
    let (outcome, events) = run_session(&app.handle().clone(), true);

    let VoiceOutcomeOut::Degraded { fallback, .. } = outcome else {
        panic!("权限被拒必须降级，而不是给出一个产出");
    };
    assert_eq!(
        fallback.reason,
        degrade_reason_key(DegradeReason::PermissionDenied)
    );
    assert_eq!(
        fallback.message,
        explain(DegradeReason::PermissionDenied, Some(Platform::Linux)),
        "解释必须原样取自 permission::explain，不在这一层另编一句"
    );
    assert!(fallback.message.contains("打字练习"));

    let stages: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "progress")
        .map(|event| &event["payload"]["stage"])
        .collect();
    assert!(
        stages.iter().any(|stage| stage["stage"] == "degraded"
            && stage["fallback"]["reason"] == degrade_reason_key(DegradeReason::PermissionDenied)),
        "进度流里必须出现携带原因的降级状态：{stages:?}"
    );
    assert!(
        items(&events, "report").is_empty(),
        "降级之后不得再给出任何产出"
    );
}

/// 中途设备掉线时降级，且**已完成的行数不清零**。
#[test]
fn a_mid_session_device_loss_degrades_without_losing_progress() {
    let (app, _) = app_with(
        "device-loss",
        FakeRig {
            listener_fails_at: Some(2),
            ..FakeRig::granted()
        },
    );
    let (outcome, events) = run_session(&app.handle().clone(), false);

    let VoiceOutcomeOut::Degraded { fallback, .. } = outcome else {
        panic!("设备掉线必须降级");
    };
    assert_eq!(
        fallback.reason,
        degrade_reason_key(DegradeReason::NoInputDevice)
    );
    assert_eq!(
        fallback.completed_lines, 2,
        "掉线之前已复诵完的行数不得清零"
    );
    assert_eq!(items(&events, "line_observed").len(), 2);
}

/// 显示的最终产出必须来自无偏置路径：注入的偏置哨兵串一次都不得出现在产出载荷里。
///
/// **两向都验。** 只断言「产出里没有哨兵」是可以被一个从不注入哨兵的装置骗过的，所以先
/// 断言哨兵确实到达了诊断项，再断言它没进产出。
#[test]
fn the_final_report_never_carries_the_biased_hypothesis() {
    let (app, _) = app_with("unbiased", FakeRig::granted());
    let (outcome, events) = run_session(&app.handle().clone(), false);

    let partials = items(&events, "asr_partial");
    assert!(
        partials.iter().any(|partial| {
            partial["biased"]
                .as_str()
                .is_some_and(|text| text.contains(SENTINEL_BIASED))
        }),
        "哨兵串必须真的被注入过，否则下面那条断言是空的"
    );

    let reports = items(&events, "report");
    assert_eq!(reports.len(), 1, "一次会话恰好一份产出");
    let report = serde_json::to_string(reports[0]).expect("产出可序列化");
    assert!(
        !report.contains(SENTINEL_BIASED),
        "偏置假设绝不得进入产出载荷：{report}"
    );

    let VoiceOutcomeOut::Reported { report: value, .. } = outcome else {
        panic!("正常跑完必须给出产出");
    };
    let returned = serde_json::to_string(&value).expect("产出可序列化");
    assert!(
        !returned.contains(SENTINEL_BIASED),
        "命令返回值同样不得携带偏置假设：{returned}"
    );
}

/// 产出载荷的**键集**被冻结：没有任何可以塞进转写的字符串字段。
///
/// 这条比「不含哨兵串」更强：它挡住的是「加了一个 `transcript` 字段但这次恰好为空」。
#[test]
fn report_payload_keys_are_frozen() {
    let (app, _) = app_with("frozen", FakeRig::granted());
    let (_, events) = run_session(&app.handle().clone(), false);
    let report = items(&events, "report")[0].clone();
    let object = report.as_object().expect("产出是对象");

    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "coherence",
            "coherence_label",
            "duration_ratio",
            "gap_variance_ms2",
            "item",
            "lines_attempted",
            "long_pause_count",
            "prompt_count",
            "relative_rhythm",
            "spoke",
        ],
        "产出的键集是冻结的：新增任何一个能承载转写的字段都必须先推翻这条断言"
    );

    for (key, value) in object {
        if let Some(text) = value.as_str() {
            assert!(
                matches!(key.as_str(), "item" | "coherence_label" | "relative_rhythm"),
                "产出里唯一允许的字符串是标签与枚举，`{key}` = {text} 不在其中"
            );
        }
    }
}

/// 会话跑着的时候另一条命令仍能完成。
///
/// # 这一条能证明什么，不能证明什么（实测结论，不是推测）
///
/// 它证明：会话的采集与识别**发生在调用它的那个任务之外**，所以并发的
/// `voice_availability` 不必排在整场会话后面。
///
/// 它**不能**证明「没有阻塞运行时」。实测过：把本命令轮询循环里的
/// `tokio::time::sleep` 换成 `std::thread::sleep`（一个真的阻塞运行时线程的缺陷），
/// 这条断言**照样通过**——Tauri 的 async 运行时是多线程的，堵住一个 worker 不会让另一个
/// 任务饿死。所以「不阻塞」这件事由两条别的断言守着：
/// [`every_voice_command_is_async`]（同步命令的函数体跑在 WebView 主线程上）与
/// [`async_command_bodies_never_block_the_runtime`]（阻塞式睡眠的源码守卫，已验证会变红）。
///
/// 保留这一条是因为它验的是另一件事：事件排空循环没有把整场会话变成一次不可中断的等待。
#[test]
fn a_running_session_does_not_serialize_other_commands() {
    let (app, _) = app_with("nonblocking", FakeRig::granted());
    let handle = app.handle().clone();
    let probe = app.handle().clone();
    let (channel, _log) = recorder();

    tauri::async_runtime::block_on(async move {
        let session = tauri::async_runtime::spawn(voice_start_session(
            handle,
            VoiceSessionRequest {
                poem_id: "any".to_owned(),
                demonstrate: false,
                operation_id: None,
            },
            channel,
        ));
        let started = Instant::now();
        voice_availability(probe).await.expect("可用性查询不应失败");
        let waited = started.elapsed();
        let outcome = session.await.expect("会话任务不应 panic");
        assert!(matches!(
            outcome.expect("会话不应以 Err 结束"),
            VoiceOutcomeOut::Reported { .. }
        ));
        assert!(
            waited < Duration::from_millis(400),
            "会话占用了 {}ms 才让另一条命令返回；采集与识别必须在自己的线程上",
            waited.as_millis()
        );
    });
}

/// 示范命令给出可播放地址与逐音步时间戳，**音频本体不经命令返回值**。
#[test]
fn demonstration_returns_a_url_and_one_mark_per_foot() {
    let (app, _) = app_with("demo", FakeRig::granted());
    let out = tauri::async_runtime::block_on(voice_demonstrate(
        app.handle().clone(),
        VoiceDemonstrateRequest {
            poem_id: "any".to_owned(),
        },
    ))
    .expect("示范命令应成功");

    assert!(
        out.audio_url.starts_with("yunjian-audio://"),
        "示范音必须经自定义 URI 协议交付：{}",
        out.audio_url
    );
    assert_eq!(out.marks.len(), body_segmentation().foot_count());
    assert!(out.duration_ms > 0);
    assert_eq!(out.sample_rate, 16_000);
}

/// 语音不可用时示范按钮给出具体原因，而不是一段静音。
#[test]
fn demonstration_refuses_with_the_specific_reason_when_voice_is_unavailable() {
    let (app, _) = app_with("demo-denied", FakeRig::denied());
    let error = tauri::async_runtime::block_on(voice_demonstrate(
        app.handle().clone(),
        VoiceDemonstrateRequest {
            poem_id: "any".to_owned(),
        },
    ))
    .expect_err("语音不可用时不得给出示范音");
    assert_eq!(
        error,
        explain(DegradeReason::PermissionDenied, Some(Platform::Linux))
    );
}

/// 模型下载报进度并给出落地目录。
#[test]
fn model_download_reports_progress_and_lands() {
    let (app, _) = app_with("fetch", FakeRig::granted());
    let (channel, log) = recorder();
    let outcome = tauri::async_runtime::block_on(voice_fetch_model(
        app.handle().clone(),
        VoiceFetchModelRequest {
            name: "vits-melo-tts-zh_en".to_owned(),
            operation_id: None,
        },
        channel,
    ))
    .expect("下载命令应成功");

    let VoiceModelOutcomeOut::Ready { directory, .. } = outcome else {
        panic!("假装置声明成功，命令必须报就位");
    };
    assert!(directory.ends_with("vits-melo-tts-zh_en"));

    let events = delivered(&log);
    let stages: Vec<&str> = events
        .iter()
        .filter(|event| event["type"] == "progress")
        .filter_map(|event| event["payload"]["stage"].as_str())
        .collect();
    // **不断言收到了哪几段。** 进度按协议是可合并快照：`OperationReporter::progress` 覆盖
    // 待取值，于是一个比消费者快的生产者会让中间段被合并掉——这正是协议允许的行为，
    // 断言「必须看到 downloading」等于断言一个时序竞态。能断言的是：进度确实到了，
    // 且每一段都是四个已知取值之一（写错串会让界面收到一个它没有文案的阶段）。
    assert!(!stages.is_empty(), "至少要有一段进度到达界面");
    for stage in &stages {
        assert!(
            matches!(
                *stage,
                "downloading" | "verifying" | "verified" | "unpacking"
            ),
            "未知的进度阶段串 `{stage}`"
        );
    }
}

/// 四段进度的线上串与模型层的枚举逐项对应。
///
/// 与上一条分工明确：上一条验「进度到得了界面」（受可合并协议约束，只能验存在性），
/// 这一条验「映射没写错」（纯函数，可逐项断言）。合成一条会让其中一半失去判据。
#[test]
fn every_fetch_stage_maps_onto_a_distinct_wire_string() {
    let mapped = [
        (
            yunjian_voice::models::FetchProgress::Downloading {
                bytes_done: 7,
                bytes_total: 9,
            },
            "downloading",
        ),
        (
            yunjian_voice::models::FetchProgress::Verifying { bytes: 9 },
            "verifying",
        ),
        (yunjian_voice::models::FetchProgress::Verified, "verified"),
        (yunjian_voice::models::FetchProgress::Unpacking, "unpacking"),
    ];
    for (source, expected) in mapped {
        let value = serde_json::to_value(ModelFetchOut::from(source)).expect("可序列化");
        assert_eq!(value["stage"], Value::from(expected));
    }
    let downloading = serde_json::to_value(ModelFetchOut::from(
        yunjian_voice::models::FetchProgress::Downloading {
            bytes_done: 7,
            bytes_total: 9,
        },
    ))
    .expect("可序列化");
    assert_eq!(downloading["bytes_done"], Value::from(7));
    assert_eq!(downloading["bytes_total"], Value::from(9));
}

/// 模型下载可取消，且取消之后给出**具体原因**而不是一句「失败了」。
#[test]
fn model_download_is_cancellable() {
    let (app, _) = app_with(
        "fetch-cancel",
        FakeRig {
            fetch: FetchBehaviour::WaitsForCancel,
            ..FakeRig::granted()
        },
    );
    let handle = app.handle().clone();
    let canceller = app.handle().clone();
    let (channel, log) = recorder();
    let watched = Arc::clone(&log);

    let outcome = tauri::async_runtime::block_on(async move {
        let task = tauri::async_runtime::spawn(voice_fetch_model(
            handle,
            VoiceFetchModelRequest {
                name: "vits-melo-tts-zh_en".to_owned(),
                operation_id: Some("fetch-op".to_owned()),
            },
            channel,
        ));
        let started = Instant::now();
        while delivered(&watched).is_empty() && started.elapsed() < Duration::from_secs(3) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(crate::ipc::cancel_operation(
            canceller,
            "fetch-op".to_owned()
        ));
        task.await.expect("下载任务不应 panic")
    })
    .expect("取消是正常落点，不该以 Err 结束");

    let VoiceModelOutcomeOut::Unavailable { fallback, .. } = outcome else {
        panic!("取消之后不得报就位");
    };
    assert_eq!(
        fallback.reason,
        degrade_reason_key(DegradeReason::ModelUnavailable)
    );
    assert!(!fallback.message.is_empty(), "取消也要说清下一步做什么");

    let events = delivered(&log);
    assert!(
        events.iter().any(|event| event["type"] == "cancelled"),
        "取消必须在流上可见：{events:?}"
    );
}

// ---------------------------------------------------------------------------
// 源码守卫
// ---------------------------------------------------------------------------

fn production_source() -> String {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/voice_ipc.rs"),
    )
    .expect("读取语音 IPC 源码");
    source
        .split("#[cfg(test)]\nmod ")
        .next()
        .expect("存在生产源码")
        .to_owned()
}

const VOICE_COMMANDS: &[&str] = &[
    "voice_availability",
    "voice_demonstrate",
    "voice_start_session",
    "voice_fetch_model",
];

/// 四条语音命令都必须是 `async`。同步命令的函数体在 WebView 主线程上跑，采集与识别放进去
/// 就是一次窗口冻结，而那种冻结在开发机上看不见——数据小、机器快。
#[test]
fn every_voice_command_is_async() {
    let source = production_source();
    for command in VOICE_COMMANDS {
        assert!(
            source.contains(&format!("async fn {command}")),
            "语音命令 `{command}` 必须是 async"
        );
    }
}

/// 命令不得返回 `Vec<u8>`：实测 6.3 MB 的音频当返回值序列化会在线上膨胀到 22.5 MB。
#[test]
fn no_voice_command_returns_audio_as_a_byte_vector() {
    let source = production_source();
    assert!(
        !source.lines().any(|line| {
            line.contains("async fn") && (line.contains("Vec<u8>") || line.contains("Vec < u8 >"))
        }),
        "命令不得返回 Vec<u8>；音频必须通过自定义 URI 协议读取"
    );
}

/// async 命令体内不得出现阻塞式睡眠。
///
/// `std::thread::sleep` 会占住一个运行时 worker 而不是把它让出去。多线程运行时下这不会
/// 立刻表现为界面冻结（[`a_running_session_does_not_serialize_other_commands`] 的文档记了
/// 这次实测），但它会在运行时线程数被占满时突然变成一次真的冻结，而那时症状与原因隔得
/// 很远。**这条守卫已验证会为该缺陷变红。**
#[test]
fn async_command_bodies_never_block_the_runtime() {
    let source = production_source();
    assert!(
        !source.contains("std::thread::sleep"),
        "async 命令体内不得阻塞运行时；让出线程请用 tokio::time::sleep"
    );
}

/// `spawn_blocking` 的闭包里不得 await，也不得构造嵌套运行时。
///
/// 前者会把长任务钉在阻塞线程池上并让 drop 取消失效，后者会造出第二个运行时。
#[test]
fn blocking_workers_neither_await_nor_nest_a_runtime() {
    let source = production_source();
    for body in source.split("blocking(").skip(1) {
        let closure = body.split(".await").next().expect("存在闭包体");
        assert!(
            !closure.contains("Runtime::new") && !closure.contains("Builder::new_"),
            "阻塞闭包不得构造嵌套 async runtime"
        );
    }
}

/// 十条降级原因各有一个互不相同的线上串。
#[test]
fn every_degrade_reason_has_a_distinct_wire_key() {
    let mut keys: Vec<&str> = WIRE_DEGRADE_REASONS
        .iter()
        .copied()
        .map(degrade_reason_key)
        .collect();
    keys.sort_unstable();
    let count = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), count, "原因码的线上串必须互不相同");
}

/// 标记时基是毫秒。写成断言而不是只写在注释里：换算比例错了会让高亮整体拉偏。
#[test]
fn marks_use_a_millisecond_timebase() {
    assert_eq!(MARK_TIMEBASE_HZ, 1_000);
}
