//! 注音层：把一首诗整篇解析成逐字读音。
//!
//! 这一层要回答的是一个诚实性问题，不是一个查表问题。同一个字的读音有四种强度完全不同的
//! 处境，界面必须把它们说成四种不同的话：破读词表给出的有据读音、通用候选只有一个时的
//! 普通话读音、多个候选而没有破读证据时的存疑、以及根本没有读音数据。把后三种任何一种
//! 说成第一种，就是拿一个猜测冒充一条考据。
//!
//! 因此四档不是一个带标签的字符串，而是 [`Reading`] 的四个变体，且「有据」那一档携带的
//! 依据强度是 [`AttestedConfidence`]——**它刻意只有两个变体**，于是词表里那种「只登记处置、
//! 明确表示不覆写」的行在类型上就没有地方可去，不可能被写成一条有据读音。这比在调用点
//! 加一次判断可靠：判断可以被下一个人删掉，变体不存在则改不动。
//!
//! **本模块不收语料库句柄。** [`annotate_poem`] 的入参只有词表与正文，于是「解析注音会不会
//! 去查库」不需要靠计数器观测——它拿不到可以查的东西。这同时也是韵书那条禁令的落地方式：
//! 韵书记录的是声部与韵部，推不出现代拼音，而本模块连韵书表都摸不到，所以「从韵部反推
//! 拼音」在这里不是被禁止，是做不到。
//!
//! 覆盖边界要如实：破读词表当前只覆盖朗读名册那 22 首，表外的字一律走通用候选那三档，
//! 不得宣称破读覆盖完整。

use crate::lexicon::{Confidence, Poyin};
use pinyin::{Pinyin, ToPinyinMulti};
use serde::Serialize;

/// 内容字的码位区间。
///
/// 只有内容字才有读音层。标点、空白与西文都不进四档判定——给「，」标一个「暂无注音」
/// 是噪声，而不是诚实。全角标点在 U+3000 区块，不落在下列任何一段里。
const IDEOGRAPH_RANGES: &[(char, char)] = &[
    ('\u{3400}', '\u{4dbf}'),
    ('\u{4e00}', '\u{9fff}'),
    ('\u{f900}', '\u{faff}'),
    ('\u{20000}', '\u{2a6df}'),
];

/// 该字是否有读音层。
#[must_use]
pub fn is_content_character(character: char) -> bool {
    IDEOGRAPH_RANGES
        .iter()
        .any(|&(low, high)| character >= low && character <= high)
}

/// 有据破读的依据强度。
///
/// **只有两个变体，这是本模块最重要的一个约束。** 破读词表里还有第三种 `confidence`，
/// 它表示的恰恰是「不覆写，只登记处置」；那一档没有对应变体，因此
/// [`Reading::Attested`] 无法用它构造出来。要把它当成有据读音，必须先给这个枚举加一个
/// 变体——那是一次显式的、会被评审看见的改动，而不是一次顺手的疏忽。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestedConfidence {
    /// 同部字的韵类把读音限定住了。
    RhymeAttested,
    /// 韵书调类分工：同字两读分属不同声部，且义项与声部对应。
    ToneSplit,
}

impl AttestedConfidence {
    /// 稳定标识。界面文案与 JSON 输出都用它，不用 `Debug`。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RhymeAttested => "rhyme_attested",
            Self::ToneSplit => "tone_split",
        }
    }
}

/// 一档 `confidence` 不表示覆写，因此当不了有据读音。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAttested;

impl TryFrom<Confidence> for AttestedConfidence {
    type Error = NotAttested;

    fn try_from(value: Confidence) -> Result<Self, Self::Error> {
        match value {
            Confidence::RhymeAttested => Ok(Self::RhymeAttested),
            Confidence::ToneSplit => Ok(Self::ToneSplit),
            Confidence::EngineDefault => Err(NotAttested),
        }
    }
}

/// 一个内容字的读音处境。四档互斥且穷尽。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reading {
    /// 第一档：当前句命中破读覆写。带调拼音与依据都来自词表。
    Attested {
        /// 带调拼音。
        pinyin: String,
        /// 依据强度。
        confidence: AttestedConfidence,
        /// 逐条可核对的依据原文。
        evidence: String,
    },
    /// 第二档：没有破读覆写，且通用候选只有一个。
    ///
    /// 这一档**不是**古典语境裁决，只是「这个字在现代普通话里没有异读」。界面必须说成
    /// 「通用拼音」。
    Generic {
        /// 带调拼音。
        pinyin: String,
    },
    /// 第三档：多个通用候选且没有破读证据。并列展示，不替用户选。
    Uncertain {
        /// 全部候选，按上游给出的顺序，已去重。
        candidates: Vec<String>,
    },
    /// 第四档：没有读音数据。不造占位读音。
    Absent,
}

impl Reading {
    /// 稳定标识，供界面按档位分流与覆盖统计使用。
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Attested { .. } => "attested",
            Self::Generic { .. } => "generic",
            Self::Uncertain { .. } => "uncertain",
            Self::Absent => "absent",
        }
    }
}

/// 正文里的一格。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// 该格的字。
    pub character: char,
    /// 读音处境。`None` 表示这一格不是内容字，因此没有读音层。
    ///
    /// 与 `Some(Reading::Absent)` 不是一回事：后者是「有读音位但查不到数据」，
    /// 前者是「这里本来就不该有读音」。两者混同会让标点显示「暂无注音」。
    pub reading: Option<Reading>,
}

/// 一行的注音。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnnotatedLine {
    /// 行号，按**去掉空行之后**的位置计数，与既有平仄标注同一套口径。
    pub line_index: usize,
    /// 该行原文。
    pub text: String,
    /// 逐字格，含非内容字，以便界面逐格对齐。
    pub cells: Vec<Cell>,
}

/// 四档的绝对数量。
///
/// 设计要求公布四档覆盖时给绝对数量和分母，所以这里存计数而不是比例：比例会把
/// 「3 / 4」和「3000 / 4000」说成同一件事。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// 有据破读字数。
    pub attested: usize,
    /// 单候选通用拼音字数。
    pub generic: usize,
    /// 多候选存疑字数。
    pub uncertain: usize,
    /// 无读音数据字数。
    pub absent: usize,
}

impl Coverage {
    /// 分母：内容字总数。
    #[must_use]
    pub const fn total(self) -> usize {
        self.attested + self.generic + self.uncertain + self.absent
    }
}

/// 整首的注音结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Annotation {
    /// 逐行注音。
    pub lines: Vec<AnnotatedLine>,
    /// 四档绝对数量。
    pub coverage: Coverage,
}

/// 解析一个字在给定句子里的读音。
///
/// 四档顺序即设计定下的优先级，且**第一档只可能来自词表的覆写行**：
/// [`Poyin::reading`] 自身就不返回登记处置行，而即便它返回了，
/// [`AttestedConfidence::try_from`] 也会拒绝——两道独立的关，任一道单独成立就够。
#[must_use]
pub fn resolve(poyin: &Poyin, character: char, line: &str) -> Option<Reading> {
    if !is_content_character(character) {
        return None;
    }

    if let Some(row) = poyin.reading(character, line) {
        // 覆写行必须带拼音，这一点由词表解析强制；此处仍按 `Option` 处理而不 `unwrap`，
        // 因为「数据坏了就崩掉」换不来任何正确性，而降级到候选那几档是诚实的。
        if let (Some(reading), Ok(confidence)) = (
            row.pinyin.as_deref(),
            AttestedConfidence::try_from(row.confidence),
        ) {
            return Some(Reading::Attested {
                pinyin: reading.to_owned(),
                confidence,
                evidence: row.evidence.clone(),
            });
        }
    }

    Some(generic_reading(character))
}

/// 不查词表时的三档：单候选、多候选、无数据。
fn generic_reading(character: char) -> Reading {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(multi) = character.to_pinyin_multi() {
        for reading in multi {
            let text = Pinyin::with_tone(reading).to_owned();
            if !candidates.contains(&text) {
                candidates.push(text);
            }
        }
    }

    // 去重之后再判档：两个候选若渲染成同一串字符，摆给用户看就是一个读音，
    // 此时说「存疑」是把一个不存在的选择塞给他。
    match candidates.len() {
        0 => Reading::Absent,
        1 => Reading::Generic {
            pinyin: candidates.remove(0),
        },
        _ => Reading::Uncertain { candidates },
    }
}

/// 整首一次性解析。
///
/// 入参只有词表与正文：没有语料库句柄、没有网络客户端、没有模型目录。这不是省事，
/// 而是让「切换开关会不会触发逐字查询」这件事在类型上就没有发生的余地。
#[must_use]
pub fn annotate_poem(poyin: &Poyin, body: &str) -> Annotation {
    let mut lines = Vec::new();
    let mut coverage = Coverage::default();

    for (line_index, text) in body
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let mut cells = Vec::new();
        for character in text.chars() {
            let reading = resolve(poyin, character, text);
            match reading {
                Some(Reading::Attested { .. }) => coverage.attested += 1,
                Some(Reading::Generic { .. }) => coverage.generic += 1,
                Some(Reading::Uncertain { .. }) => coverage.uncertain += 1,
                Some(Reading::Absent) => coverage.absent += 1,
                None => {}
            }
            cells.push(Cell { character, reading });
        }
        lines.push(AnnotatedLine {
            line_index,
            text: text.to_owned(),
            cells,
        });
    }

    Annotation { lines, coverage }
}

#[cfg(test)]
mod tests;
