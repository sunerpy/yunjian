//! [`crate::audio`] 的测试。
//!
//! **本机没有音频设备**，所以这里的策略是：把每一条可以在无设备条件下证明的事都真的
//! 证明掉（重采样、降混、四种失败的分类与降级、权限门的先后顺序、完成度判定），只把
//! 真正需要硬件的两条（真实采集、真实播放）留给 `capture` 特性下的测试，并且让它们在
//! 缺设备时**断言类型化的降级**而不是静默返回——静默跳过会让「从没真的录过音」看起来全绿。

#![expect(
    clippy::print_stderr,
    reason = "环境相关的两条测试要在日志里留下走了哪条分支：一条只说 ok 的绿色测试无法\
              区分「真的播放了」与「这台机器没有输出设备」"
)]

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::time::Duration;

use super::{
    AudioError, DeviceInfo, InputDevice, NativeFormat, Preflight, SystemSupport, TARGET_CHANNELS,
    TARGET_SAMPLE_RATE, capture_with, classify_capture_error, downmix, meets_completion,
    pick_input, rank_input, target_samples, to_target,
};
use crate::augment::resample;
use crate::permission::{DegradeReason, MicPermission, PermissionState};
use crate::platform::Platform;

/// 一台不存在的设备。存在的理由是可测性：重采样与完成度判定必须能在没有麦克风的机器上
/// 被验证，因此测试驱动一个假设备，而 [`capture_with`] 只认 [`InputDevice`] 接口。
struct FakeInput {
    info: DeviceInfo,
    queue: VecDeque<f32>,
}

impl FakeInput {
    fn new(native: NativeFormat, interleaved: Vec<f32>) -> Self {
        Self {
            info: DeviceInfo {
                name: format!("fake:{}Hz/{}ch", native.sample_rate, native.channels),
                native,
            },
            queue: interleaved.into(),
        }
    }

    /// 正弦音，所有声道同相。用正弦而不是随机噪声：重采样的正确性可以对着解析式比较，
    /// 而噪声只能比统计量。
    fn tone(native: NativeFormat, seconds: f32, hz: f32) -> Self {
        let frames = frames_of(native.sample_rate, seconds);
        let mut interleaved = Vec::with_capacity(frames * usize::from(native.channels));
        for frame in 0..frames {
            let value = sine(hz, native.sample_rate, frame);
            for _ in 0..native.channels {
                interleaved.push(value);
            }
        }
        Self::new(native, interleaved)
    }

    fn silent(native: NativeFormat, seconds: f32) -> Self {
        let frames = frames_of(native.sample_rate, seconds);
        Self::new(native, vec![0.0; frames * usize::from(native.channels)])
    }

    fn empty(native: NativeFormat) -> Self {
        Self::new(native, Vec::new())
    }
}

impl InputDevice for FakeInput {
    fn describe(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn pull(&mut self, max_samples: usize) -> Result<Vec<f32>, AudioError> {
        Ok(self
            .queue
            .drain(..max_samples.min(self.queue.len()))
            .collect())
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "测试信号的长度与相位都是小整数，不触及 f32 精度边界"
)]
fn frames_of(sample_rate: u32, seconds: f32) -> usize {
    (sample_rate as f32 * seconds) as usize
}

#[expect(
    clippy::cast_precision_loss,
    reason = "采样序号在测试信号长度内远小于 f32 的精确整数范围"
)]
fn sine(hz: f32, sample_rate: u32, index: usize) -> f32 {
    (TAU * hz * index as f32 / sample_rate as f32).sin()
}

fn granted(platform: Platform) -> Preflight {
    Preflight::new(MicPermission::new(platform, PermissionState::Granted, ""))
}

// ---------------------------------------------------------------------------
// 重采样：黄金对照
// ---------------------------------------------------------------------------

/// 低通 FIR 是 63 抽头，边界按零填充，因此首尾各约 31 个原生样本被衰减。48 kHz 下
/// 折算到输出是各约 11 个样本，取 40 留足余量后只比较内部。
const EDGE_SKIP: usize = 40;

#[test]
fn a_48khz_device_is_resampled_to_16khz_against_an_analytic_golden() {
    let native = NativeFormat::new(48_000, 1);
    let hz = 440.0;
    let input: Vec<f32> = (0..48_000)
        .map(|i| sine(hz, native.sample_rate, i))
        .collect();

    let got = to_target(&input, native);

    // 黄金参考是**解析式**合成的 16 kHz 正弦，不是「上一次运行的输出」。这一点是刻意的：
    // 拿自己的输出当黄金只能锁住行为不变，锁不住行为正确——重采样写错时那份 fixture 会
    // 连同错误一起被固化。
    let golden: Vec<f32> = (0..got.len())
        .map(|i| sine(hz, TARGET_SAMPLE_RATE, i))
        .collect();

    assert_eq!(got.len(), 16_000, "一秒 48 kHz 必须重采样成 16000 个样本");
    let mut worst = 0.0f32;
    for (index, (a, b)) in got
        .iter()
        .zip(golden.iter())
        .enumerate()
        .skip(EDGE_SKIP)
        .take(got.len().saturating_sub(2 * EDGE_SKIP))
    {
        let diff = (a - b).abs();
        assert!(
            diff < 0.02,
            "第 {index} 个样本偏离解析黄金 {diff}（got={a}, golden={b}）"
        );
        worst = worst.max(diff);
    }
    eprintln!("48k→16k 重采样与解析黄金的最大偏差：{worst:.6}");
    assert!(worst > 0.0, "偏差恒为零说明比较根本没跑，检查跳边界的算式");
}

#[test]
fn one_second_of_48khz_stereo_yields_16000_mono_samples() {
    let native = NativeFormat::new(48_000, 2);
    let device = FakeInput::tone(native, 1.0, 440.0);
    let pcm = capture_with(
        &granted(Platform::Linux),
        || Ok(device),
        Duration::from_secs(1),
    )
    .expect("假设备采集应成功");

    assert_eq!(pcm.samples.len(), 16_000);
    assert_eq!(pcm.sample_rate, TARGET_SAMPLE_RATE);
    assert_eq!(pcm.channels, TARGET_CHANNELS);
    assert_eq!(pcm.native, native, "原生格式必须原样留着，它是诊断信息");
    assert!(pcm.was_converted(), "48 kHz 双声道必须被标记为经过转换");
    assert!(
        pcm.rms() > 0.1,
        "正弦音的 RMS 应接近 0.707，实测 {}",
        pcm.rms()
    );
    assert_eq!(pcm.duration(), Duration::from_secs(1));
}

#[test]
fn downsampling_low_passes_first_so_a_12khz_tone_cannot_fold_into_the_voice_band() {
    // 12 kHz 在 48 kHz 上合法，但目标 16 kHz 的奈奎斯特频率只有 8 kHz。只做抽取的话
    // 它会折叠到 |12000 - 16000| = 4 kHz，凭空出现在语音频带里。
    let native = NativeFormat::new(48_000, 1);
    let tone: Vec<f32> = (0..48_000)
        .map(|i| sine(12_000.0, native.sample_rate, i))
        .collect();

    let naive = resample(&tone, native.sample_rate, TARGET_SAMPLE_RATE);
    let guarded = to_target(&tone, native);

    let naive_rms = crate::rms(&naive);
    let guarded_rms = crate::rms(&guarded);
    eprintln!("12 kHz 折叠：裸抽取 RMS={naive_rms:.4}，抗混叠 RMS={guarded_rms:.4}");
    assert!(
        naive_rms > 0.3,
        "裸抽取必须留下大量折叠能量，否则这条测试证明不了任何事：{naive_rms}"
    );
    assert!(
        guarded_rms < naive_rms / 10.0,
        "抗混叠后应至少衰减 20 dB，实测 {guarded_rms} vs {naive_rms}"
    );
}

#[test]
fn downmix_averages_channels_instead_of_dropping_one() {
    // 左声道满幅、右声道静音。取平均得到半幅；只取第一路会得到满幅，只取第二路得到零。
    let interleaved = vec![1.0, 0.0, 1.0, 0.0, -1.0, 0.0];
    let mono = downmix(&interleaved, 2);
    assert_eq!(mono, vec![0.5, 0.5, -0.5]);
}

#[test]
fn downmix_drops_a_partial_trailing_frame_rather_than_padding_it() {
    // 末尾多出一个样本是设备回调边界的产物。补零会在信号末尾造成一个可听的咔嗒。
    let mono = downmix(&[1.0, 1.0, 1.0], 2);
    assert_eq!(mono, vec![1.0], "残余的半帧必须丢弃，不能补零凑成一帧");
}

#[test]
fn a_device_already_at_the_target_format_is_passed_through_untouched() {
    let native = NativeFormat::TARGET;
    let input: Vec<f32> = (0..1_000)
        .map(|i| sine(440.0, native.sample_rate, i))
        .collect();
    assert_eq!(
        to_target(&input, native),
        input,
        "已经是 16 kHz 单声道时不该再过一遍滤波器"
    );
}

#[test]
fn target_sample_count_follows_the_requested_duration() {
    assert_eq!(target_samples(Duration::from_secs(1)), 16_000);
    assert_eq!(target_samples(Duration::from_millis(500)), 8_000);
    assert_eq!(target_samples(Duration::ZERO), 0);
}

// ---------------------------------------------------------------------------
// 权限门与开流顺序
// ---------------------------------------------------------------------------

#[test]
fn the_permission_gate_runs_before_the_device_is_ever_opened() {
    // macOS 与 Windows 在进程首次触达输入设备的瞬间弹窗或写入隐私记录。授权还没拿到
    // 就开流，用户会看到一个我们本不该触发的对话框，所以顺序不是风格问题。
    let denied = Preflight::new(MicPermission::new(
        Platform::MacOs,
        PermissionState::Denied,
        "",
    ));
    let mut opened = false;
    let error = capture_with(
        &denied,
        || {
            opened = true;
            Ok(FakeInput::tone(NativeFormat::TARGET, 1.0, 440.0))
        },
        Duration::from_secs(1),
    )
    .expect_err("授权被拒时必须失败");

    assert!(!opened, "权限门必须在打开设备之前拦住，实测设备被打开了");
    assert!(matches!(
        error,
        AudioError::PermissionDenied {
            state: PermissionState::Denied,
            ..
        }
    ));
}

#[test]
fn a_system_below_the_floor_beats_a_denied_permission() {
    // 低于底线的系统上 cpal 的 CoreAudio 后端无条件引用 macOS 14.2 才有的符号且没做
    // 弱链接，进程在触达音频栈时就完了。此时报「授权被拒」会把用户引到一个改了也没用的
    // 系统设置面板。
    let preflight = Preflight {
        permission: MicPermission::new(Platform::MacOs, PermissionState::Denied, ""),
        system: SystemSupport::Below {
            found: "13.6".to_owned(),
        },
    };
    let error = preflight.check().expect_err("低于底线必须失败");
    let AudioError::UnsupportedPlatformVersion {
        required, found, ..
    } = &error
    else {
        panic!("系统版本必须优先于权限被报出，实测 {error:?}");
    };
    assert_eq!(required, "14.2", "底线要引自 platform::FLOORS");
    assert_eq!(found, "13.6", "实测版本要原样进文案");
}

#[test]
fn a_granted_permission_on_a_supported_system_opens_the_gate() {
    assert!(granted(Platform::Linux).check().is_ok());
}

#[test]
fn restricted_and_undetermined_keep_their_own_reasons_through_the_audio_layer() {
    // todo 46 把这三种状态分开是因为引导完全不同。音频层只有一个 PermissionDenied
    // 变体，如果它不带 state，那个区分就在这里丢掉了。
    for (state, expected) in [
        (PermissionState::Denied, DegradeReason::PermissionDenied),
        (
            PermissionState::Restricted,
            DegradeReason::PermissionRestricted,
        ),
        (
            PermissionState::Undetermined,
            DegradeReason::PermissionUndetermined,
        ),
    ] {
        let error = Preflight::new(MicPermission::new(Platform::Android, state, ""))
            .check()
            .expect_err("非 Granted 都该失败");
        assert_eq!(
            error.degrade_reason(),
            expected,
            "{state:?} 的降级原因被压平了"
        );
    }
}

// ---------------------------------------------------------------------------
// 四个变体各由其对应条件产出
// ---------------------------------------------------------------------------

#[test]
fn permission_denied_comes_from_a_denied_report() {
    let error = Preflight::new(MicPermission::new(
        Platform::Windows,
        PermissionState::Denied,
        "",
    ))
    .check()
    .expect_err("被拒必须失败");
    assert_eq!(error.degrade_reason(), DegradeReason::PermissionDenied);
    assert_eq!(error.platform(), Some(Platform::Windows));
}

#[test]
fn no_device_comes_from_an_empty_enumeration() {
    let error = classify_capture_error(
        &crate::VoiceError::NoInputDevice {
            detail: "系统没有报告任何默认输入设备".to_owned(),
        },
        0,
    );
    assert!(matches!(error, AudioError::NoDevice { .. }), "{error:?}");
    assert_eq!(error.degrade_reason(), DegradeReason::NoInputDevice);
}

#[test]
fn an_open_device_that_produces_nothing_is_reported_as_no_device() {
    let error = capture_with(
        &granted(Platform::Linux),
        || Ok(FakeInput::empty(NativeFormat::TARGET)),
        Duration::from_secs(1),
    )
    .expect_err("零样本不该算成功");
    assert!(matches!(error, AudioError::NoDevice { .. }), "{error:?}");
}

#[test]
fn unsupported_platform_version_comes_from_the_shell_reporting_an_old_system() {
    let error = Preflight {
        permission: MicPermission::new(Platform::Android, PermissionState::Granted, ""),
        system: SystemSupport::Below {
            found: "24".to_owned(),
        },
    }
    .check()
    .expect_err("低于 API 26 必须失败");
    assert!(
        matches!(error, AudioError::UnsupportedPlatformVersion { .. }),
        "{error:?}"
    );
    assert_eq!(error.degrade_reason(), DegradeReason::SystemTooOld);
}

#[test]
fn device_busy_is_told_apart_from_no_device_only_by_the_enumeration() {
    // cpal 把 EBUSY 与 ENODEV 压成同一个 BuildStreamError::DeviceNotAvailable
    // （alsa/mod.rs:362），WASAPI 把 AUDCLNT_E_DEVICE_IN_USE 与 _INVALIDATED 压成同一个
    // （wasapi/mod.rs:103）。所以同一句错误文本，枚举结果不同就必须分类成不同的变体。
    let alsa_busy = || crate::VoiceError::AudioDevice {
        detail: "打开输入流失败".to_owned(),
        source: "ALSA function 'snd_pcm_open' failed: Device or resource busy".into(),
    };

    let busy = classify_capture_error(&alsa_busy(), 1);
    assert!(matches!(busy, AudioError::DeviceBusy { .. }), "{busy:?}");
    assert_eq!(busy.degrade_reason(), DegradeReason::DeviceBusy);

    let gone = classify_capture_error(&alsa_busy(), 0);
    assert!(
        matches!(gone, AudioError::NoDevice { .. }),
        "枚举为空时同一句报错必须落成「没有设备」，实测 {gone:?}"
    );
    assert_ne!(
        busy.degrade_reason(),
        gone.degrade_reason(),
        "两者的引导不同：一个是关掉占用程序，一个是插一个麦克风"
    );
}

#[test]
fn a_wasapi_device_in_use_also_classifies_as_busy() {
    let error = classify_capture_error(
        &crate::VoiceError::AudioDevice {
            detail: "打开输入流失败".to_owned(),
            source:
                "The requested device is no longer available. For example, it has been unplugged."
                    .into(),
        },
        2,
    );
    assert!(matches!(error, AudioError::DeviceBusy { .. }), "{error:?}");
}

#[test]
fn an_unrecognized_open_failure_degrades_without_claiming_a_cause() {
    let error = classify_capture_error(
        &crate::VoiceError::AudioDevice {
            detail: "输入设备配置失败".to_owned(),
            source: "some new upstream wording nobody has seen yet".into(),
        },
        3,
    );
    assert!(
        matches!(
            error,
            AudioError::Failed {
                stage: "打开", ..
            }
        ),
        "认不出的报错要落成 Failed 而不是硬猜一个原因：{error:?}"
    );
    assert_eq!(error.degrade_reason(), DegradeReason::CaptureFailed);
}

#[test]
fn a_truncated_capture_is_an_error_not_a_short_success() {
    // 默默返回半段音频是最坏的结果：调用方拿到一段标着「一秒」的半段录音，评分会把它
    // 算成没背完，于是一次设备故障变成一个错误的分数。
    let native = NativeFormat::new(48_000, 1);
    let error = capture_with(
        &granted(Platform::Linux),
        || Ok(FakeInput::tone(native, 0.48, 440.0)),
        Duration::from_secs(1),
    )
    .expect_err("只给到 484 ms 不该算成功");
    let AudioError::Failed { stage, detail } = &error else {
        panic!("截断应落成 Failed：{error:?}");
    };
    assert_eq!(*stage, "采集");
    assert!(detail.contains("480"), "报错要换算成毫秒：{detail}");
    assert_eq!(error.degrade_reason(), DegradeReason::CaptureFailed);
}

#[test]
fn resampler_edge_rounding_is_not_mistaken_for_truncation() {
    // 重采样器在跨度边界少给几十个样本是 4 ms 的取整，不是故障。完成度门用比例而不是
    // 相等，正是为了把它和真截断分开。
    assert!(meets_completion(15_936, 16_000));
    assert!(!meets_completion(7_744, 16_000));
    assert!(meets_completion(15_200, 16_000));
    assert!(!meets_completion(15_199, 16_000));
}

#[test]
fn a_device_reporting_an_invalid_format_fails_instead_of_dividing_by_zero() {
    let error = capture_with(
        &granted(Platform::Linux),
        || Ok(FakeInput::silent(NativeFormat::new(0, 1), 1.0)),
        Duration::from_secs(1),
    )
    .expect_err("0 Hz 不是合法格式");
    assert!(
        matches!(
            error,
            AudioError::Failed {
                stage: "打开", ..
            }
        ),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// 降级：每一种失败都退回打字练习，而不是零分
// ---------------------------------------------------------------------------

#[test]
fn every_audio_error_degrades_to_typed_practice_with_its_own_explanation() {
    let cases = [
        AudioError::PermissionDenied {
            platform: Platform::MacOs,
            state: PermissionState::Denied,
        },
        AudioError::NoDevice {
            detail: "没有输入设备".to_owned(),
        },
        AudioError::UnsupportedPlatformVersion {
            platform: Platform::MacOs,
            required: "14.2".to_owned(),
            found: "13.6".to_owned(),
        },
        AudioError::DeviceBusy {
            detail: "Device or resource busy".to_owned(),
        },
        AudioError::Failed {
            stage: "采集",
            detail: "未知".to_owned(),
        },
    ];

    let mut messages = Vec::new();
    for error in &cases {
        let practice = error.practice();
        assert!(
            practice.is_typed(),
            "{error:?} 必须降级到打字练习，不能崩也不能返回零分"
        );
        assert_eq!(practice.reason(), Some(error.degrade_reason()));
        let crate::permission::Practice::Typed { message, .. } = practice else {
            unreachable!("上一行已断言是 Typed")
        };
        assert!(
            message.contains("打字练习"),
            "{error:?} 的解释要告诉用户还能做什么：{message}"
        );
        messages.push(message);
    }

    let unique: std::collections::BTreeSet<&String> = messages.iter().collect();
    assert_eq!(
        unique.len(),
        messages.len(),
        "五种失败的解释必须各不相同，合并任意两条就是在一半情形下误导用户：{messages:#?}"
    );
}

#[test]
fn a_denied_permission_routes_the_recite_session_to_typing_not_a_zero_score() {
    // 这是失败场景的验收断言：调用方拿到的是 PermissionDenied 这一个确切变体，并且
    // 由它派生出的是「转打字模式」，而不是一个 0 分的语音成绩。
    let error = Preflight::new(MicPermission::new(
        Platform::MacOs,
        PermissionState::Denied,
        "",
    ))
    .check()
    .expect_err("被拒必须失败");

    assert!(
        matches!(
            error,
            AudioError::PermissionDenied {
                platform: Platform::MacOs,
                state: PermissionState::Denied
            }
        ),
        "调用方必须收到这个确切变体：{error:?}"
    );

    let practice = error.practice();
    assert_eq!(practice.reason(), Some(DegradeReason::PermissionDenied));
    let crate::permission::Practice::Typed { message, .. } = practice else {
        panic!("必须转打字模式");
    };
    assert!(
        message.contains("隐私与安全性"),
        "macOS 的引导要指向对的那个面板：{message}"
    );
    assert!(
        !message.contains('0') && !message.contains("分"),
        "降级解释里不能出现任何分数，那会被读成一次真实的失败成绩：{message}"
    );
}

// ---------------------------------------------------------------------------
// Linux 桥接设备优先
// ---------------------------------------------------------------------------

#[test]
fn the_pipewire_bridge_outranks_the_exclusive_alsa_hardware_device() {
    // 裸 hw: 设备是独占的，被浏览器或会议软件拿住时我们只会拿到 EBUSY；PipeWire 与
    // PulseAudio 的桥接设备是混音的，多个进程可以同时读。
    assert!(rank_input("pipewire") < rank_input("pulse"));
    assert!(rank_input("pulse") < rank_input("default"));
    assert!(rank_input("default") < rank_input("hw:CARD=PCH,DEV=0"));
    assert!(rank_input("sysdefault:CARD=PCH") < rank_input("front:CARD=PCH,DEV=0"));
}

#[test]
fn pick_input_prefers_the_bridge_over_whatever_came_first() {
    let names: Vec<String> = ["hw:CARD=PCH,DEV=0", "default", "pipewire", "pulse"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(pick_input(&names).as_deref(), Some("pipewire"));
    assert_eq!(pick_input(&[]), None, "没有设备时不该硬造一个名字");
}

#[test]
fn ties_keep_enumeration_order_so_the_choice_is_reproducible() {
    // 不稳定的设备选择会让「昨天能录今天不能」这种报告无法复现。
    let names: Vec<String> = ["USB Audio", "Webcam Mic"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(rank_input(&names[0]), rank_input(&names[1]));
    assert_eq!(pick_input(&names).as_deref(), Some("USB Audio"));
}

// ---------------------------------------------------------------------------
// 真实硬件。本机无设备，因此这两条断言的是「成功，或者带着准确的类型化原因降级」
// ---------------------------------------------------------------------------

#[cfg(feature = "capture")]
#[test]
fn playing_a_known_wav_either_completes_or_degrades_with_an_accurate_reason() {
    use std::io::BufReader;

    let dir = std::env::temp_dir().join("yunjian-audio-playback-test");
    std::fs::create_dir_all(&dir).expect("临时目录可创建");
    let path = dir.join("tone-440hz-16k-mono.wav");

    let spec = hound::WavSpec {
        channels: TARGET_CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::create(&path, spec).expect("WAV 可写");
        for i in 0..(TARGET_SAMPLE_RATE as usize / 5) {
            let value = sine(440.0, TARGET_SAMPLE_RATE, i);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "value 在 [-1, 1] 内，乘 i16::MAX 后必然落在 i16 范围"
            )]
            let quantized = (value * f32::from(i16::MAX) * 0.5) as i16;
            writer.write_sample(quantized).expect("样本可写");
        }
        writer.finalize().expect("WAV 可收尾");
    }

    // 用 hound 读而不是 rodio::Decoder：工作区关掉了 rodio 的默认特性，本构建里
    // Decoder 解不了任何容器格式。需要播放的本来就是裸 PCM。
    let mut reader = hound::WavReader::new(BufReader::new(
        std::fs::File::open(&path).expect("WAV 可读"),
    ))
    .expect("WAV 头可解析");
    let read_spec = reader.spec();
    assert_eq!(read_spec.sample_rate, TARGET_SAMPLE_RATE);
    assert_eq!(read_spec.channels, TARGET_CHANNELS);
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| f32::from(s.expect("样本可解码")) / f32::from(i16::MAX))
        .collect();
    assert!(!samples.is_empty());
    assert!(crate::rms(&samples) > 0.1, "fixture 本身不能是静音");

    let started = std::time::Instant::now();
    match super::play_samples(&samples, TARGET_SAMPLE_RATE, TARGET_CHANNELS) {
        Ok(()) => {
            let elapsed = started.elapsed();
            eprintln!("播放完成，耗时 {elapsed:?}（音频 200 ms）");
            // 只设上界，**刻意不设下界**。实测 `Player::sleep_until_end` 的墙钟时间与音频
            // 时长没有稳定关系：本机 PulseAudio 桥接设备上 200 ms 的音频 0.13 s 返回
            // （早于播完），500 ms 要 1.80 s，1 s 要 2.16 s。加一条下界断言只会做出一条
            // 依赖宿主机缓冲深度的 flaky 测试。todo 54 的音步时间戳因此必须来自样本数，
            // 不能来自播放墙钟。
            assert!(
                elapsed < Duration::from_secs(30),
                "播放 200 ms 却花了 {elapsed:?}，判定为卡住而不是播完"
            );
        }
        Err(error) => {
            let practice = error.practice();
            eprintln!("本机无可用输出设备，播放降级：{error} → {practice:?}");
            assert!(practice.is_typed(), "播放失败也必须降级而不是崩：{error:?}");
            assert!(
                matches!(
                    error.degrade_reason(),
                    DegradeReason::NoInputDevice
                        | DegradeReason::DeviceBusy
                        | DegradeReason::CaptureFailed
                ),
                "播放失败的原因要落在设备类里，不该是权限或系统版本：{error:?}"
            );
        }
    }

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "capture")]
#[test]
fn opening_the_real_default_input_either_succeeds_or_degrades_with_an_accurate_reason() {
    let preflight = granted(Platform::current().unwrap_or(Platform::Linux));
    match super::open_default_input(&preflight) {
        Ok(device) => {
            let info = device.describe();
            eprintln!(
                "真实输入设备：{} / {} Hz / {} 声道",
                info.name, info.native.sample_rate, info.native.channels
            );
            assert!(info.native.sample_rate > 0);
            assert!(info.native.channels > 0);
        }
        Err(error) => {
            eprintln!("本机无可用输入设备：{error}");
            assert!(error.practice().is_typed(), "{error:?}");
            assert!(
                matches!(
                    error,
                    AudioError::NoDevice { .. }
                        | AudioError::DeviceBusy { .. }
                        | AudioError::Failed { .. }
                ),
                "授权已给且系统达标，失败只能是设备类原因：{error:?}"
            );
        }
    }
}
