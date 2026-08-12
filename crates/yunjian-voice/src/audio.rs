//! 音频采集与播放，以及每一种失败如何降级到打字练习。
//!
//! # 为什么与 [`crate::capture`] 分成两个模块
//!
//! [`crate::capture`] 是「拿 `rodio` 直接录一段」的最短路径：整段逻辑都在设备句柄后面，
//! 于是**在没有音频设备的机器上一行都验不到**——CI 与开发机都是这种机器。本模块把同一件
//! 事重新分层，把不需要硬件的部分全部提到设备之前：
//!
//! - **判定层，不带任何特性开关**：[`AudioError`]、[`Preflight`]、[`to_target`]、
//!   [`pick_input`]、[`classify_capture_error`]、[`meets_completion`]。全是纯函数与纯
//!   数据，`make ci` 跑的默认构建就会编译它们并跑测试。
//! - **设备层，`capture` 特性**：[`RodioInput`]、[`capture_default`]、[`play_pcm`]。
//!   只有这一层需要真实硬件。
//!
//! 这条缝是刻意的，不是整洁：**重采样是否正确**与**四种失败各自降级成什么**都属于判定层，
//! 于是一台没有麦克风的机器仍然能证明它们对。
//!
//! # 重采样不是可选项
//!
//! 识别器硬要求 16 kHz 单声道，而设备给什么完全由驱动决定（本机 ALSA 默认 48 kHz 双声道，
//! macOS 常见 44.1 kHz）。因此 [`to_target`] 总是先降混再重采样，**不假定设备就是
//! 16 kHz**；降采样走 [`crate::augment::resample_antialiased`]（先低通再抽取），因为直接
//! 抽取会把 8 kHz 以上的能量折叠回语音频带，那是凭空加进去的假信号。
//!
//! # 播放为什么不解码
//!
//! 工作区把 `rodio` 的默认特性关掉了，只留 `recording` 与 `playback`，所以 `rodio::Decoder`
//! 在本构建里**不能解 WAV**（那要 `wav` 特性，进而拉入 symphonia）。这不是缺口：需要播放的
//! 是 TTS 合成出来的 `Vec<f32>`，本来就是裸 PCM，[`play_pcm`] 直接喂
//! `rodio::buffer::SamplesBuffer` 即可，不需要任何解码器。

use std::time::Duration;

use crate::augment::resample_antialiased;
use crate::permission::{DegradeReason, MicPermission, PermissionState, Practice, degrade};
use crate::platform::{Platform, floor_of};

/// 识别器要求的采样率。`sherpa_rs::read_audio_file` 对此硬断言，因此这不是偏好而是约束。
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 识别器要求的声道数。
pub const TARGET_CHANNELS: u16 = 1;

/// 可接受的最低完成度。低于此比例即判定采集被截断。
///
/// 0.95 的依据是两个实测数字之间有一个数量级的空隙：重采样器边界少给 64 个样本
/// （15936/16000 = 0.996），而真实截断只拿到 7744/16000 = 0.484。任何落在 0.95 以下的
/// 缺口都不可能是取整造成的。
pub const MIN_COMPLETION: f64 = 0.95;

/// 样本数是否达到可接受的完成度。
#[must_use]
pub fn meets_completion(got: usize, wanted: usize) -> bool {
    if wanted == 0 {
        return true;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "样本数远小于 f64 的精确整数范围，比例判定不受影响"
    )]
    let ratio = got as f64 / wanted as f64;
    ratio >= MIN_COMPLETION
}

/// 音频路径的失败面。
///
/// **前四个变体是产品要求**：界面要把每一种都路由到打字练习，并给出**不同**的解释。
/// 「麦克风不可用」这一句话在四种情形下的下一步动作完全不同——去系统设置放开授权、
/// 插一个麦克风、关掉占用它的程序、升级系统。合并任意两个就等于在一半情形下误导用户。
#[derive(Debug, Clone, thiserror::Error)]
pub enum AudioError {
    /// 麦克风授权拿不到。`state` 保留 [`PermissionState`] 的粒度，因为「被拒绝」、
    /// 「被管理策略禁用」与「还没问过」的引导完全不同。
    #[error("麦克风授权不可用（{}：{}）", platform.label(), state_label(*state))]
    PermissionDenied {
        /// 平台。
        platform: Platform,
        /// 具体的授权状态。
        state: PermissionState,
    },

    /// 系统没有报告任何输入设备，或指定的设备不存在。
    #[error("没有可用的输入设备：{detail}")]
    NoDevice {
        /// 是「一个都没有」还是「指定的那个匹配不到」。
        detail: String,
    },

    /// 系统版本低于该平台的语音底线。
    #[error("{} 版本低于语音功能所需的 {required}（实测 {found}）", platform.label())]
    UnsupportedPlatformVersion {
        /// 平台。
        platform: Platform,
        /// 该平台的底线，取自 [`crate::platform::FLOORS`]。
        required: String,
        /// 外壳实测到的版本。
        found: String,
    },

    /// 设备存在但打不开，判定为被其他程序独占。判别依据见 [`classify_capture_error`]。
    #[error("输入设备被占用：{detail}")]
    DeviceBusy {
        /// 底层报错原文，用于诊断。
        detail: String,
    },

    /// 上面四条都不是。`stage` 说明失败发生在哪一步。
    #[error("音频{stage}失败：{detail}")]
    Failed {
        /// 枚举、打开、采集还是播放。
        stage: &'static str,
        /// 底层报错原文。
        detail: String,
    },
}

/// [`PermissionState`] 的中文标签。裸 `Debug` 出现在给用户看的链路上不合适。
fn state_label(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "已授权",
        PermissionState::Denied => "被拒绝",
        PermissionState::Undetermined => "尚未询问",
        PermissionState::Restricted => "被管理策略禁用",
    }
}

impl AudioError {
    /// 该错误对应的降级原因。
    ///
    /// **这个函数必须是全函数**：它是「任何音频失败都退回打字练习」由类型系统保证而非靠
    /// 约定的地方。新增变体时编译器会在这里报缺分支。
    #[must_use]
    pub const fn degrade_reason(&self) -> DegradeReason {
        match self {
            Self::PermissionDenied { state, .. } => match state {
                PermissionState::Denied => DegradeReason::PermissionDenied,
                PermissionState::Restricted => DegradeReason::PermissionRestricted,
                PermissionState::Undetermined => DegradeReason::PermissionUndetermined,
                // 「授权已通过却报授权错误」是我们自己的 bug。把它落在采集失败上是唯一
                // 安全的选择：既不崩，也不对用户宣称一个不存在的权限问题。
                PermissionState::Granted => DegradeReason::CaptureFailed,
            },
            Self::NoDevice { .. } => DegradeReason::NoInputDevice,
            Self::UnsupportedPlatformVersion { .. } => DegradeReason::SystemTooOld,
            Self::DeviceBusy { .. } => DegradeReason::DeviceBusy,
            Self::Failed { .. } => DegradeReason::CaptureFailed,
        }
    }

    /// 出错的平台；与平台无关的失败为 `None`。
    #[must_use]
    pub const fn platform(&self) -> Option<Platform> {
        match self {
            Self::PermissionDenied { platform, .. }
            | Self::UnsupportedPlatformVersion { platform, .. } => Some(*platform),
            Self::NoDevice { .. } | Self::DeviceBusy { .. } | Self::Failed { .. } => None,
        }
    }

    /// 这次失败之后该走哪条练习路径。**永远是打字练习，不是零分。**
    ///
    /// 调用方拿到 `Err` 时唯一正确的动作是问它这个问题，而不是把空音频送进评分——
    /// 一段没录到的音频在评分里等价于「一个字都没背出来」，那会把一次设备故障变成一个
    /// 错误的分数。
    #[must_use]
    pub fn practice(&self) -> Practice {
        degrade(self.degrade_reason(), self.platform())
    }
}

/// 外壳实测到的系统版本是否达到本平台的语音底线。
///
/// **为什么是入参而不是本模块自己探测**：版本探测在各平台落在完全不同的层
/// （macOS 的 `NSProcessInfo`、Android 的 `Build.VERSION.SDK_INT`），todo 46 已把它划归
/// 原生外壳。本模块只做判定，于是判定能在没有设备也没有外壳的机器上被测到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemSupport {
    /// 达到或高于底线。
    Meets,
    /// 低于底线，附实测到的版本字符串。
    Below {
        /// 实测版本，原样进报错文案。
        found: String,
    },
}

/// 开流之前必须过的两道门。
#[derive(Debug, Clone)]
pub struct Preflight {
    /// 麦克风授权状态，由外壳填。
    pub permission: MicPermission,
    /// 系统版本判定，由外壳填。
    pub system: SystemSupport,
}

impl Preflight {
    /// 假定系统版本达标的构造器，供桌面端与测试使用。
    #[must_use]
    pub const fn new(permission: MicPermission) -> Self {
        Self {
            permission,
            system: SystemSupport::Meets,
        }
    }

    /// 两道门都过才返回 `Ok`。
    ///
    /// **顺序是先系统版本再权限**，不能反：低于底线的系统上，`cpal` 的 CoreAudio 后端
    /// 无条件引用 macOS 14.2 才有的符号且没做弱链接（见 [`crate::platform::FLOORS`]），
    /// 进程在触达音频栈时就已经完了，此时再谈「授权被拒」是一条误导用户去改系统设置的解释。
    pub fn check(&self) -> Result<(), AudioError> {
        if let SystemSupport::Below { found } = &self.system {
            let platform = self.permission.platform;
            return Err(AudioError::UnsupportedPlatformVersion {
                platform,
                required: floor_of(platform).minimum.to_owned(),
                found: found.clone(),
            });
        }
        if self.permission.state == PermissionState::Granted {
            return Ok(());
        }
        Err(AudioError::PermissionDenied {
            platform: self.permission.platform,
            state: self.permission.state,
        })
    }
}

/// 设备实际给出的格式。**不假定它是目标格式**，这正是重采样存在的理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeFormat {
    /// 设备原生采样率。
    pub sample_rate: u32,
    /// 设备原生声道数。
    pub channels: u16,
}

impl NativeFormat {
    /// 构造。
    #[must_use]
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    /// 目标格式，即无需任何转换的那一种。
    pub const TARGET: Self = Self::new(TARGET_SAMPLE_RATE, TARGET_CHANNELS);
}

/// 一个输入设备的自述。
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// 设备名，用于诊断「录到的是哪一个」。
    pub name: String,
    /// 设备原生格式。
    pub native: NativeFormat,
}

/// 输入设备的抽象。
///
/// **存在的理由是可测性**：重采样与完成度判定必须能在没有音频设备的机器上被验证，
/// 因此真实设备（[`RodioInput`]）与假设备（测试里的 `FakeInput`）都实现这一个接口，
/// 而 [`capture_with`] 只认接口。
pub trait InputDevice {
    /// 设备名与原生格式。
    fn describe(&self) -> DeviceInfo;

    /// 取走下一批**交错**样本，最多 `max_samples` 个。返回空切片表示流已结束。
    fn pull(&mut self, max_samples: usize) -> Result<Vec<f32>, AudioError>;
}

/// 一段可以直接送进识别器的 PCM。
#[derive(Debug, Clone)]
pub struct Pcm {
    /// 归一到 `[-1.0, 1.0]` 的单声道样本。
    pub samples: Vec<f32>,
    /// 采样率，恒为 [`TARGET_SAMPLE_RATE`]。
    pub sample_rate: u32,
    /// 声道数，恒为 [`TARGET_CHANNELS`]。
    pub channels: u16,
    /// 设备原生格式。与 [`NativeFormat::TARGET`] 不同即说明重采样生效了。
    pub native: NativeFormat,
    /// 设备名。
    pub device: String,
}

impl Pcm {
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

    /// 是否发生过重采样或降混。
    #[must_use]
    pub fn was_converted(&self) -> bool {
        self.native != NativeFormat::TARGET
    }
}

/// 目标格式下 `duration` 对应的样本数。
#[must_use]
pub fn target_samples(duration: Duration) -> usize {
    let millis = u64::from(u32::try_from(duration.as_millis()).unwrap_or(u32::MAX));
    usize::try_from(u64::from(TARGET_SAMPLE_RATE) * millis / 1_000).unwrap_or(usize::MAX)
}

/// 多声道交错样本按帧取平均，降成单声道。
///
/// 取平均而不是只取第一声道：立体声麦克风的两个通道往往一强一弱（一个朝向说话人），
/// 只取一路会在半数设备上丢掉大部分能量。末尾不足一帧的残余样本丢弃——那是设备回调
/// 边界的产物，补零会在信号末尾造成一个可听的咔嗒。
#[must_use]
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let width = usize::from(channels);
    #[expect(clippy::cast_precision_loss, reason = "声道数是个位数小整数")]
    let scale = 1.0 / width as f32;
    interleaved
        .chunks_exact(width)
        .map(|frame| frame.iter().sum::<f32>() * scale)
        .collect()
}

/// 交错的原生格式样本 → 16 kHz 单声道。
///
/// 先降混再重采样，不能反：先重采样会在交错序列上插值，把相邻两个**不同声道**的样本
/// 混在一起，结果是一段听起来像被梳状滤波过的音频。
#[must_use]
pub fn to_target(interleaved: &[f32], native: NativeFormat) -> Vec<f32> {
    let mono = downmix(interleaved, native.channels);
    if native.sample_rate == TARGET_SAMPLE_RATE {
        return mono;
    }
    resample_antialiased(&mono, native.sample_rate, TARGET_SAMPLE_RATE)
}

/// 采集编排：**先过门，再开设备**，然后转成目标格式并判完成度。
///
/// `open` 是 `FnOnce` 而不是一个已经打开的设备，这一点是刻意的：它让「权限门在开流之前」
/// 由类型而非注释保证——闭包只可能在 `check()` 返回 `Ok` 之后被调用，测试可以拿一个
/// 会置标志位的闭包把这条钉死。macOS 与 Windows 上首次触达输入设备就会弹窗或写入隐私
/// 记录，所以顺序颠倒不是风格问题，是会在用户面前弹出一个本不该弹的框。
pub fn capture_with<D, F>(
    preflight: &Preflight,
    open: F,
    wanted: Duration,
) -> Result<Pcm, AudioError>
where
    D: InputDevice,
    F: FnOnce() -> Result<D, AudioError>,
{
    preflight.check()?;

    let mut device = open()?;
    let info = device.describe();
    if info.native.sample_rate == 0 || info.native.channels == 0 {
        return Err(AudioError::Failed {
            stage: "打开",
            detail: format!(
                "设备 `{}` 报告了无效格式：{} Hz / {} 声道",
                info.name, info.native.sample_rate, info.native.channels
            ),
        });
    }

    let wanted_target = target_samples(wanted);
    let wanted_native = native_samples_for(wanted_target, info.native);

    let mut interleaved: Vec<f32> = Vec::with_capacity(wanted_native);
    while interleaved.len() < wanted_native {
        let chunk = device.pull(wanted_native - interleaved.len())?;
        if chunk.is_empty() {
            break;
        }
        interleaved.extend_from_slice(&chunk);
    }

    let samples = to_target(&interleaved, info.native);
    if samples.is_empty() {
        return Err(AudioError::NoDevice {
            detail: format!("设备 `{}` 打开成功但没有产出任何样本", info.name),
        });
    }
    // 默默返回半段音频是最坏的结果：调用方拿到的是一段标着「一秒」的半段录音，
    // 评分会把它算成没背完，于是一次设备故障变成一个错误的分数而不是一条可见的失败。
    if !meets_completion(samples.len(), wanted_target) {
        return Err(AudioError::Failed {
            stage: "采集",
            detail: format!(
                "只拿到 {} 个样本（约 {:.0} ms），请求 {wanted_target} 个（约 {:.0} ms）",
                samples.len(),
                millis_of(samples.len()),
                millis_of(wanted_target),
            ),
        });
    }

    Ok(Pcm {
        samples,
        sample_rate: TARGET_SAMPLE_RATE,
        channels: TARGET_CHANNELS,
        native: info.native,
        device: info.name,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "样本数远小于 f64 的精确整数范围，只用于报错文案"
)]
fn millis_of(samples: usize) -> f64 {
    samples as f64 * 1000.0 / f64::from(TARGET_SAMPLE_RATE)
}

/// 要产出 `target` 个目标样本，需要从设备取多少个交错样本。
///
/// 向上多要一帧：`resample_antialiased` 的输出长度是向下取整的，按精确比例要会稳定
/// 少给一两个样本，那种「差一点」会被完成度门当成噪声反复触发。
fn native_samples_for(target: usize, native: NativeFormat) -> usize {
    let frames = target
        .saturating_mul(usize::try_from(native.sample_rate).unwrap_or(usize::MAX))
        .div_ceil(usize::try_from(TARGET_SAMPLE_RATE).unwrap_or(1))
        .saturating_add(1);
    frames.saturating_mul(usize::from(native.channels))
}

/// Linux 上输入设备的优先级，数字越小越优先。
///
/// **为什么要排序**：ALSA 的裸 `hw:` 设备是独占的，一旦被浏览器或会议软件拿住，我们
/// 打开它只会拿到 `EBUSY`；而 PipeWire 与 PulseAudio 的桥接设备是混音的，多个进程可以
/// 同时读。因此桥接设备优先不是偏好，是避开一整类「设备被占用」失败的唯一办法。
///
/// 非 Linux 平台上 WASAPI 与 CoreAudio 的共享模式默认就是混音的，没有这个问题，排序
/// 退化成「默认设备优先」。
#[must_use]
pub fn rank_input(name: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("pipewire") {
        return 0;
    }
    if lower.contains("pulse") {
        return 1;
    }
    if lower.contains("default") {
        return 2;
    }
    // 裸 ALSA 硬件设备排最后：能开的时候能开，被占用的时候谁也救不了。
    if lower.starts_with("hw:") || lower.contains("front:") || lower.contains("surround") {
        return 4;
    }
    3
}

/// 按 [`rank_input`] 挑一个输入设备名；列表为空时返回 `None`。
///
/// 同级之间保持枚举顺序，因此同一台机器上的选择是稳定的——不稳定的设备选择会让
/// 「昨天能录今天不能」这种报告无法复现。
#[must_use]
pub fn pick_input(names: &[String]) -> Option<String> {
    names
        .iter()
        .enumerate()
        .min_by_key(|(index, name)| (rank_input(name), *index))
        .map(|(_, name)| name.clone())
}

/// 把 [`crate::VoiceError`] 分类成类型化的 [`AudioError`]。
///
/// # `DeviceBusy` 为什么要靠枚举结果判别
///
/// **`cpal` 把「设备被独占」与「设备不存在」压成了同一个变体。** ALSA 后端把 `EBUSY`
/// 与 `ENODEV`/`ENOENT` 一并映射到 `BuildStreamError::DeviceNotAvailable`
/// （`cpal-0.17.3/src/host/alsa/mod.rs:362`），WASAPI 把 `AUDCLNT_E_DEVICE_IN_USE` 与
/// `AUDCLNT_E_DEVICE_INVALIDATED` 一并映射到同一个变体
/// （`cpal-0.17.3/src/host/wasapi/mod.rs:103`）。于是错误文本本身分不出两者，唯一可用的
/// 判别信号是**枚举结果**：设备出现在可用列表里却打不开，说明它存在而被占用；列表本身
/// 是空的，说明根本没有设备。`listed_devices` 就是这个信号。
#[must_use]
pub fn classify_capture_error(error: &crate::VoiceError, listed_devices: usize) -> AudioError {
    use crate::VoiceError as V;
    match error {
        V::NoInputDevice { detail } => AudioError::NoDevice {
            detail: detail.clone(),
        },
        V::AudioDevice { detail, source } => {
            let text = format!("{detail}：{source}");
            if listed_devices > 0 && looks_unavailable(&source.to_string()) {
                AudioError::DeviceBusy { detail: text }
            } else if listed_devices == 0 {
                AudioError::NoDevice { detail: text }
            } else {
                AudioError::Failed {
                    stage: "打开",
                    detail: text,
                }
            }
        }
        V::CaptureStalled { .. } | V::CaptureTruncated { .. } => AudioError::Failed {
            stage: "采集",
            detail: error.to_string(),
        },
        V::FeatureDisabled
        | V::ModelMissing { .. }
        | V::AudioRead { .. }
        | V::AudioWrite { .. }
        | V::Backend(_) => AudioError::Failed {
            stage: "调用",
            detail: error.to_string(),
        },
    }
}

/// `cpal` 在设备存在但拿不到时给出的两种说法。文本判别是因为它们被压成了同一个变体，
/// 见 [`classify_capture_error`] 的文档。
fn looks_unavailable(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("no longer available")
        || lower.contains("device or resource busy")
        || lower.contains("in use")
        || lower.contains("unplugged")
}

#[cfg(feature = "capture")]
mod device;
#[cfg(feature = "capture")]
pub use device::{RodioInput, capture_default, open_default_input, play_pcm, play_samples};

#[cfg(test)]
mod tests;
