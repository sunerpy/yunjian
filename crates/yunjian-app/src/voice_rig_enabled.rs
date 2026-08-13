//! 带 `voice` 特性时的生产装置：真实合成、真实采集、真实双路识别、真实模型下载。
//!
//! # 三条设计取舍
//!
//! **一、`Demonstration` 的标记在这里就换算成毫秒。** 会话协议只带样本下标与整行时长，
//! 不带采样率，而合成引擎的采样率不是常量（MeloTTS 与 Kokoro 不同）。所以示范器在返回
//! 之前把标记换到 [`MARK_TIMEBASE_HZ`]，IPC 层因此不需要猜一个采样率——猜错会把整首诗的
//! 高亮时刻整体拉偏，而那种偏差在换引擎的那一天才会出现。
//!
//! **二、聆听器先录满一段再喂给识别器，而不是边录边解。** 采集侧现成可用的入口是
//! [`yunjian_voice::capture::capture_default`]（定长采集，Linux 上已实测），而
//! [`yunjian_voice::recognize::start_recognition`] 只要一个 [`PcmSource`]。把已录到的
//! 缓冲切成帧喂进去，卡顿判定与能量门控的时序算术完全不变——它们只看每帧的 RMS 与帧长，
//! 不看帧是从设备来的还是从缓冲来的。**真正不能变的是「播放与录音不重叠」**，那一条由
//! 会话状态机保证：示范返回即代表播放结束。
//!
//! **三、录音窗口按字数推出而不是等用户按停。** 一次复诵的上限取
//! `每字 800 ms + 2 s 余量`：短于此会把慢读者截断（截断在语音路径上等价于「漏读」，
//! 而那正是本项目拒绝报告的东西），长于此则每一句都要干等几秒静音。
//!
//! # 取消能真的打断一次几百兆的下载
//!
//! 探针被下压到**写入端**：[`StopTransport`] 把底层传输的字节写进一个受探针看守的
//! `Write`，探针为真时那次 `write` 直接报错，于是模型层既有的「失败不留文件」路径接手
//! 清理。只在两次进度回调之间检查取消是另一条路，而服务端一次发来几兆时那条路会拖很久
//! 才响应。

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use yunjian_core::Config;
use yunjian_core::operation::{Event, next_event};
use yunjian_voice::asr::streaming::{StreamingDualDecoder, TransducerFiles};
use yunjian_voice::models::{Bz2TarUnpacker, FetchProgress, ModelCache, ModelError, Transport};
use yunjian_voice::permission::{
    DegradeReason, MicPermission, PermissionState, Practice, degrade, explain,
};
use yunjian_voice::platform::Platform;
use yunjian_voice::prosody::{
    FootMark, Prosody, Reading, Segmentation, segment_metrical, splice as splice_reading,
};
use yunjian_voice::recognize::{
    Hotwords, OnlineDecodeConfig, PcmSource, RecognitionItem, RecognitionOutcome, RecognitionPlan,
    start_recognition,
};
use yunjian_voice::session::{
    Demonstration, Demonstrator, LineTake, Listener, SpeechTimeline, TypedFallback,
};
use yunjian_voice::tts::Synthesizer;
use yunjian_voice::{VoiceError, audio, capture};

use super::{
    AsrPartialOut, Coupling, MARK_TIMEBASE_HZ, ModelFetchOut, PartialSink, VoiceRig, corpus_body,
};
use crate::ipc::IpcResult;

/// 缺省流式识别模型。**必须是流式 Transducer**：`models.toml` 里唯一许可通过的流式条目
/// 就是它，而 Whisper 是 encoder-decoder，喂不进 `SherpaOnnxOnlineRecognizer`。
const DEFAULT_ASR_MODEL: &str = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20";

/// 缺省合成模型。
const DEFAULT_TTS_MODEL: &str = "vits-melo-tts-zh_en";

/// 一次复诵的录音窗口：每字这么多毫秒。
const MS_PER_CHAR: u64 = 800;

/// 一次复诵的录音窗口余量，毫秒。
const TAIL_MS: u64 = 2_000;

/// 喂给识别器的帧长，毫秒。
const FRAME_MS: u64 = 100;

pub(crate) struct ProductionRig;

fn asr_model(config: &Config) -> &str {
    config
        .voice
        .asr_model
        .as_deref()
        .unwrap_or(DEFAULT_ASR_MODEL)
}

fn tts_model(config: &Config) -> &str {
    config
        .voice
        .tts_model
        .as_deref()
        .unwrap_or(DEFAULT_TTS_MODEL)
}

/// 桌面三平台的授权状态。
///
/// Linux 没有系统级麦克风门，Windows 与 macOS 在进程首次触达输入设备时由系统弹窗——三者
/// 都不需要外壳先申请，所以这里报 `Granted`，真正的拒绝由采集失败经
/// [`audio::classify_capture_error`] 判定出来。Android 与 iOS 的授权**只能**由原生外壳
/// 发起（`Gate::needs_shell_code`），在外壳接上之前报 `Undetermined` 是唯一诚实的取值。
fn permission_of(platform: Platform) -> MicPermission {
    let (gate, _) = MicPermission::contract(platform);
    let state = if gate.needs_shell_code() {
        PermissionState::Undetermined
    } else {
        PermissionState::Granted
    };
    MicPermission::new(
        platform,
        state,
        "桌面外壳未单独申请：系统在首次触达设备时弹窗",
    )
}

impl VoiceRig for ProductionRig {
    fn probe(&self, config: &Config) -> Practice {
        let Some(platform) = Platform::current() else {
            return degrade(DegradeReason::CaptureFailed, None);
        };
        if let Err(error) = audio::Preflight::new(permission_of(platform)).check() {
            return error.practice();
        }
        match capture::list_inputs() {
            Ok(names) if names.is_empty() => {
                return degrade(DegradeReason::NoInputDevice, Some(platform));
            }
            Ok(_) => {}
            Err(error) => {
                return audio::classify_capture_error(&error, 0).practice();
            }
        }
        let cache = ModelCache::at(config.voice.model_dir.clone());
        for name in [asr_model(config), tts_model(config)] {
            if !cache.is_present(name) {
                return Practice::Typed {
                    reason: DegradeReason::ModelUnavailable,
                    message: format!(
                        "语音模型 `{name}` 尚未就位。{}",
                        explain(DegradeReason::ModelUnavailable, Some(platform))
                    ),
                };
            }
        }
        Practice::Voice
    }

    fn body(&self, config: &Config, poem_id: &str) -> IpcResult<String> {
        corpus_body(config, poem_id)
    }

    fn read(&self, config: &Config, body: &str) -> IpcResult<Reading> {
        let mut synthesizer = open_synthesizer(config)?;
        splice_reading(&mut synthesizer, &segment_body(body), prosody_of(config))
            .map_err(|error| error.to_string())
    }

    fn couple(&self, config: &Config, partials: PartialSink) -> IpcResult<Coupling> {
        let synthesizer = open_synthesizer(config)?;
        let files = TransducerFiles::discover(
            &ModelCache::at(config.voice.model_dir.clone()).model_dir(asr_model(config)),
            false,
        )
        .map_err(|error| error.to_string())?;
        Ok(Coupling {
            demonstrator: Box::new(SynthesizedDemonstration {
                synthesizer,
                prosody: prosody_of(config),
            }),
            listener: Box::new(CapturingListener { files, partials }),
        })
    }

    fn fetch_model(
        &self,
        config: &Config,
        name: &str,
        stop: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(ModelFetchOut),
    ) -> Result<PathBuf, TypedFallback> {
        let transport = yunjian_voice::models::HttpTransport::default();
        let guarded = StopTransport {
            inner: &transport,
            stop,
        };
        ModelCache::at(config.voice.model_dir.clone())
            .ensure_with(
                name,
                Some(&guarded),
                Some(&Bz2TarUnpacker),
                &mut |event: FetchProgress| progress(ModelFetchOut::from(event)),
            )
            .map_err(|error: ModelError| {
                TypedFallback::new(error.degrade_reason(), Platform::current(), 0)
            })
    }
}

fn prosody_of(config: &Config) -> Prosody {
    Prosody {
        foot_pause_ms: config.voice.prosody.foot_pause_ms,
        line_pause_ms: config.voice.prosody.line_pause_ms,
    }
}

fn segment_body(body: &str) -> Segmentation {
    segment_metrical(yunjian_core::derive::split_metrical_lines(body))
}

fn open_synthesizer(config: &Config) -> IpcResult<Synthesizer> {
    Synthesizer::new(&ModelCache::at(config.voice.model_dir.clone()).model_dir(tts_model(config)))
        .map_err(|error| error.to_string())
}

/// 把标记从合成器的采样率换算到 [`MARK_TIMEBASE_HZ`]。
fn rebase_marks(marks: &[FootMark], sample_rate: u32) -> Vec<FootMark> {
    let rate = if sample_rate == 0 {
        MARK_TIMEBASE_HZ
    } else {
        sample_rate
    };
    marks
        .iter()
        .map(|mark| FootMark {
            line: mark.line,
            index_in_line: mark.index_in_line,
            text: mark.text.clone(),
            start_sample: mark.start_sample * 1000 / rate as usize,
            end_sample: mark.end_sample * 1000 / rate as usize,
        })
        .collect()
}

struct SynthesizedDemonstration {
    synthesizer: Synthesizer,
    prosody: Prosody,
}

impl Demonstrator for SynthesizedDemonstration {
    fn demonstrate(&mut self, line: &str) -> Result<Demonstration, VoiceError> {
        let reading = splice_reading(
            &mut self.synthesizer,
            &segment_metrical(std::iter::once(line)),
            self.prosody,
        )?;
        let duration_ms = u64::try_from(reading.duration().as_millis()).unwrap_or(u64::MAX);
        Ok(Demonstration {
            marks: rebase_marks(&reading.marks, reading.sample_rate),
            duration_ms,
        })
    }
}

struct CapturingListener {
    files: TransducerFiles,
    partials: PartialSink,
}

/// 已录到的缓冲按定长帧喂给识别器。
///
/// 能量门控与卡顿判定只看每帧的 RMS 与帧长，因此「帧从设备来」还是「帧从缓冲来」对它们
/// 的时序算术没有差别。
struct BufferedPcm {
    samples: Vec<f32>,
    sample_rate: u32,
    cursor: usize,
    frame: usize,
}

impl PcmSource for BufferedPcm {
    fn next_frame(&mut self) -> Option<Vec<f32>> {
        if self.cursor >= self.samples.len() {
            return None;
        }
        let end = (self.cursor + self.frame).min(self.samples.len());
        let frame = self.samples[self.cursor..end].to_vec();
        self.cursor = end;
        Some(frame)
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

fn listen_window(line: &str) -> Duration {
    let chars = line.chars().filter(|c| c.is_alphabetic()).count() as u64;
    Duration::from_millis(chars.saturating_mul(MS_PER_CHAR).saturating_add(TAIL_MS))
}

impl Listener for CapturingListener {
    fn listen(&mut self, line: &str, plan: &RecognitionPlan) -> Result<LineTake, VoiceError> {
        let recorded = capture::capture_default(listen_window(line))?;
        let sample_rate = recorded.sample_rate;
        let frame = usize::try_from(u64::from(sample_rate) * FRAME_MS / 1000).unwrap_or(1600);
        let source = BufferedPcm {
            samples: recorded.samples,
            sample_rate,
            cursor: 0,
            frame: frame.max(1),
        };
        let hotwords = Hotwords::from_poem(&plan.reference);
        let decoder = StreamingDualDecoder::open(
            &self.files,
            OnlineDecodeConfig::unbiased(),
            hotwords.map(OnlineDecodeConfig::biased),
        )?;
        let mut diagnostic_plan = plan.clone();
        diagnostic_plan.diagnostics = true;
        let handle = start_recognition(source, decoder, diagnostic_plan);

        let mut prompts = Vec::new();
        let mut outcome: Option<RecognitionOutcome> = None;
        while let Some(event) = next_event(&handle, 5_000) {
            match event {
                Event::Item(RecognitionItem::Partial(hypothesis)) => {
                    (self.partials)(AsrPartialOut::new(
                        hypothesis.at_ms,
                        hypothesis
                            .unbiased
                            .as_ref()
                            .map(|hyp| hyp.as_str().to_owned()),
                        hypothesis
                            .biased
                            .as_ref()
                            .map(|hyp| hyp.as_str().to_owned()),
                    ));
                }
                Event::Item(RecognitionItem::Prompt(prompt)) => prompts.push(prompt),
                Event::Item(RecognitionItem::Outcome(value)) => outcome = Some(value),
                Event::Failed { message } => return Err(VoiceError::Backend(message)),
                Event::Progress(_) | Event::Done | Event::Cancelled => {}
            }
        }
        let outcome =
            outcome.ok_or_else(|| VoiceError::Backend("识别流结束但没有给出汇总".to_owned()))?;
        Ok(LineTake {
            timeline: SpeechTimeline::from_outcome(&outcome),
            prompts,
        })
    }
}

/// 受取消探针看守的传输。见模块文档「取消能真的打断一次几百兆的下载」。
struct StopTransport<'a> {
    inner: &'a dyn Transport,
    stop: &'a dyn Fn() -> bool,
}

impl Transport for StopTransport<'_> {
    fn fetch(
        &self,
        url: &str,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String> {
        let mut guarded = StopWriter {
            inner: sink,
            stop: self.stop,
        };
        self.inner.fetch(url, &mut guarded, progress)
    }
}

struct StopWriter<'a> {
    inner: &'a mut dyn Write,
    stop: &'a dyn Fn() -> bool,
}

impl Write for StopWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if (self.stop)() {
            return Err(std::io::Error::other("下载已取消"));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{
        DEFAULT_ASR_MODEL, DEFAULT_TTS_MODEL, StopWriter, listen_window, permission_of,
        rebase_marks,
    };
    use yunjian_voice::models::Registry;
    use yunjian_voice::platform::Platform;
    use yunjian_voice::prosody::FootMark;

    /// 两个缺省模型名必须真的在清单里且许可通过。**猜一个名字会在首次下载时才报错**，
    /// 而那时用户已经在等进度条了。
    #[test]
    fn default_model_names_are_admitted_by_the_manifest() {
        let registry = Registry::shipped().expect("随包清单可解析");
        for name in [DEFAULT_ASR_MODEL, DEFAULT_TTS_MODEL] {
            registry
                .admit(name)
                .unwrap_or_else(|error| panic!("缺省模型 `{name}` 未通过许可门禁：{error}"));
        }
    }

    /// 移动端的授权只能由原生外壳发起，桌面三平台由系统首次触达时弹窗。
    #[test]
    fn only_the_platforms_needing_shell_code_report_undetermined() {
        for platform in Platform::ALL {
            let permission = permission_of(platform);
            assert_eq!(
                permission.gate.needs_shell_code(),
                permission.state != yunjian_voice::permission::PermissionState::Granted,
                "{platform:?} 的授权状态与它的授权点不一致"
            );
        }
    }

    /// 换算之后的标记必须落在毫秒时基上，否则高亮会整体拉偏一个采样率的倍数。
    #[test]
    fn marks_are_rebased_onto_milliseconds() {
        let marks = vec![FootMark {
            line: 0,
            index_in_line: 0,
            text: "床前".to_owned(),
            start_sample: 0,
            end_sample: 44_100,
        }];
        let rebased = rebase_marks(&marks, 44_100);
        assert_eq!(rebased[0].end_sample, 1_000);
    }

    /// 录音窗口必须随字数增长，否则慢读者会被截断，而截断在语音路径上等价于漏读。
    #[test]
    fn listen_window_grows_with_the_line() {
        assert!(listen_window("床前明月光") > listen_window("床前"));
    }

    /// 探针为真时那次写入必须直接失败，而不是等到下一次进度回调。
    #[test]
    fn stop_probe_fails_the_write_immediately() {
        let mut buffer: Vec<u8> = Vec::new();
        let stop = || true;
        let mut guarded = StopWriter {
            inner: &mut buffer,
            stop: &stop,
        };
        assert!(guarded.write(b"payload").is_err());
        assert!(buffer.is_empty(), "取消之后不得留下任何字节");
    }
}
