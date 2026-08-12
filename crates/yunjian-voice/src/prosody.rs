//! 朗读节奏：音步切分、静音拼接与逐音步时间戳。
//!
//! **为什么节奏必须由我们自己造。** sherpa-onnx 既不支持 SSML，其 `silence_scale`
//! 参数也已由上游报损（#2043），所以「在音步之间停 120 毫秒、在行之间停 400 毫秒」这件
//! 事没有任何引擎侧的开关可用。唯一可控的做法是**逐音步分别合成，在 Rust 侧把静音拼进
//! 去**——静音是我们自己填的零样本，时长由我们算，不经过任何模型。
//!
//! **这带来一个副产品，而它恰好解掉另一个难题。** 逐段合成天然知道每一段起止于第几个
//! 样本，于是 karaoke 式逐音步高亮所需的时间戳是拼接的算术结果，不需要强制对齐——而强制
//! 对齐上游明确不做（#3536），且识别器只暴露 token 的 **start** 时间、没有 stop 时间
//! （#985）。换句话说：拼接不是「实现节奏的一种办法」，它同时是我们能拿到时间戳的**唯一**
//! 办法。`splice` 的测试因此同时守着这两件事。
//!
//! **本模块不带特性开关。** 切分规则、拼接算术、静音判定全是纯函数，合成本身抽象在
//! [`FootSynthesizer`] 之后，于是一台没有模型、没有声卡的机器仍然能验证节奏是对的；
//! 只有「真的调用 sherpa-onnx」那一小块在 `tts`（`voice` 特性）里。这是 `audio` 与
//! `models` 两个模块已经用过的同一种分层。

use std::time::Duration;

use crate::lexicon::{CiTuneRhythm, RhythmSource};

/// 静音判定阈值，单位 dBFS。低于它且持续足够长即算一段静音。
///
/// 取 −45 而不是 0：合成器输出的「静音」段并非全零，模型尾音与量化噪声通常落在
/// −60～−50 dBFS，而真实语音的音步内部远高于 −45。这个值把两者分开。
pub const SILENCE_FLOOR_DBFS: f32 = -45.0;

/// 判定静音时的分析窗口。
///
/// 1 毫秒够细：要断言的最短间隔是 120 毫秒，窗口比它小两个数量级，边界量化误差
/// 不会把一段合格的静音判成不合格。
pub const SILENCE_WINDOW: Duration = Duration::from_millis(1);

/// 音步之间与行之间的停顿时长。
///
/// 两个值是配置项（`[voice.prosody]` 的 `foot_pause_ms` 与 `line_pause_ms`），
/// **不是常量**——刻意如此：调参不该改动测试的结构，测试断言的是「间隔不短于配置值」，
/// 于是把 120 改成 150 之后测试照样有效，而不是需要同步改一个硬编码数字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prosody {
    /// 同一行内相邻音步之间的静音，毫秒。
    pub foot_pause_ms: u32,
    /// 行与行之间的静音，毫秒。
    pub line_pause_ms: u32,
}

impl Prosody {
    /// 默认停顿。与 `yunjian-core` 的 `VoiceProsodyConfig::default` 必须一致，
    /// 这一点由 `yunjian-cli` 的跨 crate 一致性测试守着（本 crate 不依赖 core）。
    pub const CLASSICAL: Self = Self {
        foot_pause_ms: 120,
        line_pause_ms: 400,
    };

    /// 行内停顿。
    #[must_use]
    pub const fn foot_pause(self) -> Duration {
        Duration::from_millis(self.foot_pause_ms as u64)
    }

    /// 行间停顿。
    #[must_use]
    pub const fn line_pause(self) -> Duration {
        Duration::from_millis(self.line_pause_ms as u64)
    }
}

impl Default for Prosody {
    fn default() -> Self {
        Self::CLASSICAL
    }
}

/// 一个音步：一行之内的一个朗读单位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foot {
    /// 音步文本，不含标点。
    pub text: String,
    /// 所属行在整篇里的序号，从 0 起。
    pub line: usize,
    /// 在本行内的序号，从 0 起。
    pub index_in_line: usize,
}

/// 一行的切分结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 本行音步。
    pub feet: Vec<Foot>,
    /// 本行的切分依据来自哪里。**UI 必须展示它**：`Punctuation` 意味着我们没有词谱，
    /// 只是照着标点断的，不能让界面暗示那是词谱权威。
    pub source: RhythmSource,
}

/// 整篇的切分结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segmentation {
    /// 逐行结果。
    pub lines: Vec<Line>,
}

impl Segmentation {
    /// 全部音步，按朗读顺序。
    #[must_use]
    pub fn feet(&self) -> Vec<&Foot> {
        self.lines
            .iter()
            .flat_map(|line| line.feet.iter())
            .collect()
    }

    /// 音步总数。时间戳数量必须等于它。
    #[must_use]
    pub fn foot_count(&self) -> usize {
        self.lines.iter().map(|line| line.feet.len()).sum()
    }

    /// 全篇是否有任何一行退化成按标点切分。
    #[must_use]
    pub fn any_punctuation_fallback(&self) -> bool {
        self.lines
            .iter()
            .any(|line| line.source == RhythmSource::Punctuation)
    }
}

/// 一行里参与朗读的字，即去掉标点与空白之后剩下的。
fn reading_chars(line: &str) -> Vec<char> {
    line.chars()
        .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
        .collect()
}

/// 标点判定。与 `yunjian-core::derive` 的口径一致：切分器已经把句读吃掉了，
/// 这里只需要把残留的引号、括号之类剔除，免得它们被算进字数而改变音步。
fn is_punctuation(character: char) -> bool {
    matches!(character,
        '\u{3000}'..='\u{303f}'      // CJK 标点
        | '\u{ff00}'..='\u{ffef}'    // 全角形式
        | '!'..='/'
        | ':'..='@'
        | '['..='`'
        | '{'..='~'
    )
}

/// 按字数切一行的音步。
///
/// **五言切二三、七言切二二三**，这两条只看字数，不需要任何外部数据——这正是本任务
/// 唯一不受扣留资产影响的部分。别的字数没有普适规则，退化成整行一个音步：宁可少切，
/// 也不要按一个编出来的规则去切，那会让停顿落在不该停的地方，比不停更糟。
///
/// 六言不切成二二二：那是把「二二三」的直觉外推到没有依据的地方。
#[must_use]
pub fn foot_widths(char_count: usize) -> Vec<usize> {
    match char_count {
        5 => vec![2, 3],
        7 => vec![2, 2, 3],
        0 => Vec::new(),
        other => vec![other],
    }
}

/// 把一行按给定宽度切成音步。宽度之和与字数不符时按宽度顺序尽力切，剩余归入末音步——
/// 这只在词谱句式与实际字数不符时发生，调用方已被 [`segment_ci`] 记为一处不一致。
fn cut(chars: &[char], widths: &[usize], line: usize) -> Vec<Foot> {
    let mut feet = Vec::with_capacity(widths.len());
    let mut cursor = 0;
    for (index_in_line, width) in widths.iter().enumerate() {
        if cursor >= chars.len() {
            break;
        }
        let end = if index_in_line + 1 == widths.len() {
            chars.len()
        } else {
            (cursor + width).min(chars.len())
        };
        feet.push(Foot {
            text: chars[cursor..end].iter().collect(),
            line,
            index_in_line,
        });
        cursor = end;
    }
    if cursor < chars.len() {
        feet.push(Foot {
            text: chars[cursor..].iter().collect(),
            line,
            index_in_line: feet.len(),
        });
    }
    feet
}

/// 切分近体诗：逐行按字数定音步。
///
/// 入参是**已经按格律行切开**的行序列。调用方应当用
/// `yunjian_core::derive::split_metrical_lines`，而不是 `split_rhyme_feet`——后者刻意
/// 不切逗号（它服务韵脚推导），拿它切出来的「行」会把一联的上下句粘成十字或十四字，
/// 于是五言七言的字数判据全部落空。
#[must_use]
pub fn segment_metrical<'a, I>(lines: I) -> Segmentation
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = Vec::new();
    for (line_index, raw) in lines.into_iter().enumerate() {
        let chars = reading_chars(raw);
        if chars.is_empty() {
            continue;
        }
        let widths = foot_widths(chars.len());
        out.push(Line {
            feet: cut(&chars, &widths, out.len()),
            source: if matches!(chars.len(), 5 | 7) {
                RhythmSource::CharCount
            } else {
                RhythmSource::Punctuation
            },
        });
        let _ = line_index;
    }
    Segmentation { lines: out }
}

/// 切分词：优先用词谱句式，词牌不在表里则退化成按标点切分。
///
/// **退化是要被看见的，不是被藏起来的。** 词的句读不能由字数推出，而项目自有的
/// 词谱句式表（`data/citune_rhythm.tsv`）v1 只覆盖有公有领域依据的词牌；表外的词牌
/// 一律按作品自己的标点切，并把该行标成 [`RhythmSource::Punctuation`]，使界面绝不
/// 宣称它没有的词谱权威。这条正是本任务要断言的那条。
#[must_use]
pub fn segment_ci(lines: &[&str], rhythm: Option<&CiTuneRhythm>) -> Segmentation {
    let non_empty: Vec<Vec<char>> = lines
        .iter()
        .map(|line| reading_chars(line))
        .filter(|chars| !chars.is_empty())
        .collect();

    // 词谱句式按「整篇的句序」给出，因此只有句数对得上才敢用：句数不符说明这一首
    // 与表里那支词牌的体式不同（同名异体在词里很常见），此时用词谱会把停顿放错，
    // 不如老实退化。
    let usable = rhythm.filter(|spec| spec.pattern.len() == non_empty.len());

    let mut out = Vec::with_capacity(non_empty.len());
    for (index, chars) in non_empty.iter().enumerate() {
        let (widths, source) = match usable {
            Some(spec) => (
                foot_widths_for_ci(spec.pattern[index], chars.len()),
                spec.source,
            ),
            None => (vec![chars.len()], RhythmSource::Punctuation),
        };
        out.push(Line {
            feet: cut(chars, &widths, index),
            source,
        });
    }
    Segmentation { lines: out }
}

/// 词的一句内部怎么切。
///
/// 句式表给的是**句长**，不是句内音步。句长与实际字数一致时，再按诗的字数规则去切
/// 五言七言的句内音步（词里的五言句与诗的五言句同为二三）；不一致时不切，整句一个
/// 音步——句式与文本已经对不上，再按字数细切只是把错误放大。
fn foot_widths_for_ci(declared: usize, actual: usize) -> Vec<usize> {
    if declared == actual {
        foot_widths(actual)
    } else {
        vec![actual]
    }
}

/// 一段已合成的音频。与 `tts::Synthesized` 同形，但**不依赖 `voice` 特性**——
/// 这是判定层能在没有模型的机器上被测到的原因。
#[derive(Debug, Clone, PartialEq)]
pub struct FootAudio {
    /// 归一到 `[-1.0, 1.0]` 的单声道样本。
    pub samples: Vec<f32>,
    /// 采样率。
    pub sample_rate: u32,
}

/// 逐音步合成器。
///
/// **存在的理由是可测性，与 `audio::InputDevice` 完全同因**：拼接算术与时间戳必须能在
/// 没有模型权重的机器上被验证，所以真实合成器（`tts::Synthesizer`）与测试用的假合成器
/// 都实现这一个接口，而 [`splice`] 只认接口。
pub trait FootSynthesizer {
    /// 合成一个音步。
    ///
    /// # Errors
    ///
    /// 由实现方定义；[`splice`] 只负责原样上抛。
    fn synthesize_foot(&mut self, text: &str) -> Result<FootAudio, crate::VoiceError>;
}

/// 一个音步在输出缓冲里的位置。
///
/// **由样本数算出，不由播放墙钟测量。** 墙钟会把设备缓冲与调度抖动混进来，而高亮
/// 需要的是「这一段声音在这个缓冲里的第几个样本」——那是个确定值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootMark {
    /// 所在行序号。
    pub line: usize,
    /// 行内序号。
    pub index_in_line: usize,
    /// 音步文本。
    pub text: String,
    /// 起始样本下标。
    pub start_sample: usize,
    /// 结束样本下标（不含）。
    pub end_sample: usize,
}

impl FootMark {
    /// 起始时刻。
    #[must_use]
    pub fn start(&self, sample_rate: u32) -> Duration {
        samples_to_duration(self.start_sample, sample_rate)
    }

    /// 结束时刻。
    #[must_use]
    pub fn end(&self, sample_rate: u32) -> Duration {
        samples_to_duration(self.end_sample, sample_rate)
    }
}

/// 拼接结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// 拼接后的整段音频。
    pub samples: Vec<f32>,
    /// 采样率。
    pub sample_rate: u32,
    /// 逐音步时间戳。**长度恒等于音步数**——这是本模块对外的硬契约，karaoke 高亮据此
    /// 逐音步推进，少一个就会错位到句末。
    pub marks: Vec<FootMark>,
}

impl Reading {
    /// 总时长。
    #[must_use]
    pub fn duration(&self) -> Duration {
        samples_to_duration(self.samples.len(), self.sample_rate)
    }
}

fn samples_to_duration(samples: usize, sample_rate: u32) -> Duration {
    if sample_rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(samples as f64 / f64::from(sample_rate))
}

fn silence_samples(pause: Duration, sample_rate: u32) -> usize {
    let millis = u64::try_from(pause.as_millis()).unwrap_or(u64::MAX);
    usize::try_from(u64::from(sample_rate) * millis / 1_000).unwrap_or(usize::MAX)
}

/// 逐音步合成并拼接成一段朗读。
///
/// 这个函数就是「节奏」的全部实现：它在音步之间插 `foot_pause_ms` 的零样本、在行之间插
/// `line_pause_ms` 的零样本，并顺带记下每个音步落在哪几个样本上。**把整行一次喂给引擎
/// 得不到这些间隔**，那是本任务的失败场景要证明的事。
///
/// 行末不补静音（末音步之后就是行间静音），全篇末尾也不补：尾部静音只会让播放拖沓，
/// 且会让「总时长」这个量失去意义。
///
/// # Errors
///
/// 上抛合成器的错误；另外在合成器返回的采样率前后不一致时返回
/// [`VoiceError::Backend`](crate::VoiceError::Backend)——不同采样率的段直接首尾相接会
/// 变调，必须当作错误而不是默默拼上。
pub fn splice(
    synthesizer: &mut dyn FootSynthesizer,
    segmentation: &Segmentation,
    prosody: Prosody,
) -> Result<Reading, crate::VoiceError> {
    let mut samples: Vec<f32> = Vec::new();
    let mut marks: Vec<FootMark> = Vec::new();
    let mut sample_rate: Option<u32> = None;

    for (line_position, line) in segmentation.lines.iter().enumerate() {
        if line_position > 0 && !samples.is_empty() {
            let rate = sample_rate.unwrap_or_default();
            samples.extend(std::iter::repeat_n(
                0.0,
                silence_samples(prosody.line_pause(), rate),
            ));
        }
        for (foot_position, foot) in line.feet.iter().enumerate() {
            if foot_position > 0 {
                let rate = sample_rate.unwrap_or_default();
                samples.extend(std::iter::repeat_n(
                    0.0,
                    silence_samples(prosody.foot_pause(), rate),
                ));
            }
            let audio = synthesizer.synthesize_foot(&foot.text)?;
            match sample_rate {
                None => sample_rate = Some(audio.sample_rate),
                Some(rate) if rate != audio.sample_rate => {
                    return Err(crate::VoiceError::Backend(format!(
                        "音步 `{}` 的采样率 {} Hz 与前面的 {} Hz 不一致；\
                         不同采样率的段首尾相接会变调，故拒绝拼接",
                        foot.text, audio.sample_rate, rate
                    )));
                }
                Some(_) => {}
            }
            let start_sample = samples.len();
            samples.extend_from_slice(&audio.samples);
            marks.push(FootMark {
                line: foot.line,
                index_in_line: foot.index_in_line,
                text: foot.text.clone(),
                start_sample,
                end_sample: samples.len(),
            });
        }
    }

    Ok(Reading {
        samples,
        sample_rate: sample_rate.unwrap_or_default(),
        marks,
    })
}

/// 输出缓冲里的一段静音。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentSpan {
    /// 起始样本下标。
    pub start_sample: usize,
    /// 结束样本下标（不含）。
    pub end_sample: usize,
}

impl SilentSpan {
    /// 样本数。
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end_sample - self.start_sample
    }

    /// 是否为空。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start_sample >= self.end_sample
    }

    /// 时长。
    #[must_use]
    pub fn duration(&self, sample_rate: u32) -> Duration {
        samples_to_duration(self.len(), sample_rate)
    }
}

/// dBFS 阈值换成线性 RMS 阈值。
#[must_use]
pub fn dbfs_to_rms(dbfs: f32) -> f32 {
    10.0_f32.powf(dbfs / 20.0)
}

/// 找出缓冲里所有静音段。
///
/// 静音的定义是**持续**低于 `floor_dbfs` 的 RMS，逐 [`SILENCE_WINDOW`] 窗口判定后
/// 把相邻窗口并起来。用 RMS 而不是逐样本比较绝对值：语音波形每个周期都要过零，逐样本
/// 判定会把每一个过零点都算成一段静音，得到几千段长度为 1 的「静音」而不是我们要的间隔。
#[must_use]
pub fn silent_spans(samples: &[f32], sample_rate: u32, floor_dbfs: f32) -> Vec<SilentSpan> {
    let window = silence_samples(SILENCE_WINDOW, sample_rate).max(1);
    let threshold = dbfs_to_rms(floor_dbfs);
    let mut spans: Vec<SilentSpan> = Vec::new();
    let mut cursor = 0;
    while cursor < samples.len() {
        let end = (cursor + window).min(samples.len());
        if crate::rms(&samples[cursor..end]) < threshold {
            match spans.last_mut() {
                Some(last) if last.end_sample == cursor => last.end_sample = end,
                _ => spans.push(SilentSpan {
                    start_sample: cursor,
                    end_sample: end,
                }),
            }
        }
        cursor = end;
    }
    spans
}

/// 相邻两个音步之间实际留出的静音时长。
///
/// 取的是「前一音步结束到后一音步开始」这一区间内**最长**的静音段，而不是区间长度：
/// 合成器的尾音可能越过标记边界，只有真正低于阈值的那一截才算停顿。返回 `None` 表示
/// 该区间内没有任何一段静音。
#[must_use]
pub fn gap_between(reading: &Reading, first: usize, second: usize) -> Option<Duration> {
    let from = reading.marks.get(first)?.end_sample;
    let to = reading.marks.get(second)?.start_sample;
    if to <= from {
        return None;
    }
    silent_spans(
        &reading.samples[from..to],
        reading.sample_rate,
        SILENCE_FLOOR_DBFS,
    )
    .into_iter()
    .map(|span| span.duration(reading.sample_rate))
    .max()
}

#[cfg(test)]
mod tests;
