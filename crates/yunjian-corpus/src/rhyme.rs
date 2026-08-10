//! `charlesix59/chinese_word_rhyme` 韵书导入，以及逐字反向索引的构建期推导。
//!
//! 本模块的形状由三件事决定。
//!
//! # 一、判定是逐资产的，不是逐仓库的
//!
//! 上游整体是 MIT，但那份许可只覆盖它自己的整理工作，无法为它抓来或转录的内容
//! 授权。所以 [`SHIPPED_ASSETS`] 是一份显式白名单，只列三个文件：平水韵与词林正韵
//! （底本都是前现代韵书，公有领域）以及由前者派生的逐字平仄表。
//! `Ci_Tunes.json`（19.6 MB 词谱，抓自商业站点 sou-yun.cn）、
//! `Xinyun_Rhyme.json` 及其四声版（中华新韵是 2005 年现代出版物）等被扣留的资产
//! **在本模块里没有任何读取路径**——不是「读进来再过滤」，而是根本没有代码能打开它们。
//!
//! # 二、两本书的嵌套顺序相反
//!
//! 平水韵是 `声部 -> 韵部 -> [字]`，词林正韵是 `部 -> 声 -> [字]`。两者不能共用一个
//! 解析函数，但**必须产出同一种行**（[`RhymeEntry`]），否则下游每次查询都得先问
//! 「这本书是哪种嵌套」。所以本模块有两个解析器、一个输出类型。
//!
//! # 三、逐字反向索引是推导出来的，不是引来的
//!
//! 计划原本要引 `jkak/pingShuiYun` 的 `baseCharDict.json` 来做「字 ->（声调, 韵部）」
//! 的反向索引，todo 9 实测该仓库在任何 revision 都没有 LICENSE，故拒绝。能力不受损：
//! 反向索引就是把 [`RhymeTable`] 的嵌套翻过来，见 [`CharacterRhymeIndex::derive`]。
//! 这样反而更好——索引与我们实际分发的韵部数据必然一致，不存在两份数据对不上的可能。

use crate::ingest::corpus_error;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use yunjian_core::rhyme::{RhymeBook, RhymeTone};
use yunjian_core::{Error, Result};

/// `sources.toml` 里的来源名。
pub const SOURCE_NAME: &str = "charlesix59/chinese_word_rhyme";

/// `corpus/sources.toml` 锁定的 revision。导入只认这一个版本。
pub const SOURCE_REV: &str = "ff0e9c13fb037c43e0eaa5dc929c0fe4fa2ffb18";

const LICENSE: &str = "MIT";

/// 可分发资产的显式白名单。被扣留的资产不在此列，因此没有读取路径。
pub const SHIPPED_ASSETS: [&str; 3] = [
    "data/Pingshui_Rhyme.json",
    "data/Cilin_Rhyme.json",
    "data/Word_Tune.json",
];

/// 因授权链未核实而扣留的资产。
///
/// 列出它们不是为了读，而是为了让「这些文件不得进入分发」成为代码里的断言对象：
/// [`tests`] 会验证白名单与本表无交集，于是把某个扣留资产误加进白名单会让测试失败，
/// 而不是靠谁记得这条规则。
pub const WITHHELD_ASSETS: [&str; 6] = [
    "data/Xinyun_Rhyme.json",
    "data/Xinyun_Rhyme_FourRhyme_Edition.json",
    "script/data/xinyun.txt",
    "data/Ci_Tunes.json",
    "data/Ci_Catalog.json",
    "data/Word_Explain.json",
];

/// 平水韵的五个声部键，以及各自对应的声调。
///
/// 写成显式白名单而不是遍历 JSON 顶层键，理由和 `chinese_poetry` 的资产表一样：
/// 上游多出一个键时应当是**硬错误**，而不是被当成一个新声调悄悄收下。
/// 上平与下平都归 [`RhymeTone::Level`]——那是刻本的分卷，不是声调的分别。
const PINGSHUI_TONES: [(&str, RhymeTone); 5] = [
    ("上平声部", RhymeTone::Level),
    ("下平声部", RhymeTone::Level),
    ("上声部", RhymeTone::Rising),
    ("去声部", RhymeTone::Departing),
    ("入声部", RhymeTone::Entering),
];

/// 词林正韵的三个声键。上游把上去两声并成「仄声」，此处不拆——上游没有那个信息。
const CILIN_TONES: [(&str, RhymeTone); 3] = [
    ("平声", RhymeTone::Level),
    ("仄声", RhymeTone::Oblique),
    ("入声", RhymeTone::Entering),
];

/// 韵书表里的一行：某个字属于某本书的某个韵部的某个声。
///
/// 两个解析器产出的都是这个类型，尽管上游嵌套方向相反。`tone_raw` 保留上游原键
/// （如 `上平声部` / `仄声`），沿用本项目 `dynasty_raw` 的同一条约定：归一化的值用于
/// 查询，原始串永远留着以便复核。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RhymeEntry {
    pub book: RhymeBook,
    pub rhyme_group: String,
    pub tone: RhymeTone,
    pub tone_raw: String,
    pub character: String,
}

/// 一本韵书的全部行，以及解析过程中的账目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhymeTable {
    pub book: RhymeBook,
    entries: Vec<RhymeEntry>,
    /// 上游同一韵部内重复列出同一个字的次数。实测平水韵有 266 处，故必须去重并记账。
    duplicate_entries: usize,
}

impl RhymeTable {
    pub fn entries(&self) -> &[RhymeEntry] {
        &self.entries
    }

    pub fn duplicate_entries(&self) -> usize {
        self.duplicate_entries
    }

    /// 韵部总数。
    pub fn group_count(&self) -> usize {
        self.groups().len()
    }

    /// 全部韵部名，按上游原名排序去重。
    pub fn groups(&self) -> BTreeSet<&str> {
        self.entries
            .iter()
            .map(|entry| entry.rhyme_group.as_str())
            .collect()
    }

    /// 按上游原始声部键分组的韵部数。
    ///
    /// 平水韵的 `去声部` 恰好三十个韵部，这是「平水韵三十韵部」这句话唯一成立的读法；
    /// 全书是一百零五个（见模块级文档与 `docs/CORPUS.zh.md`）。
    pub fn group_count_by_tone_raw(&self) -> BTreeMap<&str, usize> {
        let mut by_tone: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for entry in &self.entries {
            by_tone
                .entry(entry.tone_raw.as_str())
                .or_default()
                .insert(entry.rhyme_group.as_str());
        }
        by_tone
            .into_iter()
            .map(|(tone, groups)| (tone, groups.len()))
            .collect()
    }

    /// 该韵部下的全部字。
    pub fn characters_in(&self, rhyme_group: &str) -> BTreeSet<&str> {
        self.entries
            .iter()
            .filter(|entry| entry.rhyme_group == rhyme_group)
            .map(|entry| entry.character.as_str())
            .collect()
    }

    pub fn distinct_characters(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.character.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// 逐字反向索引：字 -> 该字所属的全部（声调, 韵部）。
///
/// 这就是 `jkak/pingShuiYun` 本要提供的那份数据，改由构建期反转 [`RhymeTable`] 得到。
/// 一个字可以落在多个韵部（实测平水韵有 1992 个这样的字），所以值是列表而不是单值：
/// 「临」既在下平十二侵也在去声二十七沁，两行都要在。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterRhymeIndex {
    book: Option<RhymeBook>,
    by_character: BTreeMap<String, Vec<(RhymeTone, String)>>,
}

impl CharacterRhymeIndex {
    /// 反转一本韵书得到逐字索引。
    ///
    /// 去重发生在这里：上游同一韵部内重复列出同一个字（实测 266 处）不应让该字在索引里
    /// 出现两次相同的（声调, 韵部）。`BTreeSet` 顺带给出确定顺序，让构建产物可复现。
    pub fn derive(table: &RhymeTable) -> Self {
        let mut deduped: BTreeMap<String, BTreeSet<(RhymeTone, String)>> = BTreeMap::new();
        for entry in table.entries() {
            deduped
                .entry(entry.character.clone())
                .or_default()
                .insert((entry.tone, entry.rhyme_group.clone()));
        }
        Self {
            book: Some(table.book),
            by_character: deduped
                .into_iter()
                .map(|(character, groups)| (character, groups.into_iter().collect()))
                .collect(),
        }
    }

    pub fn book(&self) -> Option<RhymeBook> {
        self.book
    }

    /// 该字的全部（声调, 韵部）。字不在韵书里时返回空切片。
    ///
    /// 注意这里的空切片与「韵书缺失」是两件不同的事：字不在韵书里是一个**有效的**
    /// 否定答案，而韵书本身缺失必须在更早的 [`RhymeBook::ensure_available`] 处变成错误。
    pub fn lookup(&self, character: &str) -> &[(RhymeTone, String)] {
        self.by_character
            .get(character)
            .map_or(&[], |groups| groups.as_slice())
    }

    pub fn len(&self) -> usize {
        self.by_character.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_character.is_empty()
    }

    /// 归属多个韵部的字数。
    pub fn polyphone_count(&self) -> usize {
        self.by_character
            .values()
            .filter(|groups| groups.len() > 1)
            .count()
    }

    /// 该字在本书中是否兼具平声与仄声归属。
    pub fn is_tone_ambiguous(&self, character: &str) -> bool {
        let groups = self.lookup(character);
        groups.iter().any(|(tone, _)| tone.is_level())
            && groups.iter().any(|(tone, _)| !tone.is_level())
    }
}

/// 上游 `Word_Tune.json` 给出的逐字平仄判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredTune {
    Level,
    Oblique,
    /// 上游写作「多」，即该字平仄两读。
    Either,
}

impl DeclaredTune {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "平" => Some(Self::Level),
            "仄" => Some(Self::Oblique),
            "多" => Some(Self::Either),
            _ => None,
        }
    }
}

/// 逐字平仄表与反向索引不一致的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToneDivergence {
    pub character: String,
    pub declared: DeclaredTune,
    /// 反向索引给出的（声调, 韵部）全集。
    pub derived: Vec<(RhymeTone, String)>,
}

/// `Word_Tune.json` 与反向索引的交叉核对结果。
///
/// # 为什么要核对，以及为什么以反向索引为准
///
/// 实测 `Word_Tune.json` 的 8232 个字键与平水韵的 8232 个不同字**完全相同**，可见它就是
/// 平水韵的逐字归约。但两者有 157 处不一致，且形态整齐得可疑：全部是反向索引判为
/// 平仄两读（「多」）而上游只写了「仄」。以「空」为例，它在上平一东（平）、上声一董与
/// 去声一送（仄）都出现，确实两读，上游却标成仄。
///
/// 这不是无关紧要的差异：若采信上游，格律检查会把「空山不见人」判为出律。所以声调维度
/// **以反向索引为准**（它与我们实际分发的韵部数据必然自洽，可逐条追溯到韵部），
/// `Word_Tune.json` 降级为交叉核对的一方，分歧数进质量报告而不是被静默采信。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToneCrossCheck {
    pub declared_rows: usize,
    pub agreements: usize,
    pub divergences: Vec<ToneDivergence>,
    /// 出现在平仄表里但不在韵书里的字。实测为 0。
    pub declared_only: Vec<String>,
    /// 出现在韵书里但不在平仄表里的字。实测为 0。
    pub index_only: Vec<String>,
}

impl ToneCrossCheck {
    pub fn divergence_count(&self) -> usize {
        self.divergences.len()
    }
}

/// 一次韵书导入的全部产出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhymeImport {
    pub pingshui: RhymeTable,
    pub cilin: RhymeTable,
    pub pingshui_index: CharacterRhymeIndex,
    pub cilin_index: CharacterRhymeIndex,
    pub tone_cross_check: ToneCrossCheck,
}

impl RhymeImport {
    /// 取某本韵书的表。未随包的韵书返回类型化的
    /// [`Error::RhymeBookUnavailable`]，**绝不返回空表**。
    pub fn table(&self, book: RhymeBook) -> Result<&RhymeTable> {
        book.ensure_available()?;
        match book {
            RhymeBook::Pingshui => Ok(&self.pingshui),
            RhymeBook::Cilin => Ok(&self.cilin),
            RhymeBook::Xinyun => Err(unreachable_after_gate(book)),
        }
    }

    /// 取某本韵书的逐字索引。未随包的韵书返回错误。
    pub fn index(&self, book: RhymeBook) -> Result<&CharacterRhymeIndex> {
        book.ensure_available()?;
        match book {
            RhymeBook::Pingshui => Ok(&self.pingshui_index),
            RhymeBook::Cilin => Ok(&self.cilin_index),
            RhymeBook::Xinyun => Err(unreachable_after_gate(book)),
        }
    }

    /// 查某个字在某本韵书里的（声调, 韵部）。
    ///
    /// 韵书维度是必填参数，所以「不指定韵书就查押韵」在类型上不可表达。
    pub fn lookup(&self, book: RhymeBook, character: &str) -> Result<&[(RhymeTone, String)]> {
        Ok(self.index(book)?.lookup(character))
    }

    pub fn total_entries(&self) -> usize {
        self.pingshui.entries().len() + self.cilin.entries().len()
    }
}

/// [`RhymeBook::ensure_available`] 已经把未随包的书拦在前面，走到这里说明闸门被绕过了。
fn unreachable_after_gate(book: RhymeBook) -> Error {
    corpus_error(format!(
        "{} 通过了可用性闸门却没有数据表，说明闸门与数据不一致",
        book.display_name()
    ))
}

/// 平水韵：`声部 -> 韵部 -> [字]`。
fn parse_pingshui(raw: &str, origin: &str) -> Result<RhymeTable> {
    let nested: BTreeMap<String, BTreeMap<String, Vec<String>>> = parse_json(raw, origin)?;
    let mut entries = Vec::new();
    let mut duplicate_entries = 0usize;

    for (tone_raw, tone) in PINGSHUI_TONES {
        let groups = nested.get(tone_raw).ok_or_else(|| {
            corpus_error(format!(
                "{origin} 缺少声部键 `{tone_raw}`；平水韵的五个声部是硬结构"
            ))
        })?;
        for (rhyme_group, characters) in groups {
            let mut seen = BTreeSet::new();
            for character in characters {
                if !seen.insert(character.clone()) {
                    duplicate_entries += 1;
                }
                entries.push(RhymeEntry {
                    book: RhymeBook::Pingshui,
                    rhyme_group: rhyme_group.clone(),
                    tone,
                    tone_raw: tone_raw.to_owned(),
                    character: character.clone(),
                });
            }
        }
    }

    reject_unknown_keys(
        &nested,
        PINGSHUI_TONES.map(|(key, _)| key).as_slice(),
        origin,
    )?;
    finish_table(RhymeBook::Pingshui, entries, duplicate_entries, origin)
}

/// 词林正韵：`部 -> 声 -> [字]`，与平水韵**方向相反**。
///
/// 十九部里十四部为 `平声` + `仄声`，五部只有 `入声`（入声独立成部，故无平仄之分）。
/// 声键必须在 [`CILIN_TONES`] 之内，遇到未知键是硬错误——猜一个声调等于编造格律。
fn parse_cilin(raw: &str, origin: &str) -> Result<RhymeTable> {
    let nested: BTreeMap<String, BTreeMap<String, Vec<String>>> = parse_json(raw, origin)?;
    let mut entries = Vec::new();
    let mut duplicate_entries = 0usize;

    for (rhyme_group, tones) in &nested {
        for (tone_raw, characters) in tones {
            let tone = CILIN_TONES
                .iter()
                .find(|(key, _)| key == tone_raw)
                .map(|(_, tone)| *tone)
                .ok_or_else(|| {
                    corpus_error(format!(
                        "{origin} 的 `{rhyme_group}` 出现未知声键 `{tone_raw}`；\
                         声调不可推测，只能是 平声 / 仄声 / 入声 之一"
                    ))
                })?;
            let mut seen = BTreeSet::new();
            for character in characters {
                if !seen.insert(character.clone()) {
                    duplicate_entries += 1;
                }
                entries.push(RhymeEntry {
                    book: RhymeBook::Cilin,
                    rhyme_group: rhyme_group.clone(),
                    tone,
                    tone_raw: tone_raw.clone(),
                    character: character.clone(),
                });
            }
        }
    }

    finish_table(RhymeBook::Cilin, entries, duplicate_entries, origin)
}

fn finish_table(
    book: RhymeBook,
    entries: Vec<RhymeEntry>,
    duplicate_entries: usize,
    origin: &str,
) -> Result<RhymeTable> {
    if entries.is_empty() {
        return Err(corpus_error(format!(
            "{origin} 解析出 0 条韵书行；声明为可分发资产的文件不得空吞"
        )));
    }
    tracing::info!(
        source = SOURCE_NAME,
        book = book.as_key(),
        origin,
        entries = entries.len(),
        duplicate_entries,
        "韵书解析完成"
    );
    Ok(RhymeTable {
        book,
        entries,
        duplicate_entries,
    })
}

fn parse_tone_table(raw: &str, origin: &str) -> Result<BTreeMap<String, DeclaredTune>> {
    let declared: BTreeMap<String, String> = parse_json(raw, origin)?;
    declared
        .into_iter()
        .map(|(character, value)| {
            DeclaredTune::parse(&value)
                .map(|tune| (character.clone(), tune))
                .ok_or_else(|| {
                    corpus_error(format!(
                        "{origin} 的 `{character}` 平仄值为 `{value}`；只接受 平 / 仄 / 多"
                    ))
                })
        })
        .collect()
}

/// 用 `Word_Tune.json` 交叉核对反向索引。以索引为准，分歧只记录不改写。
fn cross_check_tones(
    index: &CharacterRhymeIndex,
    declared: &BTreeMap<String, DeclaredTune>,
) -> ToneCrossCheck {
    let mut check = ToneCrossCheck {
        declared_rows: declared.len(),
        ..ToneCrossCheck::default()
    };

    for (character, tune) in declared {
        let derived = index.lookup(character);
        if derived.is_empty() {
            check.declared_only.push(character.clone());
            continue;
        }
        let has_level = derived.iter().any(|(tone, _)| tone.is_level());
        let has_oblique = derived.iter().any(|(tone, _)| !tone.is_level());
        let expected = match (has_level, has_oblique) {
            (true, true) => DeclaredTune::Either,
            (true, false) => DeclaredTune::Level,
            (false, _) => DeclaredTune::Oblique,
        };
        if expected == *tune {
            check.agreements += 1;
        } else {
            check.divergences.push(ToneDivergence {
                character: character.clone(),
                declared: *tune,
                derived: derived.to_vec(),
            });
        }
    }

    check.index_only = index
        .by_character
        .keys()
        .filter(|character| !declared.contains_key(*character))
        .cloned()
        .collect();
    check
}

fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str, origin: &str) -> Result<T> {
    serde_json::from_str(raw).map_err(|error| corpus_error(format!("解析 {origin} 失败：{error}")))
}

fn reject_unknown_keys<V>(
    nested: &BTreeMap<String, V>,
    allowed: &[&str],
    origin: &str,
) -> Result<()> {
    let unknown: Vec<&str> = nested
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(corpus_error(format!(
        "{origin} 出现未登记的顶层键 {unknown:?}；结构变化必须显式处理而非静默收下"
    )))
}

fn read_asset(root: &Path, relative_path: &str) -> Result<(String, String)> {
    if !SHIPPED_ASSETS.contains(&relative_path) {
        return Err(corpus_error(format!(
            "{relative_path} 不在可分发资产白名单内，拒绝读取"
        )));
    }
    let path: PathBuf = root.join(relative_path);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", path.display())))?;
    Ok((raw, path.display().to_string()))
}

/// 从上游检出目录导入三个可分发资产。
///
/// 被扣留的资产没有读取路径：[`read_asset`] 只接受 [`SHIPPED_ASSETS`] 里的路径，
/// 所以即便有人传入 `data/Ci_Tunes.json` 也会得到错误而不是数据。
pub fn import(root: impl AsRef<Path>) -> Result<RhymeImport> {
    let root = root.as_ref();

    let (pingshui_raw, pingshui_origin) = read_asset(root, SHIPPED_ASSETS[0])?;
    let pingshui = parse_pingshui(&pingshui_raw, &pingshui_origin)?;

    let (cilin_raw, cilin_origin) = read_asset(root, SHIPPED_ASSETS[1])?;
    let cilin = parse_cilin(&cilin_raw, &cilin_origin)?;

    let pingshui_index = CharacterRhymeIndex::derive(&pingshui);
    let cilin_index = CharacterRhymeIndex::derive(&cilin);

    let (tune_raw, tune_origin) = read_asset(root, SHIPPED_ASSETS[2])?;
    let declared = parse_tone_table(&tune_raw, &tune_origin)?;
    let tone_cross_check = cross_check_tones(&pingshui_index, &declared);

    tracing::info!(
        source = SOURCE_NAME,
        rev = SOURCE_REV,
        license = LICENSE,
        pingshui_groups = pingshui.group_count(),
        cilin_groups = cilin.group_count(),
        reverse_index_chars = pingshui_index.len(),
        tone_divergences = tone_cross_check.divergence_count(),
        "韵书导入完成；声调维度以反向索引为准，平仄表仅作交叉核对"
    );

    Ok(RhymeImport {
        pingshui,
        cilin,
        pingshui_index,
        cilin_index,
        tone_cross_check,
    })
}

#[cfg(test)]
mod tests;
