//! 无需麦克风、模型与网络的三种打字练习形态。
//!
//! 挖空、首字提示与遮挡都只负责**出题**：把原文按各自规则遮起来给用户看。
//! 作答一律走 [`crate::score_typed`] 这一个内核，因此打字路径与语音路径共用同一套
//! 对齐与评分实现，而不是各写一份。这也是本模块存在的全部意义：
//! 键入输入是确定性的，于是评分内核在接入语音噪声之前就有了可复现的测试基座。
//!
//! 这三种形态同时是**每一条语音失败路径的降级落点**（无设备、拒绝授权、缺模型、
//! 系统版本过低）。为此它们不依赖 `voice` 与 `capture` 任何特性，本 crate 也不
//! 依赖 `yunjian-voice`——见 `no_hidden_voice_dependency` 用例。
//!
//! # 两种切句粒度分别用在哪里
//!
//! 语料层刻意保留两套切句语义（见 `yunjian_core::derive`），本模块两者都要用：
//!
//! - [`yunjian_core::split_rhyme_feet`]（只切 `\n。！？`）给出**韵脚**。韵脚落在句末，
//!   逗号前的字不是韵脚，所以挖空的优先级判定只能用这一个。
//! - [`yunjian_core::split_metrical_lines`]（切 `，。！？；`）给出**呈现给用户的行**。
//!   《静夜思》在读者眼里是四行而不是两句，所以首字提示与逐行遮挡只能用这一个。
//!
//! 把两者合并成一个「通用切句」会同时毁掉两件事：韵脚里混进句中字，行呈现又变成半首。

use crate::score::{Poem, TypedAttempt, TypedScore, score_typed};
use std::ops::Range;
use yunjian_core::{
    CorpusHandle, Result, content_chars, is_punctuation, split_metrical_lines, split_rhyme_feet,
};

/// 提示文本中代表被遮住一字的字符。
///
/// 刻意选全角下划线而不是 ASCII `_`：后者是标点，会被 [`content_chars`] 剥掉，
/// 于是「提示里有几个空」这件事在任何按正文字符计数的地方都看不见了。
/// 提示文本只用于展示，永远不会被送进评分入口。
pub const MASK_CHARACTER: char = '＿';

/// 文言虚词表，用于把挖空偏向实词。
///
/// 只收无歧义的虚词：`故`（故乡）、`不`（否定副词，挖掉恰好考记忆）这类会误伤实义的
/// 一律不收。表小且写死，是因为工作区禁用分词器（jieba / lindera 等），而挖空只需要
/// 「这个字大概率不承载意义」这一档判断，不需要真正的词法分析。
const FUNCTION_WORDS: &[char] = &[
    '之', '乎', '者', '也', '而', '其', '以', '于', '於', '与', '與', '则', '則', '乃', '矣', '焉',
    '哉', '兮', '夫', '且', '所', '耳', '是', '为', '為',
];

/// 挖空比例与随机种子。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClozeOptions {
    ratio: f32,
    seed: u64,
}

impl ClozeOptions {
    /// 方案与命令行默认的挖空比例。
    pub const DEFAULT_RATIO: f32 = 0.3;

    /// 构造挖空参数；比例被规整到 `[0.0, 1.0]`，非有限值按 `0.0` 处理。
    ///
    /// 规整而不是报错：比例来自界面滑块或命令行参数，越界只说明输入没夹紧，
    /// 而一次练习会话没有必须中止的理由。
    #[must_use]
    pub fn new(ratio: f32, seed: u64) -> Self {
        let ratio = if ratio.is_finite() {
            ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self { ratio, seed }
    }

    /// 用默认比例构造。
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self::new(Self::DEFAULT_RATIO, seed)
    }

    /// 返回规整后的挖空比例。
    #[must_use]
    pub const fn ratio(&self) -> f32 {
        self.ratio
    }

    /// 返回随机种子。同一种子在任何机器、任何次运行上给出完全相同的空位。
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }
}

/// 遮挡进度：已被遮住的行数，`0` 为全文可见。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaskStage(usize);

impl MaskStage {
    /// 全文可见，一个字都不遮。
    pub const FULL_TEXT: Self = Self(0);

    /// 全部遮住。会话建立时按真实行数收敛为该首诗的最大档位。
    pub const ALL: Self = Self(usize::MAX);

    /// 指定遮住前若干行。超过真实行数时在会话建立时收敛为全遮挡。
    #[must_use]
    pub const fn new(masked_lines: usize) -> Self {
        Self(masked_lines)
    }

    /// 返回被遮住的行数。
    #[must_use]
    pub const fn masked_lines(&self) -> usize {
        self.0
    }

    /// 推进一档。到顶后保持不动。
    #[must_use]
    pub const fn advanced(&self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// 一种打字练习形态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PracticeMode {
    /// 挖空：按比例挖掉若干字，优先挖韵脚，其次实词。
    Cloze(ClozeOptions),
    /// 首字提示：每行只留第一个字。
    FirstChar,
    /// 遮挡：从全文可见逐行遮到全遮。
    Masked(MaskStage),
}

impl PracticeMode {
    /// 三种形态的默认配置，供界面一键各起一局。
    ///
    /// 语音路径的任一失败点都用这三个入口降级；顺序即界面按钮顺序，
    /// 由浅到深：全文可见的遮挡、首字提示、默认比例挖空。
    pub const ALL: [Self; 3] = [
        Self::Masked(MaskStage::FULL_TEXT),
        Self::FirstChar,
        Self::Cloze(ClozeOptions {
            ratio: ClozeOptions::DEFAULT_RATIO,
            seed: 0,
        }),
    ];

    /// 返回稳定的形态标识，供日志与命令行展示。
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Cloze(_) => "cloze",
            Self::FirstChar => "first-char",
            Self::Masked(_) => "masked",
        }
    }
}

/// 一局打字练习：提示文本加上共用评分内核所需的参考诗文。
///
/// 建立会话只有一次调用，因此语音界面在任一失败点都能「一键到达」任意形态。
#[derive(Debug, Clone)]
pub struct PracticeSession {
    mode: PracticeMode,
    reference: Poem,
    prompt: String,
    hidden: Vec<usize>,
    line_count: usize,
}

impl PracticeSession {
    /// 按指定形态为一首诗出题。
    ///
    /// `body` 是带标点的原文：提示文本要保留原有断句才看得出诗的形状。
    /// 参考诗文另经 [`Poem::new`] 归一化，两者的下标互不通用，见 [`Self::hidden_indices`]。
    pub fn start(handle: &CorpusHandle, body: &str, mode: PracticeMode) -> Result<Self> {
        let reference = Poem::new(handle, body)?;
        let layout = ContentLayout::of(body);
        let mode = clamp_mode(mode, layout.line_spans.len());
        let hidden = hidden_indices(mode, &layout);
        let prompt = render_prompt(body, &hidden, layout.content_len());
        Ok(Self {
            mode,
            reference,
            prompt,
            hidden,
            line_count: layout.line_spans.len(),
        })
    }

    /// 返回本局形态。遮挡档位已按真实行数收敛。
    #[must_use]
    pub const fn mode(&self) -> PracticeMode {
        self.mode
    }

    /// 返回展示给用户的提示文本：保留原标点，被遮处为 [`MASK_CHARACTER`]。
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// 返回被遮住的位置，升序。
    ///
    /// 下标是**原文正文字符**（[`content_chars`] 序列）的位置，只用于出题与展示。
    /// 评分走归一化后的参考诗文，从不读这些下标——`variant_map` 的改写不保证逐字
    /// 一对一，两套下标一旦混用就会错位。
    #[must_use]
    pub fn hidden_indices(&self) -> &[usize] {
        &self.hidden
    }

    /// 返回共用评分内核使用的参考诗文。
    #[must_use]
    pub const fn reference(&self) -> &Poem {
        &self.reference
    }

    /// 返回按呈现行切分得到的行数。
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.line_count
    }

    /// 返回遮挡形态的总档位数：全文可见加上逐行遮挡直至全遮。
    #[must_use]
    pub const fn stage_count(&self) -> usize {
        self.line_count + 1
    }

    /// 为一次键入作答评分。
    ///
    /// 这里只做转发。三种形态共用 [`score_typed`] 是本模块的设计前提，
    /// 打字输入不存在第二条评分路径。
    #[must_use]
    pub fn score(&self, attempt: &TypedAttempt) -> TypedScore {
        score_typed(&self.reference, attempt)
    }
}

/// 原文正文字符的版式：逐字内容、呈现行区间与韵脚位置。
///
/// 三份数据的下标口径完全一致，都是 [`content_chars`] 序列的位置。两个切句器切出的
/// 分隔符全是标点或空白，一律不进正文序列，所以逐段累加字数就是每段的准确区间。
struct ContentLayout {
    characters: Vec<char>,
    line_spans: Vec<Range<usize>>,
    rhyme_feet: Vec<usize>,
}

impl ContentLayout {
    fn of(body: &str) -> Self {
        let characters = content_chars(body).collect::<Vec<_>>();
        let mut line_spans = Vec::new();
        let mut cursor = 0;
        for line in split_metrical_lines(body) {
            let length = content_chars(line).count();
            if length == 0 {
                continue;
            }
            line_spans.push(cursor..cursor + length);
            cursor += length;
        }
        let mut rhyme_feet = Vec::new();
        let mut foot_cursor = 0;
        for foot in split_rhyme_feet(body) {
            let length = content_chars(foot).count();
            if length == 0 {
                continue;
            }
            foot_cursor += length;
            rhyme_feet.push(foot_cursor - 1);
        }
        Self {
            characters,
            line_spans,
            rhyme_feet,
        }
    }

    fn content_len(&self) -> usize {
        self.characters.len()
    }
}

fn clamp_mode(mode: PracticeMode, line_count: usize) -> PracticeMode {
    match mode {
        PracticeMode::Masked(stage) => {
            PracticeMode::Masked(MaskStage::new(stage.masked_lines().min(line_count)))
        }
        other => other,
    }
}

fn hidden_indices(mode: PracticeMode, layout: &ContentLayout) -> Vec<usize> {
    match mode {
        PracticeMode::Cloze(options) => cloze_blanks(options, layout),
        PracticeMode::FirstChar => {
            let visible = layout
                .line_spans
                .iter()
                .map(|span| span.start)
                .collect::<Vec<_>>();
            (0..layout.content_len())
                .filter(|index| !visible.contains(index))
                .collect()
        }
        PracticeMode::Masked(stage) => layout
            .line_spans
            .iter()
            .take(stage.masked_lines())
            .flat_map(|span| span.start..span.end)
            .collect(),
    }
}

/// 按优先级挑出空位：韵脚在最前，其次实词，虚词垫底。
///
/// 韵脚优先是因为它同时承担押韵与句末停顿，忘掉它等于忘掉整句的落点；实词优先于虚词
/// 是因为挖掉「之乎者也」只考手速不考记忆。同一优先级内按种子派生的键排序，于是选位
/// 既分散又完全可重放：同一种子在任何机器上给出同一组空位。
fn cloze_blanks(options: ClozeOptions, layout: &ContentLayout) -> Vec<usize> {
    let content_len = layout.content_len();
    let wanted = blank_count(options.ratio(), content_len);
    if wanted == 0 {
        return Vec::new();
    }
    let mut candidates = (0..content_len).collect::<Vec<_>>();
    candidates.sort_by_key(|index| {
        (
            blank_tier(*index, layout),
            blank_key(options.seed(), *index),
            *index,
        )
    });
    candidates.truncate(wanted);
    candidates.sort_unstable();
    candidates
}

fn blank_count(ratio: f32, content_len: usize) -> usize {
    let wanted = (content_len as f32 * ratio).round();
    if wanted <= 0.0 {
        0
    } else {
        (wanted as usize).min(content_len)
    }
}

fn blank_tier(index: usize, layout: &ContentLayout) -> u8 {
    if layout.rhyme_feet.contains(&index) {
        0
    } else if FUNCTION_WORDS.contains(&layout.characters[index]) {
        2
    } else {
        1
    }
}

/// 由种子和位置派生排序键。
///
/// 用 SplitMix64 而不是 `rand`：挖空只需要「同种子同结果」这一条性质，而自带实现
/// 免掉一个依赖，也免掉「换个 rand 版本导致同一种子挖出不同空位」这种静默漂移。
/// 键只依赖 (种子, 下标)，不依赖遍历顺序，所以排序结果与候选集合的枚举方式无关。
fn blank_key(seed: u64, index: usize) -> u64 {
    splitmix64(seed ^ splitmix64(index as u64))
}

const fn splitmix64(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn render_prompt(body: &str, hidden: &[usize], content_len: usize) -> String {
    let mut masked = vec![false; content_len];
    for index in hidden {
        if let Some(slot) = masked.get_mut(*index) {
            *slot = true;
        }
    }
    let mut prompt = String::with_capacity(body.len());
    let mut cursor = 0;
    for character in body.chars() {
        if is_content(character) {
            if masked.get(cursor).copied().unwrap_or_default() {
                prompt.push(MASK_CHARACTER);
            } else {
                prompt.push(character);
            }
            cursor += 1;
        } else {
            prompt.push(character);
        }
    }
    prompt
}

fn is_content(character: char) -> bool {
    !character.is_whitespace() && !is_punctuation(character)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use yunjian_core::{CorpusConfig, SCHEMA_VERSION};

    const JING_YE_SI: &str = "床前明月光，疑是地上霜。举头望明月，低头思故乡。";

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        dir: PathBuf,
        handle: CorpusHandle,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-recite-modes-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建练习 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开练习 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path) {
        let connection = Connection::open(path).expect("创建练习 fixture 数据库");
        connection
            .execute_batch(
                "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);
                 CREATE TABLE variant_map(
                     src_char TEXT PRIMARY KEY NOT NULL,
                     dst_char TEXT NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE corpus_meta(
                     singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
                     schema_version INTEGER NOT NULL,
                     corpus_version TEXT NOT NULL,
                     built_at TEXT NOT NULL,
                     poem_count INTEGER NOT NULL,
                     index_detail_mode TEXT NOT NULL,
                     derived_indexes TEXT NOT NULL,
                     shipped_scope TEXT NOT NULL,
                     integrity_check TEXT NOT NULL
                 );",
            )
            .expect("创建练习 fixture schema");
        connection
            .execute(
                "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                params!["國", "国"],
            )
            .expect("写 variant_map");
        connection
            .execute(
                "INSERT INTO corpus_meta VALUES
                 (1, ?1, 'fixture-v1', '2026-08-12T00:00:00Z', 0, 'full',
                  'first_launch', '10k', 'ok')",
                [SCHEMA_VERSION],
            )
            .expect("写 corpus_meta");
        connection.close().expect("关闭练习 fixture 数据库");
    }

    fn session(handle: &CorpusHandle, body: &str, mode: PracticeMode) -> PracticeSession {
        PracticeSession::start(handle, body, mode).expect("建立练习会话")
    }

    fn cloze(handle: &CorpusHandle, body: &str, ratio: f32, seed: u64) -> PracticeSession {
        session(
            handle,
            body,
            PracticeMode::Cloze(ClozeOptions::new(ratio, seed)),
        )
    }

    fn hidden_characters(body: &str, hidden: &[usize]) -> Vec<char> {
        let characters = content_chars(body).collect::<Vec<_>>();
        hidden.iter().map(|index| characters[*index]).collect()
    }

    #[test]
    fn the_same_seed_reproduces_identical_blanks_across_runs() {
        let fixture = fixture();
        let first = cloze(&fixture.handle, JING_YE_SI, 0.3, 42);
        let second = cloze(&fixture.handle, JING_YE_SI, 0.3, 42);
        let third = cloze(&fixture.handle, JING_YE_SI, 0.3, 42);

        assert_eq!(first.hidden_indices(), second.hidden_indices());
        assert_eq!(second.hidden_indices(), third.hidden_indices());
        assert_eq!(first.prompt(), third.prompt());
        assert!(
            (1..=8).any(|seed| {
                cloze(&fixture.handle, JING_YE_SI, 0.3, seed).hidden_indices()
                    != first.hidden_indices()
            }),
            "种子必须真的参与选位，否则可重放是因为根本没随机"
        );
    }

    #[test]
    fn a_twenty_character_poem_at_ratio_zero_point_three_blanks_exactly_six() {
        let fixture = fixture();
        assert_eq!(content_chars(JING_YE_SI).count(), 20);

        let blanked = cloze(&fixture.handle, JING_YE_SI, 0.3, 7);
        assert_eq!(blanked.hidden_indices().len(), 6);
        assert_eq!(
            blanked
                .prompt()
                .chars()
                .filter(|character| *character == MASK_CHARACTER)
                .count(),
            6
        );
    }

    #[test]
    fn rhyme_feet_are_blanked_before_anything_else() {
        let fixture = fixture();
        let feet = [9_usize, 19];
        assert_eq!(hidden_characters(JING_YE_SI, &feet), ['霜', '乡']);

        for seed in 0..16 {
            let blanked = cloze(&fixture.handle, JING_YE_SI, 0.1, seed);
            assert_eq!(blanked.hidden_indices().len(), 2);
            assert_eq!(blanked.hidden_indices(), feet, "种子 {seed} 未优先挖韵脚");
        }
    }

    #[test]
    fn content_words_are_blanked_before_function_words() {
        let fixture = fixture();
        let copula = 6_usize;
        assert_eq!(hidden_characters(JING_YE_SI, &[copula]), ['是']);

        for seed in 0..16 {
            let most = cloze(&fixture.handle, JING_YE_SI, 0.9, seed);
            assert_eq!(most.hidden_indices().len(), 18);
            assert!(
                !most.hidden_indices().contains(&copula),
                "种子 {seed}：18 个空位时虚词「是」不该入选"
            );
        }
        let all = cloze(&fixture.handle, JING_YE_SI, 1.0, 3);
        assert!(
            all.hidden_indices().contains(&copula),
            "比例为 1.0 时虚词也要挖，否则挖空数对不上"
        );
    }

    #[test]
    fn first_char_mode_keeps_exactly_one_character_per_presentation_line() {
        let fixture = fixture();
        let hinted = session(&fixture.handle, JING_YE_SI, PracticeMode::FirstChar);

        assert_eq!(hinted.line_count(), 4);
        assert_eq!(hinted.hidden_indices().len(), 16);
        let visible = (0..20)
            .filter(|index| !hinted.hidden_indices().contains(index))
            .collect::<Vec<_>>();
        assert_eq!(visible, [0, 5, 10, 15]);
        assert_eq!(
            hidden_characters(JING_YE_SI, &visible),
            ['床', '疑', '举', '低']
        );
        assert_eq!(
            hinted.prompt(),
            "床＿＿＿＿，疑＿＿＿＿。举＿＿＿＿，低＿＿＿＿。"
        );
    }

    #[test]
    fn masked_mode_walks_from_full_text_to_fully_masked() {
        let fixture = fixture();
        let stages = (0..=4)
            .map(|masked| {
                session(
                    &fixture.handle,
                    JING_YE_SI,
                    PracticeMode::Masked(MaskStage::new(masked)),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(stages[0].prompt(), JING_YE_SI);
        assert!(stages[0].hidden_indices().is_empty());
        assert_eq!(
            stages[1].prompt(),
            "＿＿＿＿＿，疑是地上霜。举头望明月，低头思故乡。"
        );
        assert_eq!(stages[4].hidden_indices().len(), 20);
        assert_eq!(
            stages[4].prompt(),
            "＿＿＿＿＿，＿＿＿＿＿。＿＿＿＿＿，＿＿＿＿＿。"
        );
        for window in stages.windows(2) {
            assert!(window[0].hidden_indices().len() < window[1].hidden_indices().len());
        }
        assert_eq!(stages[0].stage_count(), 5);
    }

    #[test]
    fn an_over_long_mask_stage_converges_to_this_poems_last_stage() {
        let fixture = fixture();
        let all = session(
            &fixture.handle,
            JING_YE_SI,
            PracticeMode::Masked(MaskStage::ALL),
        );

        assert_eq!(all.mode(), PracticeMode::Masked(MaskStage::new(4)));
        assert_eq!(all.hidden_indices().len(), 20);
        assert_eq!(MaskStage::new(4).advanced(), MaskStage::new(5));
        assert_eq!(MaskStage::ALL.advanced(), MaskStage::ALL);
    }

    #[test]
    fn a_perfect_typed_answer_scores_one_through_the_shared_kernel() {
        let fixture = fixture();
        let answer = TypedAttempt::new(&fixture.handle, JING_YE_SI).expect("构造键入尝试");

        for mode in PracticeMode::ALL {
            let score = session(&fixture.handle, JING_YE_SI, mode).score(&answer);
            assert_eq!(score.completeness, 1.0, "{}", mode.as_str());
            assert_eq!(score.accuracy_strict, 1.0, "{}", mode.as_str());
            assert_eq!(score.accuracy_lenient, 1.0, "{}", mode.as_str());
            assert!(!score.is_rejected, "{}", mode.as_str());
            assert_eq!(score.ops_summary.normal_count, 20, "{}", mode.as_str());
        }
    }

    #[test]
    fn every_mode_scores_through_the_one_shared_kernel_not_a_second_path() {
        let fixture = fixture();
        let reference = Poem::new(&fixture.handle, JING_YE_SI).expect("构造参考诗文");
        let attempts = [
            JING_YE_SI,
            "床前明月光，疑是地上霜。低头思故乡。",
            "床前明月光，疑是地上霜。举头望明月，低头思故鄉。",
            "白日依山尽，黄河入海流。",
        ];

        for mode in PracticeMode::ALL {
            let practice = session(&fixture.handle, JING_YE_SI, mode);
            for text in attempts {
                let attempt = TypedAttempt::new(&fixture.handle, text).expect("构造键入尝试");
                assert_eq!(
                    practice.score(&attempt),
                    score_typed(&reference, &attempt),
                    "{} 与共用内核不一致：{text}",
                    mode.as_str()
                );
            }
        }
    }

    #[test]
    fn all_three_modes_return_the_typed_score_type_of_the_voice_capable_kernel() {
        fn requires_typed_score(score: TypedScore) -> TypedScore {
            score
        }

        let fixture = fixture();
        let answer = TypedAttempt::new(&fixture.handle, JING_YE_SI).expect("构造键入尝试");
        let scores = PracticeMode::ALL.map(|mode| {
            requires_typed_score(session(&fixture.handle, JING_YE_SI, mode).score(&answer))
        });

        assert_eq!(scores[0], scores[1]);
        assert_eq!(scores[1], scores[2]);

        let library = library_source();
        for forbidden in ["AlignOp", "Alignment", "align_normalized", "OpsSummary {"] {
            assert!(
                !library.contains(forbidden),
                "modes.rs 不得自建对齐或评分路径：{forbidden}"
            );
        }
        assert!(library.contains("score_typed(&self.reference, attempt)"));
    }

    /// 只取测试模块之前的源码。
    ///
    /// 守卫用例自己会把待禁字面量写在断言里，`include_str!` 又会把测试模块一并读进来，
    /// 于是不切掉测试部分的话，守卫永远命中自己写的针，既报假红也不再守任何东西。
    fn library_source() -> &'static str {
        include_str!("modes.rs")
            .split_once(concat!("#[cfg(", "test)]"))
            .expect("定位测试模块边界")
            .0
    }

    #[test]
    fn no_hidden_voice_dependency() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["yunjian-voice", "sherpa", "rodio", "cpal"] {
            assert!(
                !manifest.contains(forbidden),
                "背诵 crate 不得依赖语音栈：{forbidden}"
            );
        }
        assert!(!manifest.contains("[features]"));

        let library = library_source();
        for forbidden in [
            "feature = \"voice\"",
            "feature = \"capture\"",
            "yunjian_voice",
            "cfg(feature",
        ] {
            assert!(
                !library.contains(forbidden),
                "三种形态不得带特性开关或语音依赖：{forbidden}"
            );
        }
    }

    #[test]
    fn rendering_and_scoring_indices_stay_separate_across_variant_rewriting() {
        let fixture = fixture();
        let body = "國破山河在，城春草木深。";
        let hinted = session(&fixture.handle, body, PracticeMode::FirstChar);

        assert_eq!(hinted.prompt(), "國＿＿＿＿，城＿＿＿＿。");
        assert_eq!(hinted.reference().as_str(), "国破山河在城春草木深");

        for text in [body, "国破山河在，城春草木深。"] {
            let attempt = TypedAttempt::new(&fixture.handle, text).expect("构造键入尝试");
            assert_eq!(hinted.score(&attempt).accuracy_strict, 1.0, "{text}");
        }
    }

    #[test]
    fn the_prompt_preserves_punctuation_and_the_content_character_count() {
        let fixture = fixture();
        for body in [
            JING_YE_SI,
            "国破山河在，城春草木深。\n感时花溅泪，恨别鸟惊心。",
            "少年不识愁滋味，爱上层楼。",
        ] {
            let expected = content_chars(body).count();
            for mode in PracticeMode::ALL {
                let prompt = session(&fixture.handle, body, mode).prompt().to_owned();
                assert_eq!(
                    prompt.chars().filter(|c| is_content(*c)).count(),
                    expected,
                    "{body}"
                );
                assert_eq!(
                    prompt
                        .chars()
                        .filter(|c| !is_content(*c))
                        .collect::<String>(),
                    body.chars().filter(|c| !is_content(*c)).collect::<String>(),
                    "{body}"
                );
            }
        }
    }

    #[test]
    fn both_splitters_cover_every_content_character_exactly_once() {
        for body in [
            JING_YE_SI,
            "国破山河在，城春草木深。\n感时花溅泪，恨别鸟惊心。",
            "花间一壶酒",
            "少年不识愁滋味，爱上层楼。爱上层楼，为赋新词强说愁。",
        ] {
            let total = content_chars(body).count();
            assert_eq!(
                split_metrical_lines(body)
                    .map(|line| content_chars(line).count())
                    .sum::<usize>(),
                total,
                "{body}"
            );
            assert_eq!(
                split_rhyme_feet(body)
                    .map(|foot| content_chars(foot).count())
                    .sum::<usize>(),
                total,
                "{body}"
            );
        }
    }

    #[test]
    fn out_of_range_and_non_finite_ratios_are_regularized() {
        let fixture = fixture();
        assert_eq!(ClozeOptions::new(-1.0, 0).ratio(), 0.0);
        assert_eq!(ClozeOptions::new(2.0, 0).ratio(), 1.0);
        assert_eq!(ClozeOptions::new(f32::NAN, 0).ratio(), 0.0);
        assert_eq!(ClozeOptions::with_seed(9).seed(), 9);
        assert_eq!(
            ClozeOptions::with_seed(9).ratio(),
            ClozeOptions::DEFAULT_RATIO
        );

        assert!(
            cloze(&fixture.handle, JING_YE_SI, 0.0, 1)
                .hidden_indices()
                .is_empty()
        );
        assert_eq!(
            cloze(&fixture.handle, JING_YE_SI, 1.0, 1)
                .hidden_indices()
                .len(),
            20
        );
    }

    #[test]
    fn an_empty_body_yields_an_answerable_session_rather_than_a_panic() {
        let fixture = fixture();
        let empty = TypedAttempt::new(&fixture.handle, "").expect("构造空尝试");

        for mode in PracticeMode::ALL {
            let practice = session(&fixture.handle, "　\n，。", mode);
            assert_eq!(practice.line_count(), 0);
            assert_eq!(practice.stage_count(), 1);
            assert!(practice.hidden_indices().is_empty());
            assert_eq!(practice.score(&empty).completeness, 1.0);
        }
    }
}
