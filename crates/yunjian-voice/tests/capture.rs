//! 桌面采集集成测试：真的打开麦克风，录一秒，断言产出识别器要求的格式且**不是静音**。
//!
//! 这条测试**默认执行**，缺设备时硬失败而不是静默跳过。理由是门禁静默蒸发比门禁报错
//! 危险得多：一条 `if no_device { return }` 会让「CI 上从未真的录过音」看起来全绿。
//! 真的没有音频设备的开发机可以设 `YUNJIAN_SKIP_CAPTURE_TEST=1` 明确豁免；CI 永不设它，
//! 并由 `workflow_wires_a_virtual_input_device` 反向钉住这一点。

#![cfg(feature = "capture")]
#![expect(
    clippy::print_stderr,
    reason = "测试要在 CI 日志里留下实际用了哪个设备、RMS 是多少——\
              一条只说 ok 的绿色测试没法证明它真的录到了声音"
)]

use std::time::Duration;

use yunjian_voice::capture::{self, TARGET_CHANNELS, TARGET_SAMPLE_RATE};

const SKIP_VAR: &str = "YUNJIAN_SKIP_CAPTURE_TEST";

/// 静音判定阈值。虚拟输入设备（PulseAudio null sink + module-sine、ALSA snd-aloop）
/// 的 RMS 在 0.3 量级；真实麦克风的环境底噪也远高于此。取 1e-4 是为了只排除
/// 「一个字节都没录到」的情形，而不是给信号强度设门。
const SILENCE_FLOOR: f32 = 1e-4;

#[test]
fn captures_one_second_of_16k_mono_pcm_with_nonzero_rms() {
    if std::env::var_os(SKIP_VAR).is_some() {
        eprintln!("已设 {SKIP_VAR}，跳过真实采集");
        return;
    }

    let inputs = capture::list_inputs().unwrap_or_else(|err| {
        panic!(
            "枚举输入设备失败：{err}\n\
             这台机器确实没有音频设备时请设 {SKIP_VAR}=1 明确豁免；\n\
             CI 上应先接入虚拟输入设备（见 .github/workflows/audio-permissions.yml）"
        )
    });
    assert!(
        !inputs.is_empty(),
        "没有任何输入设备。CI 上需要虚拟输入设备；本机无声卡请设 {SKIP_VAR}=1"
    );

    let mut failures = Vec::new();
    // 先试默认设备，失败再逐个试。理由：CI 上 `default` 到底解析到哪一个取决于
    // 虚拟设备的接入方式（ALSA snd-aloop 会成为 card 0，PulseAudio 走 pcm.pulse），
    // 而这条测试要断言的是「采集通路成立」，不是「default 恰好对」。
    let mut attempts: Vec<Option<String>> = vec![None];
    attempts.extend(inputs.iter().cloned().map(Some));

    for attempt in attempts {
        let result = match attempt.as_deref() {
            None => capture::capture_default(Duration::from_secs(1)),
            Some(name) => capture::capture_from(name, Duration::from_secs(1)),
        };
        let label = attempt.clone().unwrap_or_else(|| "default".to_owned());
        match result {
            Err(err) => failures.push(format!("{label}：{err}")),
            Ok(got) => {
                assert_eq!(
                    got.sample_rate, TARGET_SAMPLE_RATE,
                    "识别器硬要求 16 kHz，设备原生 {} Hz 时必须重采样",
                    got.device_sample_rate
                );
                assert_eq!(
                    got.channels, TARGET_CHANNELS,
                    "识别器硬要求单声道，设备原生 {} 声道时必须降混",
                    got.device_channels
                );
                let expected = TARGET_SAMPLE_RATE as usize;
                assert_eq!(
                    got.samples.len(),
                    expected,
                    "一秒 16 kHz 单声道应为 {expected} 个样本"
                );
                let rms = got.rms();
                if rms <= SILENCE_FLOOR {
                    failures.push(format!("{label}：格式正确但全是静音（RMS={rms:.9}）"));
                    continue;
                }
                eprintln!(
                    "采集成功：{label} / 设备原生 {} Hz {} 声道 / RMS={rms:.6}",
                    got.device_sample_rate, got.device_channels
                );
                return;
            }
        }
    }

    panic!(
        "所有 {} 个候选设备都没能录到非静音的 16 kHz 单声道 PCM：\n  {}\n\
         CI 上请确认虚拟输入设备已接入并且**有信号在喂**——一个只产出零的回环设备\n\
         能通过所有格式断言却证明不了任何事。",
        inputs.len() + 1,
        failures.join("\n  ")
    );
}

#[test]
fn a_stalled_device_is_reported_rather_than_hanging_forever() {
    let start = std::time::Instant::now();
    let outcome = capture::capture_from("绝不存在的设备名-yunjian-test", Duration::from_secs(1));
    let err = outcome.expect_err("不存在的设备名必须报错");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "查不到设备应立即失败而不是等到超时：{:?}",
        start.elapsed()
    );
    assert!(
        matches!(err, yunjian_voice::VoiceError::NoInputDevice { .. }),
        "设备名匹配不到应报 NoInputDevice，实际 {err:?}"
    );
    assert_eq!(
        err.degrade_reason(),
        yunjian_voice::permission::DegradeReason::NoInputDevice
    );
}

#[test]
fn workflow_wires_a_virtual_input_device_and_never_sets_the_skip_var() {
    let workflow = include_str!("../../../.github/workflows/audio-permissions.yml");
    assert!(
        !workflow.contains(SKIP_VAR),
        "CI 一旦设 {SKIP_VAR} 就再也不会真的录音，这条测试也就只是装饰"
    );
    for needle in ["module-sine", "snd-aloop"] {
        assert!(
            workflow.contains(needle),
            "工作流要接入能产出信号的虚拟输入设备，缺 {needle}"
        );
    }
}
