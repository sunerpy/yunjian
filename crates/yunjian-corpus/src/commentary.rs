//! 历代集评（前现代诗话/辑评）入库。
//!
//! 这条通道存在的理由是法律性的，不是功能性的：现存开源诗词数据集里凡带**现代**
//! 注释、译文、赏析的，授权链一律立不住（无 LICENSE、NC、仅限学术，或仓库级
//! LICENSE 盖不住它转录的内容）。而**前现代**的诗话辑评本身已过保护期——宋人评
//! 唐诗是公有领域，与现代赏析在法律上是两个类别。所以「MIT 原文 + 逐条出处的公有
//! 领域集评 + 明确标注的 AI 赏析」是唯一能凑出完整词典的合法组合，本模块负责其中
//! 的第二项。
//!
//! 因此本模块的形状由两条规则决定：
//!
//! 1. **出处是必填的，而且必须可定位。** 只有非空的 `work` 会让「《某某诗话》云」
//!    这种无法复核的引用溜过去，而可审计性正是这条管道存在的理由。所以
//!    [`Citation::source_note`] 必须同时含卷次/章节定位符与所据版本，
//!    [`Citation::work_completed_by`] 必须是前 1912 的成书上界。
//! 2. **绝不批量导入任何数据集的 `Comment` 类字段。** 唯一被核实含前现代诗话的
//!    数据集自身无 LICENSE 且声明「仅限于交流学习」，所以它的文本不可整体复制；
//!    它只能用作定位原始公有领域出处的**指针**，然后直接引用那个原始出处。本模块
//!    读的是 `corpus/commentary/` 下逐条转录并逐条注明出处的种子文件，不读任何
//!    上游数据集。

use crate::model::{CanonicalRecord, Dynasty, SourceLocatorKind};
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use yunjian_core::{Error, Result};

/// 清朝终于 1912 年。晚于此的著作一律不是前现代作品，与其自称的朝代无关。
pub const PRE_MODERN_YEAR_EXCLUSIVE: u16 = 1912;

/// 成书年下界。低于此值几乎一定是把行号、页码之类误填进了年份字段。
const EARLIEST_PLAUSIBLE_YEAR: u16 = 200;

/// 评语正文的最短长度（字符）。比这更短的不可能是一条可引用的评语。
const MIN_TEXT_CHARS: usize = 8;

/// 卷次/章节定位符的关键字。
const LOCATOR_KEYWORDS: [char; 12] = [
    '卷', '則', '则', '條', '条', '篇', '章', '編', '编', '部', '冊', '册',
];

/// 可充当序号的字符。`上/中/下/前/後/后/首/末` 是合法定位（如「卷上」），
/// 不能因为没有数字就判成无定位。
const ORDINAL_CHARS: [char; 27] = [
    '〇', '零', '一', '二', '三', '四', '五', '六', '七', '八', '九', '十', '百', '千', '两', '兩',
    '上', '中', '下', '前', '後', '后', '首', '末', '甲', '乙', '丙',
];

/// 「所据版本」必须出现的版本类关键字。
const EDITION_KEYWORDS: [&str; 8] = ["本", "版", "刊", "全書", "全书", "文庫", "文库", "錄文"];

/// 「所据版本」必须出现的据引标记。逼出「据…本」的写法，而不是只写一个书名。
const CITED_FROM_MARKERS: [&str; 2] = ["据", "據"];

/// 现代体裁标记。没有一部前现代诗话叫「…鉴赏辞典」。
const MODERN_GENRE_MARKERS: [&str; 24] = [
    "鉴赏辞典",
    "鑒賞辭典",
    "赏析",
    "賞析",
    "译注",
    "譯注",
    "今译",
    "今譯",
    "白话",
    "白話",
    "选注",
    "選注",
    "评传",
    "評傳",
    "文学史",
    "文學史",
    "辞典",
    "辭典",
    "论文",
    "論文",
    "学报",
    "學報",
    "出版社",
    "研究会",
];

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 一条集评所引的前现代出处。**每个字段都是必填的，且都要过校验。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    /// 所引著作，如 `鹤林玉露`。不带书名号，由展示层加。
    pub work: String,
    /// 所引著作的作者。
    pub author: String,
    /// 所引著作的朝代原串，必须能归一到十五个前 1912 规范键之一。
    pub dynasty: String,
    /// 成书年代的**保守上界**（成书不晚于该年）。不是精确成书年——多数诗话的
    /// 精确成书年无考，取作者卒年之类可查的上界既诚实又足够守住前 1912 判据。
    pub work_completed_by: u16,
    /// 卷次/章节定位符与所据版本，缺一不可。没有定位符的引用无法复核，
    /// 而可审计性正是这条管道存在的理由。
    pub source_note: String,
}

/// 集评所评诗篇的引用。三个字段共同定位到唯一一首诗。
///
/// 不直接写 `stable_id`：`stable_id` 由 [`crate::model::rebuild_corpus`] 在构建期
/// 铸造，手写等于把一个内容地址硬编码进人工数据，上游一次重排就全错。所以种子
/// 文件写人类可核对的三元组，构建期解析成 `poem_id`，解析失败硬失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoemRef {
    pub author: String,
    pub title: String,
    /// 首句。用于在同作者同题的多首之间消歧，也让人读 JSON 就能核对链对没链对。
    pub first_line: String,
}

/// `corpus/commentary/` 下逐条转录的种子条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentarySeed {
    /// 人工维护的稳定键，仅限小写 ASCII 字母、数字与连字符。
    pub id: String,
    pub poem: PoemRef,
    /// 评语正文，前现代文言原文。
    pub text: String,
    pub citation: Citation,
}

/// 一个种子文件的顶层结构。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CommentaryFile {
    pub entries: Vec<CommentarySeed>,
}

/// 过了校验的出处。`dynasty` 已归一为类型，原串保留在 `dynasty_raw`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCitation {
    pub work: String,
    pub author: String,
    pub dynasty: Dynasty,
    pub dynasty_raw: String,
    pub work_completed_by: u16,
    pub source_note: String,
}

/// 入库产物。`poem_id` 是解析出的 `stable_id`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentaryRecord {
    pub id: String,
    pub poem_id: String,
    pub poem: PoemRef,
    pub text: String,
    pub citation: AcceptedCitation,
}

/// 一条种子被拒的原因。每一种都必须能指名道姓，不允许「校验失败」这种回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// 条目 id 为空或含非法字符。
    InvalidEntryId,
    /// 同一批次里 id 重复。
    DuplicateEntryId,
    /// 引用的某个必填字段为空。缺 citation 的极端情形也走这里。
    MissingCitationField,
    /// 所评诗篇引用的某个必填字段为空。
    MissingPoemRefField,
    /// 评语正文为空、过短，或含 ASCII 字母、现代年份等非前现代文本特征。
    InvalidText,
    /// `citation.dynasty` 无法归一到十五个前 1912 规范键。`现代`/`当代`/`民国` 全在此被拒。
    DynastyNotPreModern,
    /// `citation.work_completed_by` 不早于 1912，即所引著作不是前现代作品。
    WorkNotPreModern,
    /// 所引著作名带现代体裁标记（鉴赏辞典、赏析、译注……）。
    ModernGenreMarker,
    /// `source_note` 没有卷次/章节定位符，引用无法复核。
    SourceNoteMissingLocator,
    /// `source_note` 没有说明所据版本。
    SourceNoteMissingEdition,
    /// 语料里找不到该诗篇。
    PoemUnresolved,
    /// 该三元组匹配到多首诗，不允许猜。
    PoemAmbiguous,
}

impl RejectionReason {
    /// 稳定的机器可读键，同时用于报告与测试断言。
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::InvalidEntryId => "invalid_entry_id",
            Self::DuplicateEntryId => "duplicate_entry_id",
            Self::MissingCitationField => "missing_citation_field",
            Self::MissingPoemRefField => "missing_poem_ref_field",
            Self::InvalidText => "invalid_text",
            Self::DynastyNotPreModern => "dynasty_not_pre_modern",
            Self::WorkNotPreModern => "work_not_pre_modern",
            Self::ModernGenreMarker => "modern_genre_marker",
            Self::SourceNoteMissingLocator => "source_note_missing_locator",
            Self::SourceNoteMissingEdition => "source_note_missing_edition",
            Self::PoemUnresolved => "poem_unresolved",
            Self::PoemAmbiguous => "poem_ambiguous",
        }
    }
}

/// 一条被拒种子的完整交代：是谁、为什么、以及可核对的细节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    pub entry_id: String,
    pub reason: RejectionReason,
    pub detail: String,
}

/// 一次集评入库的全部产出。被拒条目**记录而不静默丢弃**。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentaryOutcome {
    pub records: Vec<CommentaryRecord>,
    pub rejections: Vec<Rejection>,
}

impl CommentaryOutcome {
    /// 构建期门禁：种子集是本仓库自己维护的，出现任何被拒条目都是缺陷而非上游噪声，
    /// 所以这里硬失败并把每一条原因都报出来。
    pub fn require_all_accepted(self) -> Result<Vec<CommentaryRecord>> {
        if self.rejections.is_empty() {
            return Ok(self.records);
        }
        let detail = self
            .rejections
            .iter()
            .map(|rejection| {
                format!(
                    "{}（{}）：{}",
                    rejection.entry_id,
                    rejection.reason.as_key(),
                    rejection.detail
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        Err(corpus_error(format!(
            "集评种子集有 {} 条被拒：{detail}",
            self.rejections.len()
        )))
    }
}

/// 校验一条种子的结构与出处，不涉及诗篇解析。
///
/// 拆成独立函数是为了让「出处必填且可定位」这条判据在**没有语料**的环境下也能
/// 全量跑——CI 里没有 455 MB 的上游检出，但版权墙必须每次提交都受检。
pub fn validate_seed(seed: &CommentarySeed) -> std::result::Result<AcceptedCitation, Rejection> {
    let reject = |reason: RejectionReason, detail: String| Rejection {
        entry_id: seed.id.clone(),
        reason,
        detail,
    };

    if !is_valid_entry_id(&seed.id) {
        return Err(reject(
            RejectionReason::InvalidEntryId,
            format!("id 必须是非空的小写 ASCII 字母/数字/连字符：{:?}", seed.id),
        ));
    }

    for (field, value) in [
        ("poem.author", &seed.poem.author),
        ("poem.title", &seed.poem.title),
        ("poem.first_line", &seed.poem.first_line),
    ] {
        if value.trim().is_empty() {
            return Err(reject(
                RejectionReason::MissingPoemRefField,
                format!("{field} 不能为空"),
            ));
        }
    }

    validate_text(&seed.text).map_err(|detail| reject(RejectionReason::InvalidText, detail))?;

    let citation = &seed.citation;
    for (field, value) in [
        ("citation.work", &citation.work),
        ("citation.author", &citation.author),
        ("citation.dynasty", &citation.dynasty),
        ("citation.source_note", &citation.source_note),
    ] {
        if value.trim().is_empty() {
            return Err(reject(
                RejectionReason::MissingCitationField,
                format!("{field} 不能为空——只有非空的 work 会让无法定位的引用溜过去"),
            ));
        }
    }

    if let Some(marker) = MODERN_GENRE_MARKERS
        .iter()
        .find(|marker| citation.work.contains(**marker))
    {
        return Err(reject(
            RejectionReason::ModernGenreMarker,
            format!(
                "所引著作《{}》含现代体裁标记「{marker}」，不是前现代诗话",
                citation.work
            ),
        ));
    }
    if let Some(year) = first_modern_year(&citation.work) {
        return Err(reject(
            RejectionReason::ModernGenreMarker,
            format!("所引著作《{}》名中含现代年份 {year}", citation.work),
        ));
    }

    let (dynasty, dynasty_raw) = Dynasty::canonicalize(&citation.dynasty).map_err(|error| {
        reject(
            RejectionReason::DynastyNotPreModern,
            format!("{error}；十五个规范键全部止于清（1912 年前），现代/当代/民国均不可用"),
        )
    })?;

    if citation.work_completed_by >= PRE_MODERN_YEAR_EXCLUSIVE {
        return Err(reject(
            RejectionReason::WorkNotPreModern,
            format!(
                "所引著作《{}》成书上界 {} 不早于 {PRE_MODERN_YEAR_EXCLUSIVE}，即便自称{}也不是前现代作品",
                citation.work, citation.work_completed_by, citation.dynasty
            ),
        ));
    }
    if citation.work_completed_by < EARLIEST_PLAUSIBLE_YEAR {
        return Err(reject(
            RejectionReason::WorkNotPreModern,
            format!(
                "所引著作《{}》成书上界 {} 早于 {EARLIEST_PLAUSIBLE_YEAR}，疑为误填",
                citation.work, citation.work_completed_by
            ),
        ));
    }

    if find_volume_locator(&citation.source_note).is_none() {
        return Err(reject(
            RejectionReason::SourceNoteMissingLocator,
            format!(
                "source_note 缺卷次/章节定位符（需形如「卷一」「卷上」「第十二则」）：{:?}",
                citation.source_note
            ),
        ));
    }
    if !has_edition_marker(&citation.source_note) {
        return Err(reject(
            RejectionReason::SourceNoteMissingEdition,
            format!(
                "source_note 未说明所据版本（需形如「据…本」）：{:?}",
                citation.source_note
            ),
        ));
    }

    Ok(AcceptedCitation {
        work: citation.work.trim().to_owned(),
        author: citation.author.trim().to_owned(),
        dynasty,
        dynasty_raw,
        work_completed_by: citation.work_completed_by,
        source_note: citation.source_note.trim().to_owned(),
    })
}

fn is_valid_entry_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_text(text: &str) -> std::result::Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("评语正文不能为空".to_owned());
    }
    let chars = trimmed.chars().count();
    if chars < MIN_TEXT_CHARS {
        return Err(format!(
            "评语正文只有 {chars} 字，短于 {MIN_TEXT_CHARS} 字下限"
        ));
    }
    if trimmed
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        return Err("评语正文含 ASCII 字母，前现代文言不会出现".to_owned());
    }
    if let Some(year) = first_modern_year(trimmed) {
        return Err(format!("评语正文含现代年份 {year}，疑为现代文字"));
    }
    Ok(())
}

/// 找出文本中第一个 1912..=2999 的四位年份。前现代文本不会自称现代年份。
///
/// 汉字数目必须紧跟 `年` 才算年份：《苕溪渔隐丛话》引的谜语「四八二八，飛泉仰流」
/// 恰好是四个连续汉字数目，按字形一刀切会把一条真的宋人诗话误判成现代文字。
/// 阿拉伯数字不要求 `年`，因为前现代文言里根本不会出现。
fn first_modern_year(text: &str) -> Option<u16> {
    let chars = text.chars().collect::<Vec<_>>();
    chars.windows(4).enumerate().find_map(|(index, window)| {
        let digits = window
            .iter()
            .map(|character| decimal_digit(*character))
            .collect::<Option<Vec<_>>>()?;
        if window.iter().any(|character| !character.is_ascii_digit())
            && chars.get(index + 4) != Some(&'年')
        {
            return None;
        }
        let year = digits.iter().fold(0_u16, |acc, digit| acc * 10 + *digit);
        (PRE_MODERN_YEAR_EXCLUSIVE..=2999)
            .contains(&year)
            .then_some(year)
    })
}

/// 同时认阿拉伯数字与汉字数目，因为现代文字两种写法都用。
const fn decimal_digit(character: char) -> Option<u16> {
    match character {
        '0'..='9' => Some(character as u16 - '0' as u16),
        '〇' | '零' => Some(0),
        '一' => Some(1),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

/// 定位符判据：出现卷/则/条/篇/章… 之一，且紧邻位置有序号字符。
///
/// 只查关键字会让「卷帙浩繁」这种描述蒙混过关，只查数字又会漏掉「卷上」。
/// 两者都要，且要求相邻，才是一个真的定位。
fn find_volume_locator(note: &str) -> Option<String> {
    let chars = note.chars().collect::<Vec<_>>();
    for (index, character) in chars.iter().enumerate() {
        if !LOCATOR_KEYWORDS.contains(character) {
            continue;
        }
        let lower = index.saturating_sub(3);
        let upper = (index + 4).min(chars.len());
        let neighborhood = &chars[lower..upper];
        if neighborhood
            .iter()
            .any(|nearby| ORDINAL_CHARS.contains(nearby) || nearby.is_ascii_digit())
        {
            return Some(neighborhood.iter().collect());
        }
    }
    None
}

/// 版本判据：既要有「据/據」这样的据引标记，也要有「本/版/刊…」这样的版本词。
fn has_edition_marker(note: &str) -> bool {
    CITED_FROM_MARKERS
        .iter()
        .any(|marker| note.contains(*marker))
        && EDITION_KEYWORDS
            .iter()
            .any(|keyword| note.contains(*keyword))
}

/// 诗篇引用与规范记录之间的匹配键归一器。
///
/// 两侧都要过同一套归一：`chinese-poetry` 同一仓库内全唐诗是繁体、宋词是简体，
/// 按仓库假定字形必然错；标点与空白在上游也不统一。
pub struct RefMatcher {
    to_simplified: OpenCC,
}

impl RefMatcher {
    pub fn new() -> Result<Self> {
        Ok(Self {
            to_simplified: OpenCC::from_config(BuiltinConfig::T2s)
                .map_err(|error| corpus_error(format!("初始化繁转简失败：{error}")))?,
        })
    }

    /// 归一：转简体、去标点与空白。
    pub fn normalize(&self, value: &str) -> String {
        self.to_simplified
            .convert(value)
            .chars()
            .filter(|character| {
                !character.is_whitespace()
                    && !character.is_ascii_punctuation()
                    && !is_cjk_punctuation(*character)
            })
            .collect()
    }
}

fn is_cjk_punctuation(character: char) -> bool {
    matches!(
        character,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '（'
            | '）'
            | '《'
            | '》'
            | '〈'
            | '〉'
            | '【'
            | '】'
            | '〔'
            | '〕'
            | '—'
            | '…'
            | '·'
            | '　'
    )
}

/// 供匹配用的诗篇索引。按（归一作者，归一题目）分组，组内再用首句消歧。
pub struct PoemIndex<'a> {
    matcher: RefMatcher,
    by_author_title: BTreeMap<(String, String), Vec<&'a CanonicalRecord>>,
}

impl<'a> PoemIndex<'a> {
    pub fn build(records: &'a [CanonicalRecord]) -> Result<Self> {
        let matcher = RefMatcher::new()?;
        let mut by_author_title = BTreeMap::<(String, String), Vec<&'a CanonicalRecord>>::new();
        for record in records {
            let author = matcher.normalize(&record.author);
            for title in title_keys(record) {
                let key = (author.clone(), matcher.normalize(&title));
                by_author_title.entry(key).or_default().push(record);
            }
        }
        Ok(Self {
            matcher,
            by_author_title,
        })
    }

    /// 解析成唯一的 `stable_id`。
    ///
    /// 三级判据，顺序固定，每一级都有上游实测依据：
    ///
    /// 1. **唯一命中**——正常情形。
    /// 2. **同一 `work_group`**——上游同一首诗重出于多个分片（如《登高》同时在
    ///    `全唐诗` 与 `水墨唐诗`），正文去标点后完全相同。此时任取一个是安全的，
    ///    因为 todo 14 的重出分组本就把它们视为一组；取 `stable_id` 排序最小者
    ///    以保证构建可复现。
    /// 3. **原生键优先**——繁简或异体导致 `work_group` 不同（如
    ///    `噫吁戲/噫吁嚱`），但其中恰有一条来自带上游 `id` 的资产
    ///    （`全唐诗/poet.*`）。原生键对上游重排免疫，是更可靠的锚。
    ///
    /// 三级都判不出唯一答案就硬失败，**绝不猜**：一条挂错诗的集评比没有集评更糟。
    pub fn resolve(
        &self,
        reference: &PoemRef,
    ) -> std::result::Result<String, (RejectionReason, String)> {
        let key = (
            self.matcher.normalize(&reference.author),
            self.matcher.normalize(&reference.title),
        );
        let Some(candidates) = self.by_author_title.get(&key) else {
            return Err((
                RejectionReason::PoemUnresolved,
                format!("语料中没有 {}《{}》", reference.author, reference.title),
            ));
        };
        let first_line = self.matcher.normalize(&reference.first_line);
        let matched = candidates
            .iter()
            .copied()
            .filter(|record| {
                self.matcher
                    .normalize(&record.body_lines.join(""))
                    .starts_with(&first_line)
            })
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Err((
                RejectionReason::PoemUnresolved,
                format!(
                    "{}《{}》有 {} 首同题，但没有一首以「{}」起句",
                    reference.author,
                    reference.title,
                    candidates.len(),
                    reference.first_line
                ),
            ));
        }

        let distinct = matched
            .iter()
            .map(|record| record.stable_id.as_str())
            .collect::<BTreeSet<_>>();
        if distinct.len() == 1 {
            return Ok(distinct.iter().copied().collect::<String>());
        }

        let work_groups = matched
            .iter()
            .map(|record| record.work_group.as_str())
            .collect::<BTreeSet<_>>();
        if work_groups.len() == 1 {
            return Ok(distinct
                .first()
                .map(|id| (*id).to_owned())
                .unwrap_or_default());
        }

        let native = matched
            .iter()
            .filter(|record| record.source_locator_kind == SourceLocatorKind::Native)
            .map(|record| record.stable_id.as_str())
            .collect::<BTreeSet<_>>();
        if native.len() == 1 {
            return Ok(native.iter().copied().collect::<String>());
        }

        Err((
            RejectionReason::PoemAmbiguous,
            format!(
                "{}《{}》以「{}」起句的有 {} 首且正文互不相同，原生键也无法定一：{}",
                reference.author,
                reference.title,
                reference.first_line,
                distinct.len(),
                distinct.into_iter().collect::<Vec<_>>().join("、")
            ),
        ))
    }
}

/// 记录可用于匹配的全部题目形态。
///
/// 上游的题目形态实测有四种，缺一种就会让一批引用解析不出来：
///
/// - 原样 `title` 与 `title_raw`；
/// - `词牌·题目` 的词牌部分——诗话引用词作时通常只称词牌（「东坡《水龙吟》」）；
/// - `词牌 其一` / `菩薩蠻 五` 的空格前缀——`全唐诗` 收词时用序号后缀区分同调。
fn title_keys(record: &CanonicalRecord) -> Vec<String> {
    let mut keys = vec![record.title.clone(), record.title_raw.clone()];
    if let Some(tune) = &record.ci_tune {
        keys.push(tune.clone());
    }
    for key in keys.clone() {
        if let Some((head, _)) = key.split_once(' ')
            && !head.is_empty()
        {
            keys.push(head.to_owned());
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

/// 只做结构与出处校验，不解析诗篇。CI 用这条路径全量校验种子集。
pub fn validate_all(seeds: &[CommentarySeed]) -> CommentaryOutcome {
    let mut outcome = CommentaryOutcome::default();
    let mut seen = BTreeSet::new();
    for seed in seeds {
        if !seen.insert(seed.id.clone()) {
            outcome.rejections.push(Rejection {
                entry_id: seed.id.clone(),
                reason: RejectionReason::DuplicateEntryId,
                detail: format!("id {} 在本批次中重复出现", seed.id),
            });
            continue;
        }
        match validate_seed(seed) {
            Ok(citation) => outcome.records.push(CommentaryRecord {
                id: seed.id.clone(),
                poem_id: String::new(),
                poem: seed.poem.clone(),
                text: seed.text.trim().to_owned(),
                citation,
            }),
            Err(rejection) => outcome.rejections.push(rejection),
        }
    }
    outcome
}

/// 完整入库：结构与出处校验后，把 `poem` 三元组解析成 `poem_id`。
pub fn ingest(seeds: &[CommentarySeed], records: &[CanonicalRecord]) -> Result<CommentaryOutcome> {
    let index = PoemIndex::build(records)?;
    let mut outcome = validate_all(seeds);
    let mut resolved = Vec::with_capacity(outcome.records.len());
    for mut record in outcome.records {
        match index.resolve(&record.poem) {
            Ok(poem_id) => {
                record.poem_id = poem_id;
                resolved.push(record);
            }
            Err((reason, detail)) => outcome.rejections.push(Rejection {
                entry_id: record.id,
                reason,
                detail,
            }),
        }
    }
    outcome.records = resolved;
    Ok(outcome)
}

/// `corpus/commentary/` 的目录约定。
pub const SOURCES_DIR: &str = "sources";
/// 聚合索引的文件名。它是生成物，由 `require_index_matches` 守住不漂移。
pub const INDEX_FILE: &str = "index.json";

/// 读取 `corpus/commentary/sources/*.json` 的全部种子，按文件名与文件内顺序排定。
pub fn load_seeds(commentary_dir: impl AsRef<Path>) -> Result<Vec<CommentarySeed>> {
    let dir = commentary_dir.as_ref().join(SOURCES_DIR);
    let mut files = std::fs::read_dir(&dir)
        .map_err(|error| corpus_error(format!("读取集评目录失败（{}）：{error}", dir.display())))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::result::Result<Vec<PathBuf>, _>>()?
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    files.sort();
    if files.is_empty() {
        return Err(corpus_error(format!(
            "集评目录 {} 下没有任何 .json 种子文件",
            dir.display()
        )));
    }
    let mut seeds = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path)?;
        let file: CommentaryFile = serde_json::from_str(&raw).map_err(|error| {
            corpus_error(format!("解析集评种子失败（{}）：{error}", path.display()))
        })?;
        seeds.extend(file.entries);
    }
    Ok(seeds)
}

/// 生成聚合索引的规范文本。排序与格式固定，因此可逐字节比对。
pub fn render_index(seeds: &[CommentarySeed]) -> Result<String> {
    let mut sorted = seeds.to_vec();
    sorted.sort_by(|left, right| left.id.cmp(&right.id));
    let mut rendered = serde_json::to_string_pretty(&sorted)
        .map_err(|error| corpus_error(format!("序列化集评索引失败：{error}")))?;
    rendered.push('\n');
    Ok(rendered)
}

/// 索引漂移门禁：`index.json` 必须与 `sources/` 的重新生成结果逐字节一致。
pub fn require_index_matches(commentary_dir: impl AsRef<Path>) -> Result<usize> {
    let dir = commentary_dir.as_ref();
    let seeds = load_seeds(dir)?;
    let expected = render_index(&seeds)?;
    let index_path = dir.join(INDEX_FILE);
    let actual = std::fs::read_to_string(&index_path).map_err(|error| {
        corpus_error(format!(
            "读取集评索引失败（{}）：{error}",
            index_path.display()
        ))
    })?;
    if actual != expected {
        return Err(corpus_error(format!(
            "{} 与 {}/ 不一致：索引是生成物，请重新生成后提交",
            index_path.display(),
            SOURCES_DIR
        )));
    }
    Ok(seeds.len())
}

#[cfg(test)]
mod tests;
