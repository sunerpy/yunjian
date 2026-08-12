//! 真实音频设备。本文件是整个 crate 里唯一直接触碰 `rodio` 设备句柄的地方。
//!
//! 上层 [`crate::audio`] 的判定与转换逻辑一行都不在这里，因此在没有音频硬件的机器上
//! 那些逻辑仍然完整可测；本文件的内容只能在有设备的机器上验证。

use std::num::NonZero;
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::microphone::{MicrophoneBuilder, available_inputs};
use rodio::stream::DeviceSinkBuilder;

use super::{
    AudioError, DeviceInfo, InputDevice, NativeFormat, Pcm, Preflight, TARGET_CHANNELS,
    TARGET_SAMPLE_RATE, classify_capture_error, pick_input,
};

/// 打开一个输入流。
///
/// **权限门在开流之前**，因为 macOS 与 Windows 都在进程首次触达输入设备的瞬间弹窗或写入
/// 隐私记录：授权还没拿到就开流，用户会看到一个我们本不该触发的对话框。
pub fn open_default_input(preflight: &Preflight) -> Result<RodioInput, AudioError> {
    preflight.check()?;

    let names = list_input_names()?;
    let chosen = pick_input(&names);
    let base = MicrophoneBuilder::new();

    let with_device = match &chosen {
        None => base.default_device(),
        Some(name) => {
            let inputs = available_inputs().map_err(|source| AudioError::Failed {
                stage: "枚举",
                detail: source.to_string(),
            })?;
            let input = inputs
                .into_iter()
                .find(|input| &input.to_string() == name)
                .ok_or_else(|| AudioError::NoDevice {
                    detail: format!("枚举到 `{name}` 但再次查找时它消失了"),
                })?;
            base.device(input)
        }
    }
    .map_err(|source| classify_builder_error(&source.to_string(), names.len()))?;

    // 优先直接向设备要 16 kHz 或它的整数倍：省掉一次重采样，且整数倍重采样误差最小。
    // 要不到就退回设备默认配置，由 `to_target` 兜——这就是「不假定设备是 16 kHz」的落点。
    let configured = with_device
        .default_config()
        .map_err(|source| classify_builder_error(&source.to_string(), names.len()))?
        .prefer_sample_rates([
            nonzero_rate(TARGET_SAMPLE_RATE),
            nonzero_rate(TARGET_SAMPLE_RATE * 2),
            nonzero_rate(TARGET_SAMPLE_RATE * 3),
        ])
        .prefer_channel_counts([nonzero_channels(TARGET_CHANNELS)]);

    let config = configured.get_config();
    let native = NativeFormat::new(config.sample_rate.get(), config.channel_count.get());

    let microphone = configured
        .open_stream()
        .map_err(|source| classify_builder_error(&source.to_string(), names.len()))?;

    Ok(RodioInput {
        microphone,
        info: DeviceInfo {
            name: chosen.unwrap_or_else(|| "default".to_owned()),
            native,
        },
    })
}

/// 可用输入设备名。`rodio` 已滤掉 driver 为 `null` 的设备（那种只产出零）。
pub fn list_input_names() -> Result<Vec<String>, AudioError> {
    let inputs = available_inputs().map_err(|source| AudioError::Failed {
        stage: "枚举",
        detail: source.to_string(),
    })?;
    Ok(inputs.iter().map(ToString::to_string).collect())
}

/// [`InputDevice`] 的真实实现。
///
/// # 阻塞语义
///
/// `rodio` 的 `Microphone` 实现 `Iterator`，而它的 `next` 在缓冲区为空时自旋休眠、
/// **只在流报错时**返回 `None`：一个既不产出数据也不报错的设备会让它永久阻塞。因此
/// [`InputDevice::pull`] 没有超时，超时判定属于调用方。[`capture_default`] 走
/// [`crate::capture`]，那条路径已经把设备关进独立线程并用 `recv_timeout` 兜住停摆。
pub struct RodioInput {
    microphone: rodio::microphone::Microphone,
    info: DeviceInfo,
}

impl InputDevice for RodioInput {
    fn describe(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn pull(&mut self, max_samples: usize) -> Result<Vec<f32>, AudioError> {
        Ok((&mut self.microphone).take(max_samples).collect())
    }
}

/// 采集一段并转成 16 kHz 单声道。
///
/// 走 [`crate::capture`] 而不是 [`super::capture_with`] + [`RodioInput`]：那条路径带
/// todo 46 已实测过的停摆超时与截断判定，而 `RodioInput::pull` 本身没有超时。本函数的
/// 增量是这个 todo 的三件事——开流前过权限门、按 [`pick_input`] 优先桥接设备、把
/// [`crate::VoiceError`] 分类成类型化的 [`AudioError`]。
pub fn capture_default(preflight: &Preflight, duration: Duration) -> Result<Pcm, AudioError> {
    preflight.check()?;

    let names = list_input_names()?;
    if names.is_empty() {
        return Err(AudioError::NoDevice {
            detail: "系统没有报告任何输入设备".to_owned(),
        });
    }
    let chosen = pick_input(&names);

    let captured = match &chosen {
        Some(name) => crate::capture::capture_from(name, duration),
        None => crate::capture::capture_default(duration),
    }
    .map_err(|error| classify_capture_error(&error, names.len()))?;

    Ok(Pcm {
        samples: captured.samples,
        sample_rate: captured.sample_rate,
        channels: captured.channels,
        native: NativeFormat::new(captured.device_sample_rate, captured.device_channels),
        device: captured.device,
    })
}

/// 播放一段裸 PCM，返回时播放已结束。
///
/// 不经解码器：工作区把 `rodio` 的默认特性关了，只留 `recording` 与 `playback`，
/// `rodio::Decoder` 在本构建里解不了任何容器格式。这不是缺口——需要播放的是 TTS 合成出来
/// 的 `Vec<f32>`，本来就是裸 PCM。
pub fn play_samples(samples: &[f32], sample_rate: u32, channels: u16) -> Result<(), AudioError> {
    if samples.is_empty() {
        return Err(AudioError::Failed {
            stage: "播放",
            detail: "没有可播放的样本".to_owned(),
        });
    }
    let sink = DeviceSinkBuilder::open_default_sink().map_err(classify_sink_error)?;
    let source = SamplesBuffer::new(
        nonzero_channels(channels),
        nonzero_rate(sample_rate),
        samples.to_vec(),
    );
    // 走 `Player::connect_new` + `append`，**不是** `rodio::stream::play`：后者的约束是
    // `R: Read + Seek`，它是「把一个文件流交给解码器」的便捷入口，喂不进已经在内存里的
    // `Source`。裸 PCM 只能走这条路。
    let player = rodio::Player::connect_new(sink.mixer());
    player.append(source);
    player.sleep_until_end();
    Ok(())
}

/// 播放一段采集或合成得到的 PCM。
pub fn play_pcm(pcm: &Pcm) -> Result<(), AudioError> {
    play_samples(&pcm.samples, pcm.sample_rate, pcm.channels)
}

/// `rodio` 的 `microphone::builder::Error` 与 `OpenError` 都定义在**私有模块**里
/// （`rodio-0.22.2/src/microphone.rs:110` 是 `mod builder;`，不是 `pub mod`），无法在
/// 下游签名里命名，因此只能按 `Display` 文本判别。上游改文案时的退化结果是
/// [`AudioError::Failed`] 而不是更具体的变体，降级仍然发生，只是解释变粗——
/// 这条判别有测试钉着。
fn classify_builder_error(message: &str, listed_devices: usize) -> AudioError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("no input device") {
        return AudioError::NoDevice {
            detail: message.to_owned(),
        };
    }
    if listed_devices > 0 && super::looks_unavailable(message) {
        return AudioError::DeviceBusy {
            detail: message.to_owned(),
        };
    }
    AudioError::Failed {
        stage: "打开",
        detail: message.to_owned(),
    }
}

fn classify_sink_error(error: rodio::stream::DeviceSinkError) -> AudioError {
    match error {
        rodio::stream::DeviceSinkError::NoDevice => AudioError::NoDevice {
            detail: "系统没有报告任何输出设备".to_owned(),
        },
        other => {
            let text = other.to_string();
            if super::looks_unavailable(&text) {
                AudioError::DeviceBusy { detail: text }
            } else {
                AudioError::Failed {
                    stage: "播放",
                    detail: text,
                }
            }
        }
    }
}

fn nonzero_rate(rate: u32) -> NonZero<u32> {
    NonZero::new(rate).unwrap_or(NonZero::<u32>::MIN)
}

fn nonzero_channels(channels: u16) -> NonZero<u16> {
    NonZero::new(channels).unwrap_or(NonZero::<u16>::MIN)
}
