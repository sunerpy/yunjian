//! 破读词表、词谱句式表与朗读覆盖名册。
//!
//! **这三份 TSV 是内容工作，不是配置。** 它们的每一行都要能被追到一部公有领域著作的一个
//! 具体位置，理由是版权墙：不存在一部授权允许随包分发的现代权威读音辞书，所以古典读音
//! 只能自公有领域韵书推得，**绝不从现代词典转录**。这条规则如果只写在注释里就等于没写，
//! 因此依据列由 [`located_evidence`] 强制校验——空而存在的引用必须被拒绝。
//!
//! 校验标准与集评的 `source_note` 刻意相同（定位符 + 所据版本），因为它们是同一件事：
//! 一条引用要么能被第三方翻到那一页，要么不算引用。
//!
//! **本模块不带特性开关。** 解析、校验与覆盖闭合都是纯函数，于是没有模型、没有声卡、
//! 甚至没有语料库的机器上都能验证数据是对的。这是 `audio` 与 `models` 用过的同一分层。

use std::collections::{BTreeMap, BTreeSet};

/// 破读词表。
pub const POYIN_TSV: &str = include_str!("../../../data/poyin.tsv");
/// 词谱句式表。
pub const CITUNE_TSV: &str = include_str!("../../../data/citune_rhythm.tsv");
/// 朗读覆盖名册。
pub const ROSTER_TSV: &str = include_str!("../../../data/reading_roster.tsv");
/// 多音字索引，覆盖闭合检查的候选集来源。
pub const POLYPHONE_TSV: &str = include_str!("../../../data/polyphone_index.tsv");

/// 依据里可以充当定位符的关键字。
///
/// 与集评那套同源，并补了 `行` 与 `条`：本模块引的除了刻本还有随仓的契约文件与语料记录，
/// 「第 108 行起」「第 2 条记录」和「卷六」是同一类东西——都能让第三方翻到那个位置。
/// **`版` 刻意不在其中**：它说的是照谁的本子，不是在哪一页，属于所据版本那一半判据。
const LOCATOR_KEYWORDS: &[char] = &['卷', '部', '页', '则', '首', '编', '册', '号', '行', '条'];

/// 序数字。中文序数与阿拉伯数字都算。
const ORDINAL_CHARS: &[char] = &[
    '一', '二', '三', '四', '五', '六', '七', '八', '九', '十', '百', '千', '上', '中', '下',
];

/// 据引标记。没有它就说不清「照谁的本子」。
const CITED_FROM_MARKERS: &[&str] = &["据", "據"];

/// 版本词。
const EDITION_KEYWORDS: &[&str] = &["本", "版", "刊", "校", "转录", "轉錄"];

/// 一行切分依据来自哪里。
///
/// **这个枚举的存在就是为了不让界面越权宣称权威。** 三个值代表三种强度不同的依据，
/// 界面必须把它们说成不同的话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhythmSource {
    /// 按字数：五言二三、七言二二三。只看字数，不需要外部数据，因此最稳。
    CharCount,
    /// 按公有领域词谱的句式，带卷次页码。v1 尚无此类数据。
    CiTune,
    /// 按随包《全宋词》实测的众数句式，带样本量。**不是词谱权威。**
    CorpusModal,
    /// 按作品自己的标点切分。这是回落，界面必须如实说明。
    Punctuation,
}

impl RhythmSource {
    /// 稳定的机器可读标识。界面文案与 JSON 输出都用它，不用 `Debug`。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CharCount => "char_count",
            Self::CiTune => "citune",
            Self::CorpusModal => "corpus_modal",
            Self::Punctuation => "punctuation",
        }
    }

    /// 是否声称词谱权威。只有 [`Self::CiTune`] 是。
    #[must_use]
    pub const fn claims_citune_authority(self) -> bool {
        matches!(self, Self::CiTune)
    }

    fn parse(raw: &str) -> Result<Self, LexiconError> {
        match raw {
            "citune" => Ok(Self::CiTune),
            "corpus_modal" => Ok(Self::CorpusModal),
            other => Err(LexiconError::BadField {
                file: "citune_rhythm.tsv",
                line: 0,
                detail: format!(
                    "来源列只接受 citune（公有领域词谱）或 corpus_modal（全宋词实测众数），得到 {other:?}"
                ),
            }),
        }
    }
}

/// 一行破读的置信来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// 韵部实证：同部字的韵类把读音限定住了。
    RhymeAttested,
    /// 韵书调类分工：同字两读分属不同声部，且义项与声部对应。
    ToneSplit,
    /// 不覆写，只登记处置。
    EngineDefault,
}

impl Confidence {
    /// 稳定标识。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RhymeAttested => "rhyme_attested",
            Self::ToneSplit => "tone_split",
            Self::EngineDefault => "engine_default",
        }
    }

    /// 本行是否真的覆写读音。
    #[must_use]
    pub const fn overrides(self) -> bool {
        !matches!(self, Self::EngineDefault)
    }

    fn parse(raw: &str, line: usize) -> Result<Self, LexiconError> {
        match raw {
            "rhyme_attested" => Ok(Self::RhymeAttested),
            "tone_split" => Ok(Self::ToneSplit),
            "engine_default" => Ok(Self::EngineDefault),
            other => Err(LexiconError::BadField {
                file: "poyin.tsv",
                line,
                detail: format!("confidence 列不认识的值 {other:?}"),
            }),
        }
    }
}

/// 数据表的错误。每一种都指出文件与行号——数据错误如果不告诉人哪一行，改起来比重写还慢。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LexiconError {
    /// 列数不符。
    #[error("{file} 第 {line} 行有 {got} 列，应为 {want} 列")]
    BadArity {
        file: &'static str,
        line: usize,
        got: usize,
        want: usize,
    },
    /// 某一列的值不合法。
    #[error("{file} 第 {line} 行字段非法：{detail}")]
    BadField {
        file: &'static str,
        line: usize,
        detail: String,
    },
    /// 依据列不成立。**这是本模块最重要的一种错误**：它拦的是「有引用的形状但没有引用的
    /// 内容」，也就是空而存在的引用。
    #[error("{file} 第 {line} 行的依据不成立（{reason}）：{note:?}")]
    Unlocated {
        file: &'static str,
        line: usize,
        reason: &'static str,
        note: String,
    },
    /// 表头缺失或不符。
    #[error("{file} 缺少表头或表头不符，期望首个非注释行为 {want:?}")]
    BadHeader { file: &'static str, want: String },
    /// 覆盖闭合失败：名册里有多音字没有对应的破读行。
    #[error("朗读名册内 {} 个多音字在 poyin.tsv 里没有对应行：{}", missing.len(), missing.iter().collect::<String>())]
    CoverageGap { missing: BTreeSet<char> },
    /// 依据与所声明的来源类型不符。与 [`Self::Unlocated`] 拦的不是同一件事：那一种拦
    /// 「空而存在的引用」，这一种拦「引用成立但类型被冒充」——统计推断写成词谱卷页。
    #[error("{file} 第 {line} 行的依据与来源 {declared} 不符（{reason}）：{note:?}")]
    ProvenanceMismatch {
        file: &'static str,
        line: usize,
        declared: &'static str,
        reason: &'static str,
        note: String,
    },
    /// 覆盖闭合失败：名册里的词牌没有对应的句式行。
    #[error("朗读名册内 {} 支词牌在 citune_rhythm.tsv 里没有对应行：{}", missing.len(), missing.iter().cloned().collect::<Vec<_>>().join("、"))]
    TuneCoverageGap { missing: BTreeSet<String> },
}

/// 依据是否成立：既要有定位符，也要有所据版本。
///
/// 定位符判据是「定位关键字附近有序数字或阿拉伯数字」，窗口取前后 3 字——「卷六」「第三部」
/// 「n=135」「135 首」都能过，而「据某本」这种只说版本不说位置的不能过。
///
/// 返回 `Err` 时给出的是原因分类，调用方据此产出可操作的报错。
pub fn located_evidence(note: &str) -> Result<(), &'static str> {
    if note.trim().is_empty() {
        return Err("依据为空");
    }
    if find_locator(note).is_none() {
        return Err("缺卷次/部次/页码/样本量定位符");
    }
    if !has_edition_marker(note) {
        return Err("未说明所据版本（需形如「据…本」）");
    }
    Ok(())
}

fn find_locator(note: &str) -> Option<String> {
    let chars: Vec<char> = note.chars().collect();
    for (index, character) in chars.iter().enumerate() {
        if !LOCATOR_KEYWORDS.contains(character) {
            continue;
        }
        let lower = index.saturating_sub(3);
        let upper = (index + 4).min(chars.len());
        let window = &chars[lower..upper];
        if window
            .iter()
            .any(|nearby| ORDINAL_CHARS.contains(nearby) || nearby.is_ascii_digit())
        {
            return Some(window.iter().collect());
        }
    }
    // 实测类依据用 `n=135` 这种样本量定位符，它不含定位关键字，但同样能让第三方复现。
    if let Some(rest) = note.split("n=").nth(1)
        && rest.starts_with(|c: char| c.is_ascii_digit())
    {
        return Some(format!(
            "n={}",
            rest.split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or_default()
        ));
    }
    None
}

fn has_edition_marker(note: &str) -> bool {
    CITED_FROM_MARKERS.iter().any(|m| note.contains(*m))
        && EDITION_KEYWORDS.iter().any(|k| note.contains(*k))
}

/// 该定位关键字是否带序数，也就是它是不是一个真的定位符而非顺口一提。
///
/// 与 [`find_locator`] 同一个窗口判据，只是限定到指定关键字：`find_locator` 回答「有没有
/// 任何定位符」，这里回答「有没有卷/页这一种定位符」，后者是词谱依据的硬要求。
fn has_locator_for(note: &str, keyword: char) -> bool {
    let chars: Vec<char> = note.chars().collect();
    chars.iter().enumerate().any(|(index, character)| {
        *character == keyword
            && chars[index.saturating_sub(3)..(index + 4).min(chars.len())]
                .iter()
                .any(|nearby| ORDINAL_CHARS.contains(nearby) || nearby.is_ascii_digit())
    })
}

/// 词谱书名词。写下其中任何一个就是在声称「这条出自一部词谱」。
const CITUNE_WORK_MARKERS: &[&str] = &["词谱", "詞譜", "词律", "詞律"];

/// 实测口径词。写下它们表示这条是自语料统计得出的。
const CORPUS_MEASURE_MARKERS: &[&str] = &["实测", "實測", "众数", "眾數"];

/// 依据的**类型**是否与所声明的来源相符。
///
/// **这不是 [`located_evidence`] 的重复，删不掉。** 那一条只问「引用能不能被第三方翻到」，
/// `卷五` 与 `n=135` 都算合格定位符，于是它答不了「这条是词谱还是统计」——而方案对词句读的
/// 要求恰恰在类型上。判据是双向的：词谱行必须有书名、卷与页，实测行必须有样本量**且不许**
/// 出现词谱书名或卷页，后一半才是防洗白的那一半。
///
/// # Errors
///
/// 返回原因分类，调用方据此产出带行号的报错。
pub fn evidence_matches_source(source: RhythmSource, note: &str) -> Result<(), &'static str> {
    match source {
        RhythmSource::CiTune => {
            if CORPUS_MEASURE_MARKERS
                .iter()
                .any(|marker| note.contains(marker))
                || note.contains("n=")
            {
                return Err("词谱依据里出现实测口径，来源类型自相矛盾");
            }
            if !CITUNE_WORK_MARKERS
                .iter()
                .any(|marker| note.contains(marker))
            {
                return Err("词谱依据未写出词谱书名");
            }
            if !has_locator_for(note, '卷') {
                return Err("词谱依据缺卷次");
            }
            if !has_locator_for(note, '页') {
                return Err("词谱依据缺页码");
            }
            Ok(())
        }
        RhythmSource::CorpusModal => {
            if CITUNE_WORK_MARKERS
                .iter()
                .any(|marker| note.contains(marker))
            {
                return Err("实测依据引了词谱书名，等于把统计推断冒充词谱权威");
            }
            if has_locator_for(note, '卷') || has_locator_for(note, '页') {
                return Err("实测依据写了卷次或页码，等于把统计推断冒充词谱权威");
            }
            if !note.contains("n=") {
                return Err("实测依据缺样本量（需形如 n=135）");
            }
            if !CORPUS_MEASURE_MARKERS
                .iter()
                .any(|marker| note.contains(marker))
            {
                return Err("实测依据未写出实测口径");
            }
            Ok(())
        }
        RhythmSource::CharCount | RhythmSource::Punctuation => {
            Err("该来源是运行期推得的切分方式，不能作为表内行的来源")
        }
    }
}

/// TSV 的数据行，跳过注释与空行。返回 `(1 起的行号, 各列)`。
fn data_rows(text: &str) -> impl Iterator<Item = (usize, Vec<&str>)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        // BOM 在第一行会把首列名字弄脏，去掉它比在每处比较时容忍它安全。
        .map(|(number, line)| {
            (
                number,
                line.trim_start_matches('\u{feff}').trim_end_matches('\r'),
            )
        })
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(number, line)| (number, line.split('\t').collect()))
}

fn expect_header(text: &str, file: &'static str, want: &[&str]) -> Result<(), LexiconError> {
    let Some((_, header)) = data_rows(text).next() else {
        return Err(LexiconError::BadHeader {
            file,
            want: want.join("\t"),
        });
    };
    if header == want {
        return Ok(());
    }
    Err(LexiconError::BadHeader {
        file,
        want: want.join("\t"),
    })
}

/// 一行破读。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoyinRow {
    /// 字。
    pub character: char,
    /// 语境限定；`*` 表示不限。
    pub context: String,
    /// 带调拼音；不覆写时为 `None`。
    pub pinyin: Option<String>,
    /// 依据，已校验。
    pub evidence: String,
    /// 置信来源。
    pub confidence: Confidence,
}

impl PoyinRow {
    /// 本行是否适用于给定诗句。
    ///
    /// `*` 不限语境；否则要求 `line` 含 `context` 这个片段。语境判定用**包含**而不是相等：
    /// 词表里写的是「石径斜」这样的三字片段，而实际行是整句七言。
    #[must_use]
    pub fn applies_to(&self, line: &str) -> bool {
        self.context == "*" || line.contains(&self.context)
    }
}

/// 破读词表。
#[derive(Debug, Clone, Default)]
pub struct Poyin {
    rows: Vec<PoyinRow>,
}

impl Poyin {
    /// 解析随仓词表。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。**依据不成立即为硬错误**，不是警告。
    pub fn shipped() -> Result<Self, LexiconError> {
        Self::parse(POYIN_TSV)
    }

    /// 解析给定文本。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn parse(text: &str) -> Result<Self, LexiconError> {
        const FILE: &str = "poyin.tsv";
        const HEADER: &[&str] = &["字", "context", "pinyin", "依据", "confidence"];
        expect_header(text, FILE, HEADER)?;

        let mut rows = Vec::new();
        for (line, fields) in data_rows(text).skip(1) {
            if fields.len() != HEADER.len() {
                return Err(LexiconError::BadArity {
                    file: FILE,
                    line,
                    got: fields.len(),
                    want: HEADER.len(),
                });
            }
            let mut chars = fields[0].chars();
            let (Some(character), None) = (chars.next(), chars.next()) else {
                return Err(LexiconError::BadField {
                    file: FILE,
                    line,
                    detail: format!("字列必须恰好一个汉字，得到 {:?}", fields[0]),
                });
            };
            let confidence = Confidence::parse(fields[4], line)?;
            let pinyin = match (fields[2], confidence.overrides()) {
                ("-", false) => None,
                ("-", true) => {
                    return Err(LexiconError::BadField {
                        file: FILE,
                        line,
                        detail: format!(
                            "confidence 为 {} 却没有给拼音；覆写行必须给出读音",
                            confidence.as_str()
                        ),
                    });
                }
                (raw, true) if !raw.trim().is_empty() => Some(raw.to_owned()),
                (raw, true) => {
                    return Err(LexiconError::BadField {
                        file: FILE,
                        line,
                        detail: format!("拼音列为空白 {raw:?}"),
                    });
                }
                (raw, false) => {
                    return Err(LexiconError::BadField {
                        file: FILE,
                        line,
                        detail: format!(
                            "confidence 为 engine_default（不覆写）却给了拼音 {raw:?}；\
                             两者必须一致，否则读者无法判断这一行到底生效不生效"
                        ),
                    });
                }
            };
            located_evidence(fields[3]).map_err(|reason| LexiconError::Unlocated {
                file: FILE,
                line,
                reason,
                note: fields[3].to_owned(),
            })?;
            rows.push(PoyinRow {
                character,
                context: fields[1].to_owned(),
                pinyin,
                evidence: fields[3].to_owned(),
                confidence,
            });
        }
        Ok(Self { rows })
    }

    /// 全部行。
    #[must_use]
    pub fn rows(&self) -> &[PoyinRow] {
        &self.rows
    }

    /// 已登记的字，含只登记处置的。覆盖闭合检查用它。
    #[must_use]
    pub fn characters(&self) -> BTreeSet<char> {
        self.rows.iter().map(|row| row.character).collect()
    }

    /// 真正覆写读音的行数。
    #[must_use]
    pub fn override_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.confidence.overrides())
            .count()
    }

    /// 查某个字在某句里该读什么。返回 `None` 表示不覆写，交给引擎默认读音。
    ///
    /// 语境行优先于不限语境行：`*` 是兜底，具体片段是特例，顺序反了特例就永远命中不到。
    #[must_use]
    pub fn reading(&self, character: char, line: &str) -> Option<&PoyinRow> {
        let mut fallback = None;
        for row in &self.rows {
            if row.character != character || !row.confidence.overrides() || !row.applies_to(line) {
                continue;
            }
            if row.context == "*" {
                fallback = fallback.or(Some(row));
            } else {
                return Some(row);
            }
        }
        fallback
    }

    /// 一句话里全部命中的破读，按字在句中的顺序。这是注入引擎词典的输入。
    #[must_use]
    pub fn readings_in(&self, line: &str) -> Vec<(char, &PoyinRow)> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for character in line.chars() {
            if !seen.insert(character) {
                continue;
            }
            if let Some(row) = self.reading(character, line) {
                out.push((character, row));
            }
        }
        out
    }
}

/// 一支词牌的句式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiTuneRhythm {
    /// 词牌。
    pub tune: String,
    /// 逐句字数。
    pub pattern: Vec<usize>,
    /// 依据强度。
    pub source: RhythmSource,
    /// 依据，已校验。
    pub evidence: String,
}

/// 词谱句式表。
#[derive(Debug, Clone, Default)]
pub struct CiTunes {
    by_tune: BTreeMap<String, CiTuneRhythm>,
}

impl CiTunes {
    /// 解析随仓句式表。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn shipped() -> Result<Self, LexiconError> {
        Self::parse(CITUNE_TSV)
    }

    /// 解析给定文本。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn parse(text: &str) -> Result<Self, LexiconError> {
        const FILE: &str = "citune_rhythm.tsv";
        const HEADER: &[&str] = &["词牌", "句式", "来源", "依据"];
        expect_header(text, FILE, HEADER)?;

        let mut by_tune = BTreeMap::new();
        for (line, fields) in data_rows(text).skip(1) {
            if fields.len() != HEADER.len() {
                return Err(LexiconError::BadArity {
                    file: FILE,
                    line,
                    got: fields.len(),
                    want: HEADER.len(),
                });
            }
            let mut pattern = Vec::new();
            for piece in fields[1].split('-') {
                let width: usize = piece.parse().map_err(|_| LexiconError::BadField {
                    file: FILE,
                    line,
                    detail: format!("句式 {:?} 里的 {piece:?} 不是正整数", fields[1]),
                })?;
                if width == 0 {
                    return Err(LexiconError::BadField {
                        file: FILE,
                        line,
                        detail: format!("句式 {:?} 含零字句", fields[1]),
                    });
                }
                pattern.push(width);
            }
            let source = RhythmSource::parse(fields[2]).map_err(|error| match error {
                LexiconError::BadField { detail, .. } => LexiconError::BadField {
                    file: FILE,
                    line,
                    detail,
                },
                other => other,
            })?;
            located_evidence(fields[3]).map_err(|reason| LexiconError::Unlocated {
                file: FILE,
                line,
                reason,
                note: fields[3].to_owned(),
            })?;
            evidence_matches_source(source, fields[3]).map_err(|reason| {
                LexiconError::ProvenanceMismatch {
                    file: FILE,
                    line,
                    declared: source.as_str(),
                    reason,
                    note: fields[3].to_owned(),
                }
            })?;
            by_tune.insert(
                fields[0].to_owned(),
                CiTuneRhythm {
                    tune: fields[0].to_owned(),
                    pattern,
                    source,
                    evidence: fields[3].to_owned(),
                },
            );
        }
        Ok(Self { by_tune })
    }

    /// 查一支词牌。**返回 `None` 是正常路径**，调用方据此退化到按标点切分。
    #[must_use]
    pub fn get(&self, tune: &str) -> Option<&CiTuneRhythm> {
        self.by_tune.get(tune)
    }

    /// 已收录的词牌数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_tune.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_tune.is_empty()
    }

    /// 声称词谱权威的行数。v1 应为 0——仓库内没有公有领域词谱。
    #[must_use]
    pub fn citune_authority_count(&self) -> usize {
        self.by_tune
            .values()
            .filter(|row| row.source.claims_citune_authority())
            .count()
    }

    /// 已收录的词牌名。
    #[must_use]
    pub fn tunes(&self) -> BTreeSet<String> {
        self.by_tune.keys().cloned().collect()
    }
}

/// 覆盖闭合检查：名册里每一支词牌都必须在句式表里有一行。
///
/// **分母取名册而不是「宋词三百首的三百首」，因为后者在本仓库里不存在。** 方案要求覆盖
/// 宋词三百首所含词牌，而该选本的收录名单没有任何随包资产（见 `reading_roster.tsv` 顶部
/// 的说明），仓库能知道的全部成员就是名册里标了该选本的那些作品。于是这条检查与
/// [`assert_coverage`] 同形：断言的是闭合而不是百分比——百分比能靠扩大分母变好看，闭合不行。
///
/// # Errors
///
/// [`LexiconError::TuneCoverageGap`] 列出缺哪些词牌。
pub fn assert_tune_coverage(tunes: &CiTunes, roster: &Roster) -> Result<(), LexiconError> {
    let covered = tunes.tunes();
    let missing: BTreeSet<String> = roster
        .entries()
        .iter()
        .filter_map(|entry| entry.ci_tune.clone())
        .filter(|tune| !covered.contains(tune))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(LexiconError::TuneCoverageGap { missing })
}

/// 名册里的一首。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// 名册内唯一标识。
    pub id: String,
    /// 选本；`None` 表示不属于任何已评审选本名单。
    pub anthology: Option<String>,
    /// 作者。
    pub author: String,
    /// 题目。
    pub title: String,
    /// 词牌；`None` 表示诗。
    pub ci_tune: Option<String>,
    /// 正文，无换行。
    pub body: String,
    /// 依据，已校验。
    pub evidence: String,
}

/// 朗读覆盖名册。
#[derive(Debug, Clone, Default)]
pub struct Roster {
    entries: Vec<RosterEntry>,
}

impl Roster {
    /// 解析随仓名册。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn shipped() -> Result<Self, LexiconError> {
        Self::parse(ROSTER_TSV)
    }

    /// 解析给定文本。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn parse(text: &str) -> Result<Self, LexiconError> {
        const FILE: &str = "reading_roster.tsv";
        const HEADER: &[&str] = &["id", "选本", "作者", "题目", "词牌", "正文", "依据"];
        expect_header(text, FILE, HEADER)?;

        let mut entries = Vec::new();
        for (line, fields) in data_rows(text).skip(1) {
            if fields.len() != HEADER.len() {
                return Err(LexiconError::BadArity {
                    file: FILE,
                    line,
                    got: fields.len(),
                    want: HEADER.len(),
                });
            }
            if fields[5].trim().is_empty() {
                return Err(LexiconError::BadField {
                    file: FILE,
                    line,
                    detail: "正文为空".to_owned(),
                });
            }
            located_evidence(fields[6]).map_err(|reason| LexiconError::Unlocated {
                file: FILE,
                line,
                reason,
                note: fields[6].to_owned(),
            })?;
            entries.push(RosterEntry {
                id: fields[0].to_owned(),
                anthology: dash_to_none(fields[1]),
                author: fields[2].to_owned(),
                title: fields[3].to_owned(),
                ci_tune: dash_to_none(fields[4]),
                body: fields[5].to_owned(),
                evidence: fields[6].to_owned(),
            });
        }
        Ok(Self { entries })
    }

    /// 全部作品。
    #[must_use]
    pub fn entries(&self) -> &[RosterEntry] {
        &self.entries
    }

    /// 属于给定选本的作品数。
    #[must_use]
    pub fn in_anthology(&self, name: &str) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.anthology.as_deref() == Some(name))
            .count()
    }

    /// 名册全部正文里出现的汉字。
    #[must_use]
    pub fn characters(&self) -> BTreeSet<char> {
        self.entries
            .iter()
            .flat_map(|entry| entry.body.chars())
            .filter(|character| is_han(*character))
            .collect()
    }
}

fn dash_to_none(raw: &str) -> Option<String> {
    if raw == "-" || raw.trim().is_empty() {
        None
    } else {
        Some(raw.to_owned())
    }
}

/// 是否为需要读音判定的汉字。只收基本区：扩展区在本名册里不出现，收进来只会让
/// 覆盖检查去追一批根本不存在的字。
#[must_use]
pub fn is_han(character: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&character)
}

/// 多音字索引：覆盖闭合检查的候选集。
///
/// **它必须独立于破读词表。** 拿词表自己当候选集是循环论证——漏掉的字既不在词表里也不在
/// 候选集里，检查永远通过。所以候选集来自韵书的兼收判据，随仓落在
/// `data/polyphone_index.tsv`，与词表两份数据互相制约。
#[derive(Debug, Clone, Default)]
pub struct Polyphones {
    characters: BTreeSet<char>,
}

impl Polyphones {
    /// 解析随仓索引。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn shipped() -> Result<Self, LexiconError> {
        Self::parse(POLYPHONE_TSV)
    }

    /// 解析给定文本。
    ///
    /// # Errors
    ///
    /// 见 [`LexiconError`]。
    pub fn parse(text: &str) -> Result<Self, LexiconError> {
        const FILE: &str = "polyphone_index.tsv";
        const HEADER: &[&str] = &["字", "兼收"];
        expect_header(text, FILE, HEADER)?;

        let mut characters = BTreeSet::new();
        for (line, fields) in data_rows(text).skip(1) {
            if fields.len() != HEADER.len() {
                return Err(LexiconError::BadArity {
                    file: FILE,
                    line,
                    got: fields.len(),
                    want: HEADER.len(),
                });
            }
            let mut chars = fields[0].chars();
            let (Some(character), None) = (chars.next(), chars.next()) else {
                return Err(LexiconError::BadField {
                    file: FILE,
                    line,
                    detail: format!("字列必须恰好一个字，得到 {:?}", fields[0]),
                });
            };
            if fields[1].trim().is_empty() {
                return Err(LexiconError::BadField {
                    file: FILE,
                    line,
                    detail: format!("字 {character} 的兼收依据为空；判为多音字必须给出韵书证据"),
                });
            }
            characters.insert(character);
        }
        Ok(Self { characters })
    }

    /// 全部多音字。
    #[must_use]
    pub const fn characters(&self) -> &BTreeSet<char> {
        &self.characters
    }

    /// 是否为多音字。
    #[must_use]
    pub fn contains(&self, character: char) -> bool {
        self.characters.contains(&character)
    }
}

/// 覆盖闭合检查：名册里每一个多音字都必须在破读词表里有一行。
///
/// 「有一行」包含只登记处置（`engine_default`）的行：**不覆写也是一种处置**，要求写下来
/// 是为了让「漏掉」与「判过了不改」区分开——前者是缺陷，后者是决定。
///
/// # Errors
///
/// [`LexiconError::CoverageGap`] 列出缺哪些字。
pub fn assert_coverage(
    poyin: &Poyin,
    roster: &Roster,
    candidates: &BTreeSet<char>,
) -> Result<(), LexiconError> {
    let covered = poyin.characters();
    let in_roster = roster.characters();
    let missing: BTreeSet<char> = candidates
        .iter()
        .copied()
        .filter(|character| in_roster.contains(character) && !covered.contains(character))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(LexiconError::CoverageGap { missing })
}

/// 一个拼音音节的规范形式：无调基底 + 调号。
///
/// 引擎的词典就是按这两样编码的（实测 `斜 x ie 2 2`：声母、韵母、调、调），所以「把破读表里
/// 的拼音变成引擎认得的音素」这件事等于「把拼音拆成基底与调，再去词典里找一个已有的同音
/// 条目」。**这条路径刻意不自己写音素**：手写音素等于凭记忆猜引擎的音系，而借用词典里已有
/// 的同音条目是可核对的——找不到就报错，不会静默合成出一个错读音。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Syllable {
    /// 无调基底，如 `xia`。`ü` 一律写作 `ü`。
    pub base: String,
    /// 调号，1–4 为四声，5 为轻声。
    pub tone: u8,
}

/// 带调元音到 (无调元音, 调号) 的映射。只列拼音实际用到的那些。
const TONED_VOWELS: &[(char, char, u8)] = &[
    ('ā', 'a', 1),
    ('á', 'a', 2),
    ('ǎ', 'a', 3),
    ('à', 'a', 4),
    ('ē', 'e', 1),
    ('é', 'e', 2),
    ('ě', 'e', 3),
    ('è', 'e', 4),
    ('ī', 'i', 1),
    ('í', 'i', 2),
    ('ǐ', 'i', 3),
    ('ì', 'i', 4),
    ('ō', 'o', 1),
    ('ó', 'o', 2),
    ('ǒ', 'o', 3),
    ('ò', 'o', 4),
    ('ū', 'u', 1),
    ('ú', 'u', 2),
    ('ǔ', 'u', 3),
    ('ù', 'u', 4),
    ('ǖ', 'ü', 1),
    ('ǘ', 'ü', 2),
    ('ǚ', 'ü', 3),
    ('ǜ', 'ü', 4),
];

impl Syllable {
    /// 解析一个拼音音节。接受带调符（`xiá`）与尾随数字（`xia2`）两种写法。
    ///
    /// 无调号视为轻声（5），与引擎词典的约定一致。
    ///
    /// # Errors
    ///
    /// 拼音为空、含非拼音字符或有多个调号时返回说明。
    pub fn parse(pinyin: &str) -> Result<Self, String> {
        let trimmed = pinyin.trim();
        if trimmed.is_empty() {
            return Err("拼音为空".to_owned());
        }
        let mut base = String::with_capacity(trimmed.len());
        let mut tone = None;
        for character in trimmed.chars() {
            if let Some(digit) = character.to_digit(10) {
                if tone.is_some() {
                    return Err(format!("{trimmed:?} 有多个调号"));
                }
                if !(1..=5).contains(&digit) {
                    return Err(format!("{trimmed:?} 的调号 {digit} 不在 1–5"));
                }
                tone = Some(u8::try_from(digit).unwrap_or(5));
                continue;
            }
            if let Some((_, plain, marked)) = TONED_VOWELS
                .iter()
                .find(|(toned, _, _)| *toned == character)
            {
                if tone.is_some() {
                    return Err(format!("{trimmed:?} 有多个调号"));
                }
                base.push(*plain);
                tone = Some(*marked);
                continue;
            }
            match character {
                'v' | 'ü' => base.push('ü'),
                'a'..='z' => base.push(character),
                other => return Err(format!("{trimmed:?} 含非拼音字符 {other:?}")),
            }
        }
        Ok(Self {
            base,
            tone: tone.unwrap_or(5),
        })
    }
}

/// 引擎词典的音素索引。
///
/// 从 MeloTTS 中文包的 `lexicon.txt` 解析而来。实测该文件的每一个单字条目都是
/// `字 声母 韵母 调 调`（20888 条，形态完全一致），因此音节基底可以由前两个字段还原，
/// 于是这张索引同时是「拼音 → 音素」与「字 → 音素」两个方向的查表。
#[derive(Debug, Clone, Default)]
pub struct PhonemeIndex {
    by_character: BTreeMap<char, String>,
    by_syllable: BTreeMap<Syllable, String>,
}

/// 无声母音节在词典里用的伪声母。还原音节基底时要丢掉它们，否则 `AA an` 会被拼成 `aaan`。
const PSEUDO_INITIALS: &[&str] = &["AA", "EE", "OO"];

impl PhonemeIndex {
    /// 解析一份 `lexicon.txt`。
    ///
    /// 只收单字条目：多字条目（如 `一溜歪斜`）在还原音节时没有意义，而覆写行要生成的正是
    /// 多字条目，拿它们当输入只会互相污染。
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut by_character = BTreeMap::new();
        let mut by_syllable = BTreeMap::new();
        for line in text.lines() {
            let mut parts = line.split(' ');
            let Some(word) = parts.next() else { continue };
            let mut chars = word.chars();
            let (Some(character), None) = (chars.next(), chars.next()) else {
                continue;
            };
            if !is_han(character) {
                continue;
            }
            let fields: Vec<&str> = parts.collect();
            if fields.len() != 4 {
                continue;
            }
            let phonemes = fields.join(" ");
            let Ok(tone) = fields[2].parse::<u8>() else {
                continue;
            };
            let mut base = String::new();
            if !PSEUDO_INITIALS.contains(&fields[0]) {
                base.push_str(fields[0]);
            }
            base.push_str(fields[1]);
            let base = base.replace('v', "ü");
            by_syllable
                .entry(Syllable { base, tone })
                .or_insert_with(|| phonemes.clone());
            by_character.entry(character).or_insert(phonemes);
        }
        Self {
            by_character,
            by_syllable,
        }
    }

    /// 收录的单字数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_character.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_character.is_empty()
    }

    /// 某个字的默认音素。
    #[must_use]
    pub fn character(&self, character: char) -> Option<&str> {
        self.by_character.get(&character).map(String::as_str)
    }

    /// 某个读音的音素，取自词典里任一同音条目。
    #[must_use]
    pub fn syllable(&self, syllable: &Syllable) -> Option<&str> {
        self.by_syllable.get(syllable).map(String::as_str)
    }
}

/// 为破读词表生成的一条词典覆写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconOverride {
    /// 词条，即破读表里的语境片段（如 `石径斜`）。
    ///
    /// **写成多字词条而不是单字**：引擎按词切分，多字词条优先，于是「石径斜」读 xiá 而
    /// 单独一个「斜」字仍走默认读音——这正是破读表用语境列限定的意思。
    pub word: String,
    /// 该词条的逐字音素。
    pub phonemes: String,
}

impl LexiconOverride {
    /// 词典文件里的一行。
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {}", self.word, self.phonemes)
    }
}

/// 把破读词表编译成引擎词典的覆写行。
///
/// 每一条覆写的音素全部来自 `index`：被破读的那个字取目标读音的同音条目，其余字取各自的
/// 默认条目。**一个音素都不是手写的**，所以引擎音系里表达不出来的读音会在这里报错，而不是
/// 悄悄合成成别的音。
///
/// # Errors
///
/// 拼音解析失败、目标读音在词典里找不到同音条目、或语境里某个字不在词典里时，返回逐条说明。
/// 全部作为错误而非跳过：一条静默失效的破读，听起来和没有破读表一模一样。
pub fn compile_overrides(
    poyin: &Poyin,
    index: &PhonemeIndex,
) -> Result<Vec<LexiconOverride>, Vec<String>> {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for row in poyin.rows() {
        let Some(pinyin) = row.pinyin.as_deref() else {
            continue;
        };
        let syllable = match Syllable::parse(pinyin) {
            Ok(syllable) => syllable,
            Err(detail) => {
                problems.push(format!("{} 的拼音无法解析：{detail}", row.character));
                continue;
            }
        };
        let Some(target) = index.syllable(&syllable) else {
            problems.push(format!(
                "{} 的目标读音 {pinyin} 在引擎词典里没有同音条目；\
                 该读音无法表达，请换一个可核对的写法而不是手写音素",
                row.character
            ));
            continue;
        };
        let word = if row.context == "*" {
            row.character.to_string()
        } else {
            row.context.clone()
        };
        let mut phonemes = Vec::new();
        let mut failed = false;
        for character in word.chars() {
            if character == row.character {
                phonemes.push(target.to_owned());
                continue;
            }
            match index.character(character) {
                Some(default) => phonemes.push(default.to_owned()),
                None => {
                    problems.push(format!(
                        "语境 {word:?} 里的 {character} 不在引擎词典里，无法生成覆写词条"
                    ));
                    failed = true;
                    break;
                }
            }
        }
        if !failed {
            out.push(LexiconOverride {
                word,
                phonemes: phonemes.join(" "),
            });
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests;
