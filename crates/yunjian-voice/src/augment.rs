//! 音频增强。
//!
//! **为什么需要它**：本项目拿不到许可的真人朗诵语料——唯一公开的 MCGA 是
//! CC BY-NC-SA-4.0 且只放了 test split，NC 条款直接排除本项目。于是 CER 只能用
//! TTS 合成音测量，而合成音是「干净得不真实」的：单一说话人、无信道噪声、无语速抖动。
//! 直接拿它测出来的 CER 会乐观到没有参考价值。
//!
//! 标准做法是在合成音上叠加信道与说话人变异的近似：窄带往返（模拟电话/低端麦克风的
//! 带宽损失）、粉噪（模拟环境底噪，粉噪而非白噪，因为真实房间噪声的频谱是向低频倾斜的）、
//! 时间伸缩（模拟语速差异，且必须保持音高——变速不变调才像「另一个人说得快一点」，
//! 单纯重采样会连音高一起改，那是另一个人的声带，不是同一个人的语速）。
//!
//! 这仍然只是**近似**：它逼近的是信道与语速，逼近不了口音、韵律习惯、吞音与方言底色。
//! 因此由它得出的 CER 永远是真人 CER 的**乐观上界**，`docs/reports/asr-cer.md`
//! 必须原样写明这一点。
//!
//! 本模块不依赖 `voice` 特性：全是纯 Rust 的信号处理，因此默认构建也能跑它的单元测试。

/// 电话带宽的上限。3.4 kHz 是 ITU-T G.711 语音频带的习惯取值，配 8 kHz 采样率
/// 恰好留出过渡带；取到 4 kHz 会正好压在奈奎斯特频率上，抗混叠滤波器无处发挥。
const NARROWBAND_CUTOFF_HZ: f32 = 3400.0;
/// 窄带往返的中间采样率。
const NARROWBAND_RATE: u32 = 8000;
/// 低通 FIR 的抽头数。63 阶在 16 kHz 上约 -40 dB 阻带衰减，足以让混叠低于粉噪底噪。
const FIR_TAPS: usize = 63;

/// 重采样到 8 kHz 再回到原采样率。
///
/// 两次重采样各自先低通再改采样率：只做抽取不做抗混叠，高频会折叠回语音频带，
/// 那是**加进去的**假信号，不是「带宽变窄」，会让 CER 恶化得毫无物理意义。
#[must_use]
pub fn narrowband_roundtrip(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || sample_rate <= NARROWBAND_RATE {
        return samples.to_vec();
    }
    let down = resample(
        &lowpass(samples, sample_rate, NARROWBAND_CUTOFF_HZ),
        sample_rate,
        NARROWBAND_RATE,
    );
    let up = resample(&down, NARROWBAND_RATE, sample_rate);
    lowpass(&up, sample_rate, NARROWBAND_CUTOFF_HZ)
}

/// 改采样率并在降采样时先做抗混叠低通。
///
/// **降采样前必须低通**：TTS 输出是 24 或 44.1 kHz，直接抽取到 16 kHz 会把 8 kHz 以上
/// 的能量折叠回语音频带，那是凭空加进去的假信号。它会让 CER 恶化，而恶化的原因是
/// 本测量自己的处理链，不是识别器——那种数字没有意义。
///
/// 截止取目标奈奎斯特频率的 95%，留出滤波器过渡带。
#[must_use]
pub fn resample_antialiased(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from <= to {
        return resample(samples, from, to);
    }
    #[expect(clippy::cast_precision_loss, reason = "采样率取值远在 f32 精度内")]
    let cutoff = to as f32 / 2.0 * 0.95;
    resample(&lowpass(samples, from, cutoff), from, to)
}

/// 线性插值重采样。**不做抗混叠**，降采样请用 `resample_antialiased`。
///
/// 只在已经低通过的信号上调用，因此不需要更高阶的插值核：带宽已被限制在目标奈奎斯特
/// 频率以下时，线性插值引入的失真远低于粉噪底噪。
#[must_use]
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "采样点数远小于 f64 的精确整数范围，向下取整即所需语义"
    )]
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "同上：索引规模不会触及 f64 精度边界"
            )]
            let pos = i as f64 * ratio;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "pos 非负且小于 samples.len()"
            )]
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            #[expect(clippy::cast_possible_truncation, reason = "插值系数只需 f32 精度")]
            let frac = (pos - pos.floor()) as f32;
            samples[lo] * (1.0 - frac) + samples[hi] * frac
        })
        .collect()
}

/// 汉明窗 sinc 低通。零相位（对称核 + 居中输出），因此不引入群延迟偏移，
/// 增强前后的时间轴可以直接对齐。
#[must_use]
pub fn lowpass(samples: &[f32], sample_rate: u32, cutoff_hz: f32) -> Vec<f32> {
    #[expect(clippy::cast_precision_loss, reason = "采样率取值远在 f32 精度内")]
    let nyquist = sample_rate as f32 / 2.0;
    if samples.is_empty() || cutoff_hz >= nyquist {
        return samples.to_vec();
    }
    let fc = cutoff_hz / nyquist; // 归一化到 (0, 1)
    let mid = (FIR_TAPS / 2) as isize;
    let mut kernel = [0.0f32; FIR_TAPS];
    let mut sum = 0.0f32;
    for (i, tap) in kernel.iter_mut().enumerate() {
        let n = i as isize - mid;
        #[expect(clippy::cast_precision_loss, reason = "抽头序号是小整数")]
        let x = n as f32 * fc * std::f32::consts::PI;
        let sinc = if n == 0 { 1.0 } else { x.sin() / x };
        #[expect(clippy::cast_precision_loss, reason = "抽头序号是小整数")]
        let w = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FIR_TAPS - 1) as f32).cos();
        *tap = sinc * w;
        sum += *tap;
    }
    for tap in &mut kernel {
        *tap /= sum;
    }

    let len = samples.len();
    (0..len)
        .map(|i| {
            let mut acc = 0.0f32;
            for (k, tap) in kernel.iter().enumerate() {
                let idx = i as isize + k as isize - mid;
                // 边界按零填充。语音首尾本就是静音，零填充不会造成可听的截断噪声。
                if idx >= 0 && (idx as usize) < len {
                    acc += samples[idx as usize] * tap;
                }
            }
            acc
        })
        .collect()
}

/// 按目标信噪比混入粉噪。
///
/// `snr_db` 是**信号相对噪声**的分贝数，越小越吵。`seed` 固定随机序列：
/// 测量必须可复现，否则同一份 fixture 两次跑出两个 CER，谁也说不清是模型变了还是噪声变了。
#[must_use]
pub fn mix_pink_noise(samples: &[f32], snr_db: f32, seed: u64) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let signal_rms = rms(samples);
    if signal_rms <= f32::EPSILON {
        return samples.to_vec();
    }
    let noise = pink_noise(samples.len(), seed);
    let noise_rms = rms(&noise);
    if noise_rms <= f32::EPSILON {
        return samples.to_vec();
    }
    // signal_rms / (gain * noise_rms) = 10^(snr/20)
    let gain = signal_rms / (noise_rms * 10.0_f32.powf(snr_db / 20.0));
    samples
        .iter()
        .zip(noise.iter())
        .map(|(s, n)| (s + n * gain).clamp(-1.0, 1.0))
        .collect()
}

/// Paul Kellet 的经济型粉噪滤波器（白噪经 -3 dB/oct 整形）。
///
/// 选它而不是 Voss-McCartney：后者的频谱是分段阶梯，前者在语音频带内更接近真实
/// 房间噪声的连续斜率。
#[must_use]
pub fn pink_noise(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = Xorshift64(seed | 1);
    let mut b = [0.0f32; 7];
    (0..len)
        .map(|_| {
            let white = rng.next_bipolar();
            b[0] = 0.990_05 * b[0] + white * 0.055_264;
            b[1] = 0.963_00 * b[1] + white * 0.074_046;
            b[2] = 0.575_00 * b[2] + white * 0.153_852;
            let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + white * 0.546_5;
            b[3] = 0.868_6 * b[3] + white * 0.011_531;
            b[4] = 0.550_0 * b[4] + white * 0.007_678;
            b[5] = -0.761_6 * b[5] - white * 0.016_898;
            b[6] = white * 0.115_926;
            pink * 0.2
        })
        .collect()
}

/// WSOLA 变速不变调。`ratio` 是输出与输入的时长比：1.1 慢 10%，0.9 快 10%。
///
/// 为什么不用重采样：重采样把 1.1 倍时长和 1.1 倍波长绑在一起，音高随之下移
/// 约 1.6 个半音。那模拟的是「换了一个人」，而增强要模拟的是「同一个人语速不同」。
/// WSOLA 在相邻帧的自然延续处做互相关搜索后再叠加，因此周期结构不被破坏，音高不变。
#[must_use]
pub fn time_stretch(samples: &[f32], ratio: f32) -> Vec<f32> {
    const FRAME: usize = 1024;
    const SEARCH: usize = 128;
    let hop_out = FRAME / 2;

    if samples.len() < FRAME * 2 || !(0.5..=2.0).contains(&ratio) || (ratio - 1.0).abs() < 1e-6 {
        return samples.to_vec();
    }
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "帧长与跳距都是小整数"
    )]
    let hop_in = ((hop_out as f32) / ratio).round() as usize;
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "输出长度由输入长度乘常数得出"
    )]
    let target_len = ((samples.len() as f32) * ratio).round() as usize;

    let window: Vec<f32> = (0..FRAME)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "窗序号是小整数")]
            let x = 2.0 * std::f32::consts::PI * i as f32 / FRAME as f32;
            0.5 - 0.5 * x.cos()
        })
        .collect();

    let mut out = vec![0.0f32; target_len + FRAME];
    let mut read = 0usize;
    let mut write = 0usize;

    while write + FRAME <= out.len() && read + FRAME <= samples.len() {
        for i in 0..FRAME {
            out[write + i] += samples[read + i] * window[i];
        }
        // 上一帧「自然延续」的那段，就是下一帧最该长得像的目标。
        let natural = read + hop_out;
        let nominal = read + hop_in;
        read = best_match(samples, natural, nominal, SEARCH, hop_out);
        write += hop_out;
    }

    out.truncate(target_len);
    out
}

/// 在 `nominal ± search` 里找与 `samples[natural..]` 最相关的起点。
///
/// 判据是**归一化**互相关而不是裸点积。裸点积会偏向能量大的候选而不是相位对得上的候选，
/// 于是叠加处出现相位跳变——听起来还是同一个音高，但零交叉率会漂，实测约 10%，
/// 与「直接重采样改音高」已经区分不开。归一化把能量因子除掉，只比波形形状。
fn best_match(
    samples: &[f32],
    natural: usize,
    nominal: usize,
    search: usize,
    corr_len: usize,
) -> usize {
    let last = samples.len().saturating_sub(corr_len + 1);
    if natural + corr_len >= samples.len() {
        return nominal.min(last);
    }
    let target = &samples[natural..natural + corr_len];
    let lo = nominal.saturating_sub(search);
    let hi = (nominal + search).min(last);
    let mut best = nominal.min(last);
    let mut best_score = f32::NEG_INFINITY;
    for cand in lo..=hi {
        if cand + corr_len > samples.len() {
            break;
        }
        let window = &samples[cand..cand + corr_len];
        let dot: f32 = target.iter().zip(window).map(|(a, b)| a * b).sum();
        let energy: f32 = window.iter().map(|b| b * b).sum::<f32>().sqrt();
        let score = if energy > f32::EPSILON {
            dot / energy
        } else {
            f32::NEG_INFINITY
        };
        if score > best_score {
            best_score = score;
            best = cand;
        }
    }
    best
}

fn rms(samples: &[f32]) -> f32 {
    crate::rms(samples)
}

/// xorshift64*。够随机、无依赖、可复现，粉噪的白噪源不需要密码学强度。
struct Xorshift64(u64);

impl Xorshift64 {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// 均匀分布于 [-1, 1)。
    fn next_bipolar(&mut self) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "只取高 24 位，恰在 f32 可精确表示的整数范围内"
        )]
        let unit = (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32;
        unit * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        mix_pink_noise, narrowband_roundtrip, pink_noise, resample, resample_antialiased, rms,
        time_stretch,
    };

    fn sine(freq: f32, secs: f32, rate: u32) -> Vec<f32> {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "测试信号规模很小"
        )]
        let n = (secs * rate as f32) as usize;
        (0..n)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "同上")]
                let t = i as f32 / rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect()
    }

    /// 稳态段的零交叉率。只取中间 80%：首尾是加窗与零填充的过渡区，
    /// 那里的零交叉密度反映的是边界处理而不是音高。
    fn zero_crossing_rate(samples: &[f32]) -> f32 {
        let skip = samples.len() / 10;
        let core = &samples[skip..samples.len() - skip];
        let crossings = core
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        #[expect(clippy::cast_precision_loss, reason = "测试信号规模很小")]
        let rate = crossings as f32 / core.len() as f32;
        rate
    }

    /// 混噪后实测信噪比必须等于请求值。这条断言守的是增益公式：写错了噪声会淹掉信号，
    /// 而 CER 报告只会显示「模型很差」，看不出是 fixture 造错了。
    #[test]
    fn pink_noise_hits_the_requested_snr() {
        let clean = sine(440.0, 1.0, 16_000);
        for target in [20.0f32, 10.0] {
            let noisy = mix_pink_noise(&clean, target, 7);
            let residual: Vec<f32> = noisy.iter().zip(clean.iter()).map(|(n, c)| n - c).collect();
            let measured = 20.0 * (rms(&clean) / rms(&residual)).log10();
            assert!(
                (measured - target).abs() < 0.5,
                "请求 {target} dB，实测 {measured} dB"
            );
        }
    }

    /// 粉噪的频谱必须真的向低频倾斜，否则它只是白噪，模拟不了房间底噪。
    /// 判据用一阶差分的能量：白噪的差分能量约为自身的 √2 倍，粉噪明显更低。
    #[test]
    fn pink_noise_is_spectrally_tilted_not_white() {
        let pink = pink_noise(32_768, 11);
        let diff: Vec<f32> = pink.windows(2).map(|w| w[1] - w[0]).collect();
        let tilt = rms(&diff) / rms(&pink);
        assert!(tilt < 1.0, "差分/自身能量比 {tilt} 不像粉噪（白噪约 1.41）");
    }

    /// 变速必须不变调。这条断言是 WSOLA 与「直接重采样」的唯一分水岭：
    /// 重采样会让零交叉率随比例一起变，WSOLA 不会。
    #[test]
    fn time_stretch_preserves_pitch_while_changing_duration() {
        let rate = 16_000;
        let clean = sine(440.0, 2.0, rate);
        let base_zc_rate = zero_crossing_rate(&clean);

        for ratio in [0.9f32, 1.1] {
            let stretched = time_stretch(&clean, ratio);
            let len_ratio = stretched.len() as f32 / clean.len() as f32;
            assert!(
                (len_ratio - ratio).abs() < 0.03,
                "时长比 {len_ratio} 偏离目标 {ratio}"
            );
            let zc_rate = zero_crossing_rate(&stretched);
            assert!(
                (zc_rate / base_zc_rate - 1.0).abs() < 0.05,
                "零交叉率从 {base_zc_rate} 变成 {zc_rate}，说明音高被改了（那就是重采样，不是变速）"
            );
            assert!(rms(&stretched) > rms(&clean) * 0.5, "变速后能量塌了");
        }
    }

    /// 对照实验：重采样确实会改音高。没有这条，上一条断言可能只是因为 ratio 太小而恰好通过。
    #[test]
    fn plain_resampling_does_change_pitch() {
        let clean = sine(440.0, 1.0, 16_000);
        let slower = resample(&clean, 16_000, 14_545); // 约 1.1 倍时长
        let base = zero_crossing_rate(&clean);
        let after = zero_crossing_rate(&slower);
        assert!(
            (after / base - 1.0).abs() > 0.05,
            "重采样理应改变零交叉率，实测 {base} -> {after}"
        );
    }

    /// 窄带往返必须真的削掉高频，且长度不变——长度变了后续按样点对齐的处理会错位。
    #[test]
    fn narrowband_roundtrip_attenuates_above_the_telephone_band() {
        let rate = 16_000;
        let low = sine(300.0, 0.5, rate);
        let high = sine(6_000.0, 0.5, rate);
        let low_out = narrowband_roundtrip(&low, rate);
        let high_out = narrowband_roundtrip(&high, rate);

        assert!(
            (low_out.len() as isize - low.len() as isize).abs() <= 2,
            "长度不该变：{} -> {}",
            low.len(),
            low_out.len()
        );
        let low_keep = rms(&low_out) / rms(&low);
        let high_keep = rms(&high_out) / rms(&high);
        assert!(low_keep > 0.7, "300 Hz 被削掉了：保留 {low_keep}");
        assert!(high_keep < 0.1, "6 kHz 没被削掉：保留 {high_keep}");
    }

    /// 降采样必须先抗混叠：24 kHz 里 10 kHz 的分量下到 16 kHz 时会折叠成 6 kHz，
    /// 那是凭空多出来的信号。这条断言比较两条路径在折叠频点上的能量。
    #[test]
    fn antialiased_downsampling_suppresses_the_folded_image() {
        let tone = sine(10_000.0, 0.5, 24_000);
        let naive = resample(&tone, 24_000, 16_000);
        let clean = resample_antialiased(&tone, 24_000, 16_000);
        assert!(
            rms(&clean) < rms(&naive) * 0.25,
            "抗混叠后残留 {} 相比朴素抽取 {} 没有明显下降",
            rms(&clean),
            rms(&naive)
        );
    }

    /// 升采样不该被低通削弱：那条路径没有混叠问题。
    #[test]
    fn upsampling_is_left_alone() {
        let tone = sine(300.0, 0.2, 16_000);
        let up = resample_antialiased(&tone, 16_000, 24_000);
        assert!(rms(&up) > rms(&tone) * 0.9, "升采样能量塌了");
    }

    #[test]
    fn degenerate_inputs_pass_through_unchanged() {
        assert!(narrowband_roundtrip(&[], 16_000).is_empty());
        assert!(mix_pink_noise(&[], 20.0, 1).is_empty());
        let silence = vec![0.0f32; 4096];
        assert_eq!(mix_pink_noise(&silence, 20.0, 1), silence);
        let short = vec![0.1f32; 32];
        assert_eq!(time_stretch(&short, 1.1), short);
    }
}
