//! 上游语料入库器共用的粒度、处置与平仄表示。
//!
//! 入库器只产出 [`RecordInput`](crate::model::RecordInput)、平仄与处置报告，
//! 身份铸造与分组一律交给 [`crate::model::rebuild_corpus`]。

pub mod chinese_poetry;

use crate::model::Script;
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use yunjian_core::{Error, Result};

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 逐字平仄。`？`/`○` 必须落到 [`Tone::Unknown`]，不得当作平声。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Level,
    Oblique,
    Unknown,
}

impl Tone {
    /// 非平仄字符（标点）返回 `None`，由调用方跳过。
    pub const fn from_char(character: char) -> Option<Self> {
        match character {
            '平' => Some(Self::Level),
            '仄' => Some(Self::Oblique),
            '？' | '○' | '?' => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// 一行正文对应的平仄，同时保留上游原串以便复核。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrainLine {
    pub raw: String,
    pub tones: Vec<Tone>,
}

impl StrainLine {
    fn parse(raw: &str) -> Self {
        Self {
            raw: raw.to_owned(),
            tones: raw.chars().filter_map(Tone::from_char).collect(),
        }
    }
}

/// 平仄与诗篇的对齐方式。
///
/// 上游声称 `strains/json/<file>` 与 `全唐诗/<file>` 逐下标一一对应，实测其中
/// 三个文件的条数就不相等，另有 2430 条同下标 `id` 不符。所以按下标取到之后
/// **必须**用原生 `id` 复核；复核失败改走同文件内的 `id` 索引，并留下记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrainAlignment {
    Positional,
    RecoveredByNativeId,
}

/// 挂到某条记录上的平仄。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStrains {
    pub source_locator: String,
    pub lines: Vec<StrainLine>,
    pub alignment: StrainAlignment,
}

impl RecordStrains {
    pub fn unknown_count(&self) -> usize {
        self.lines
            .iter()
            .flat_map(|line| &line.tones)
            .filter(|tone| **tone == Tone::Unknown)
            .count()
    }
}

/// 一条输入未能进入可分发集合的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefectReason {
    /// 上游条目没有任何古典正文可取。
    MissingBody,
    /// 整个文件都是现代编者文字，正文无法与之分离。
    ModernCommentaryInseparable,
    /// 平仄文件与诗文件在该下标上不一致。
    StrainsMisaligned,
    /// 上游没有为该首诗算出平仄。
    StrainsUnavailable,
    /// 平仄行数与正文行数不符。挂上去会把声调错配到字上，故不挂。
    StrainsLineMismatch,
    /// 朝代串无法归一到十五个规范键。
    UnknownDynasty,
}

/// 一条输入的非入库处置。记录而不静默丢弃。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defect {
    pub relative_path: String,
    pub ordinal: usize,
    pub reason: DefectReason,
    pub detail: String,
}

/// 单个上游文件的记录数账目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTally {
    pub relative_path: String,
    pub input_records: usize,
    pub emitted_records: usize,
}

/// 一次入库的全部产出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestOutcome {
    pub records: Vec<crate::model::RecordInput>,
    pub strains: Vec<RecordStrains>,
    pub defects: Vec<Defect>,
    pub tallies: Vec<FileTally>,
}

impl IngestOutcome {
    pub fn emitted(&self) -> usize {
        self.records.len()
    }

    pub fn tally(&self, relative_path: &str) -> Option<&FileTally> {
        self.tallies
            .iter()
            .find(|tally| tally.relative_path == relative_path)
    }

    /// 归并同一资产家族（同前缀分片）的产出条数。
    pub fn emitted_under(&self, path_prefix: &str) -> usize {
        self.tallies
            .iter()
            .filter(|tally| tally.relative_path.starts_with(path_prefix))
            .map(|tally| tally.emitted_records)
            .sum()
    }
}

/// 逐记录繁简探测。
///
/// 判据是「转换后是否发生变化」而非字表命中：`t2s` 改变说明含繁体专用字形，
/// `s2t` 改变说明含简体专用字形，两者皆改即混排。都不变时该文本不含变体字
/// （如「日月」），按规范化目标记为简体。
pub struct ScriptDetector {
    to_simplified: OpenCC,
    to_traditional: OpenCC,
}

impl ScriptDetector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            to_simplified: OpenCC::from_config(BuiltinConfig::T2s)
                .map_err(|error| corpus_error(format!("初始化繁转简失败：{error}")))?,
            to_traditional: OpenCC::from_config(BuiltinConfig::S2t)
                .map_err(|error| corpus_error(format!("初始化简转繁失败：{error}")))?,
        })
    }

    pub fn detect(&self, body: &str) -> Script {
        let has_traditional = self.to_simplified.convert(body) != body;
        let has_simplified = self.to_traditional.convert(body) != body;
        match (has_traditional, has_simplified) {
            (true, true) => Script::Mixed,
            (true, false) => Script::Traditional,
            (false, _) => Script::Simplified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undetermined_tone_markers_become_unknown() {
        let line = StrainLine::parse("仄仄仄○○，平仄？？仄。");
        assert_eq!(line.tones.len(), 10);
        assert_eq!(
            line.tones,
            vec![
                Tone::Oblique,
                Tone::Oblique,
                Tone::Oblique,
                Tone::Unknown,
                Tone::Unknown,
                Tone::Level,
                Tone::Oblique,
                Tone::Unknown,
                Tone::Unknown,
                Tone::Oblique,
            ]
        );
        assert_eq!(line.raw, "仄仄仄○○，平仄？？仄。");
    }

    #[test]
    fn punctuation_is_skipped_rather_than_scored() {
        let line = StrainLine::parse("平平平仄仄，平仄仄平平。");
        assert_eq!(line.tones.len(), 10);
        assert!(!line.tones.contains(&Tone::Unknown));
    }

    #[test]
    fn script_is_detected_per_text_not_per_repository() -> Result<()> {
        let detector = ScriptDetector::new()?;
        assert_eq!(
            detector.detect("秦川雄帝宅，函谷壯皇居。"),
            Script::Traditional
        );
        assert_eq!(
            detector.detect("气和玉烛，睿化著鸿明。"),
            Script::Simplified
        );
        assert_eq!(detector.detect("日月"), Script::Simplified);
        assert_eq!(detector.detect("國破山河在，国破山河在"), Script::Mixed);
        Ok(())
    }
}
