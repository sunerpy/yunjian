//! 麦克风采集，产出识别器要求的 16 kHz 单声道 PCM。
//!
//! 走 `rodio::microphone` 而不是直接用 `cpal`：`rodio 0.22.2` 内部固定 `cpal 0.17.3`，
//! 而工作区曾固定 `cpal 0.18`，两份 `cpal::Device` 是**不同类型**，跨边界传参会得到
//! 只有版本注记区别的 `expected cpal::Device, found cpal::Device`。本模块因此不在
//! 公开签名里出现任何 `cpal` 类型，`cpal` 也不是本项目的直接依赖。
//!
//! 采集在**独立线程**里完成并经 channel 回传，原因是 [`rodio::microphone::Microphone`]
//! 的 `Iterator::next` 在缓冲区为空时自旋休眠、只在流报错时返回 `None`：一个既不产出
//! 数据也不报错的设备会让它永久阻塞，而阻塞在迭代器内部时任何超时检查都跑不到。
//! 把它关进线程后调用方能用 `recv_timeout` 兜住这种停摆。

use std::num::NonZero;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rodio::microphone::{MicrophoneBuilder, available_inputs};
use rodio::source::UniformSourceIterator;

use crate::error::VoiceError;

/// 识别器要求的采样率。`sherpa_rs::read_audio_file` 对此硬断言，因此这不是偏好而是约束。
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 识别器要求的声道数。
pub const TARGET_CHANNELS: u16 = 1;

/// 一次采集的结果。
#[derive(Debug, Clone)]
pub struct Capture {
    /// 归一到 `[-1.0, 1.0]` 的单声道样本。
    pub samples: Vec<f32>,
    /// 采样率，恒为 [`TARGET_SAMPLE_RATE`]。
    pub sample_rate: u32,
    /// 声道数，恒为 [`TARGET_CHANNELS`]。
    pub channels: u16,
    /// 设备实际给出的采样率。与目标不同即说明重采样生效了，是诊断信息。
    pub device_sample_rate: u32,
    /// 设备实际给出的声道数。
    pub device_channels: u16,
    /// 设备名，用于诊断「录到的是哪一个」。
    pub device: String,
}

impl Capture {
    /// 均方根电平。全零缓冲区在长度断言下也能过，因此这是「真的录到声音」的判据。
    #[must_use]
    pub fn rms(&self) -> f32 {
        crate::rms(&self.samples)
    }

    /// 样本数对应的时长。
    #[must_use]
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(
            f64::from(u32::try_from(self.samples.len()).unwrap_or(u32::MAX))
                / f64::from(self.sample_rate),
        )
    }
}

/// 列出可用输入设备名。
///
/// `rodio` 会滤掉 driver 为 `null` 的设备（那种设备只产出零），因此这里返回的都是
/// 理论上能录到东西的设备。
pub fn list_inputs() -> Result<Vec<String>, VoiceError> {
    let inputs = available_inputs().map_err(|source| VoiceError::AudioDevice {
        detail: "枚举输入设备失败".to_owned(),
        source: Box::new(source),
    })?;
    Ok(inputs.iter().map(ToString::to_string).collect())
}

/// 用系统默认输入设备采集 `duration` 时长。
pub fn capture_default(duration: Duration) -> Result<Capture, VoiceError> {
    capture_inner(None, duration)
}

/// 用名字**包含** `name` 的第一个输入设备采集。名字来自 [`list_inputs`]。
pub fn capture_from(name: &str, duration: Duration) -> Result<Capture, VoiceError> {
    capture_inner(Some(name.to_owned()), duration)
}

/// 采集线程停摆的判定余量：目标时长的三倍再加两秒。
///
/// 三倍是因为重采样与环形缓冲都会引入落后；固定加两秒覆盖设备打开本身的耗时。
fn stall_deadline(duration: Duration) -> Duration {
    duration.saturating_mul(3) + Duration::from_secs(2)
}

fn capture_inner(name: Option<String>, duration: Duration) -> Result<Capture, VoiceError> {
    let (tx, rx) = mpsc::channel();
    let wanted = duration;
    // 设备句柄在多数平台上不是 `Send`，所以它必须在工作线程内部创建，而不是传进去。
    thread::Builder::new()
        .name("yunjian-capture".to_owned())
        .spawn(move || {
            let _ = tx.send(run_capture(name.as_deref(), wanted));
        })
        .map_err(|source| VoiceError::AudioDevice {
            detail: "采集线程创建失败".to_owned(),
            source: Box::new(source),
        })?;

    match rx.recv_timeout(stall_deadline(duration)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(VoiceError::CaptureStalled {
            waited: stall_deadline(duration),
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(VoiceError::AudioDevice {
            detail: "采集线程异常退出".to_owned(),
            source: "channel disconnected".into(),
        }),
    }
}

fn run_capture(name: Option<&str>, duration: Duration) -> Result<Capture, VoiceError> {
    let base = MicrophoneBuilder::new();
    let with_device = match name {
        None => base.default_device().map_err(device_error)?,
        Some(needle) => {
            let inputs = available_inputs().map_err(|source| VoiceError::AudioDevice {
                detail: "枚举输入设备失败".to_owned(),
                source: Box::new(source),
            })?;
            let chosen = inputs
                .into_iter()
                .find(|input| input.to_string().contains(needle))
                .ok_or_else(|| VoiceError::NoInputDevice {
                    detail: format!("没有名字包含 `{needle}` 的输入设备"),
                })?;
            base.device(chosen).map_err(device_error)?
        }
    };

    // 优先直接向设备要 16 kHz 或它的整数倍：能省掉一次重采样，且 16k 的倍数重采样
    // 误差最小。要不到就退回设备默认配置，由 `UniformSourceIterator` 兜。
    let configured = with_device
        .default_config()
        .map_err(device_error)?
        .prefer_sample_rates([
            NonZero::new(TARGET_SAMPLE_RATE).expect("16000 非零"),
            NonZero::new(TARGET_SAMPLE_RATE * 2).expect("32000 非零"),
            NonZero::new(TARGET_SAMPLE_RATE * 3).expect("48000 非零"),
        ])
        .prefer_channel_counts([NonZero::new(TARGET_CHANNELS).expect("1 非零")]);

    let config = configured.get_config();
    let device_sample_rate = config.sample_rate.get();
    let device_channels = config.channel_count.get();

    let mic = configured
        .open_stream()
        .map_err(|source| VoiceError::AudioDevice {
            detail: "打开输入流失败".to_owned(),
            source: Box::new(source),
        })?;
    let device = name.unwrap_or("default").to_owned();

    let wanted_samples = usize::try_from(
        (u64::from(TARGET_SAMPLE_RATE)
            * u64::from(u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)))
            / 1_000,
    )
    .unwrap_or(usize::MAX);

    let uniform = UniformSourceIterator::new(
        mic,
        NonZero::new(TARGET_CHANNELS).expect("1 非零"),
        NonZero::new(TARGET_SAMPLE_RATE).expect("16000 非零"),
    );
    let samples: Vec<f32> = uniform.take(wanted_samples).collect();

    if samples.is_empty() {
        return Err(VoiceError::NoInputDevice {
            detail: format!("设备 `{device}` 打开成功但没有产出任何样本"),
        });
    }

    // 短于请求时长即报错，不当成功返回。`Microphone` 的迭代器只在流报错时返回 `None`，
    // 所以任何缺口都意味着采集中途断了——实测过一次 `alsa::poll() spuriously returned`
    // 让一秒的请求只拿到 484 ms。**默默接受截断是最坏的结果**：调用方拿到的是一段
    // 标着「一秒」的半段音频，背诵评分会把它算成没背完，于是一次设备故障变成一个
    // 错误的分数，而不是一条可见的失败。
    if samples.len() < wanted_samples {
        return Err(VoiceError::CaptureTruncated {
            got: samples.len(),
            wanted: wanted_samples,
            sample_rate: TARGET_SAMPLE_RATE,
        });
    }

    Ok(Capture {
        samples,
        sample_rate: TARGET_SAMPLE_RATE,
        channels: TARGET_CHANNELS,
        device_sample_rate,
        device_channels,
        device,
    })
}

/// `rodio` 的 `microphone::builder::Error` 定义在私有模块里，只能通过泛型接住而
/// 无法在签名里命名，因此「没有设备」这一支只能按 Display 文本识别。上游若改这句
/// 文案，退化结果是 [`VoiceError::AudioDevice`] 而不是 [`VoiceError::NoInputDevice`]，
/// 两者的降级原因不同，所以有测试钉住这条判别。
fn device_error<E>(source: E) -> VoiceError
where
    E: std::error::Error + Send + Sync + 'static,
{
    if is_no_device(&source.to_string()) {
        return VoiceError::NoInputDevice {
            detail: "系统没有报告任何默认输入设备".to_owned(),
        };
    }
    VoiceError::AudioDevice {
        detail: "输入设备配置失败".to_owned(),
        source: Box::new(source),
    }
}

fn is_no_device(message: &str) -> bool {
    message.contains("no input device")
}

#[cfg(test)]
mod tests {
    use super::{Capture, TARGET_CHANNELS, TARGET_SAMPLE_RATE, stall_deadline};
    use std::time::Duration;

    #[test]
    fn stall_deadline_leaves_room_for_a_slow_device() {
        let deadline = stall_deadline(Duration::from_secs(1));
        assert!(deadline >= Duration::from_secs(3), "{deadline:?}");
        assert!(
            deadline <= Duration::from_secs(10),
            "不要等到测试超时：{deadline:?}"
        );
    }

    #[test]
    fn a_truncated_capture_is_an_error_not_a_short_success() {
        let err = super::VoiceError::CaptureTruncated {
            got: 7_744,
            wanted: 16_000,
            sample_rate: TARGET_SAMPLE_RATE,
        };
        let text = err.to_string();
        assert!(text.contains("7744"), "报错要给出实际样本数：{text}");
        assert!(
            text.contains("484"),
            "报错要换算成毫秒，纯样本数看不出录短了多少：{text}"
        );
        assert_eq!(
            err.degrade_reason(),
            crate::permission::DegradeReason::CaptureFailed,
            "截断必须降级；默默返回半段音频会让背诵评分算成没背完"
        );
    }

    #[test]
    fn duration_derives_from_the_sample_count() {
        let capture = Capture {
            samples: vec![0.0; TARGET_SAMPLE_RATE as usize],
            sample_rate: TARGET_SAMPLE_RATE,
            channels: TARGET_CHANNELS,
            device_sample_rate: 48_000,
            device_channels: 2,
            device: "test".to_owned(),
        };
        assert_eq!(capture.duration(), Duration::from_secs(1));
        assert!(capture.rms() < f32::EPSILON, "全零缓冲区的 RMS 必须是零");
    }
}
