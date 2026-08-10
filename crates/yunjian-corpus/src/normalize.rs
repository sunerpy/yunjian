//! 构建期繁简归一，以及供运行时改写查询的字形映射表。
//!
//! 本模块的形状由四个已实测的事实决定。
//!
//! # 一、转换只在构建期发生，运行时一个转换字典都不带
//!
//! 全文索引只建在 [`NormalizedRecord::body`] 这一列上（简体）。用户输入
//! 「國破山河在」能搜到，靠的**不是**运行时再跑一次 OpenCC，也**不是**建第二个
//! 繁体索引列，而是本模块同时产出的 [`VariantMap`]：一张 `(src_char, dst_char)`
//! 表，进语料库；运行时逐字查表改写查询即可。
//!
//! 两条硬约束都由此而来：
//!
//! - **不建第二个索引列**。todo 43 实测 `detail=full` 加 n-gram 辅助表已经是
//!   28.9 MB，而 CJK trigram 本身放大 2.2–2.6 倍；再复制一列繁体会让这个体积
//!   预算直接失效。
//! - **`yunjian-core` 不依赖任何转换 crate**。`ferrous-opencc` 只出现在本 crate
//!   的 `[dependencies]` 里，发布二进制既不链接它，也不需要随包带字典。
//!
//! # 二、原字形逐字保留，因为转换会放大上游的抄录错误
//!
//! 上游 issue #261 记录了约 4,278 处疑似讹误，其中一类是形近误录：「傅」被录成
//! 「傳」，转换后成了「传」，错误从此更难辨认。所以
//! [`NormalizedRecord::body_original`] 与输入逐字节相同，任何时候都能回到源字形
//! 复核；并且凡转换非往返稳定的记录都会得到一条 `conversion_unstable`
//! 发现（[`NormalizationReason`]），进缺陷报告而不是被静默接受。
//!
//! # 三、字级表必须挡住「归一后仍然存活」的字，否则会砸掉精确命中
//!
//! `t2s` 是短语感知的：单字「乾」转成「干」，但「乾坤」整体保持「乾坤」。于是
//! 「乾」既可能被转换、也可能原样进索引。这时若把 `乾 -> 干` 放进查询改写表，
//! 用户照着语料原文输入「乾坤」反而会被改写成「干坤」而**一条都搜不到**——用户
//! 输的字与索引里的字完全一致却查不到，是比「靠转换才能命中的那类查询失败」严重
//! 得多的故障。所以 [`VariantMap`] 的生成规则是两条而不是一条：
//!
//! 1. 目标字必须出现在归一后的语料里（否则改写过去也是空命中）；
//! 2. **源字不得出现在归一后的语料里**（否则改写会砸掉精确命中）。
//!
//! 这也是「只含语料中实际出现的映射」的确切含义：表由语料裁剪，而不是把整本
//! OpenCC 字典倒进数据库。
//!
//! # 四、异体字覆盖只能来自 OpenCC 的中文侧配置，日文侧配置实测有害
//!
//! `t2s` 覆盖繁简与大部分异体，但实测漏掉「氷」（`冰` 的异体）与「馀」。补齐的
//! 办法按代价从小到大试过三种：
//!
//! - **繁体往返推导**（采用）：对 `t2s` 不动的字算 `t2s(s2t(c))`，若变了且目标字
//!   自己还有一个不同的繁体对应，就是一条 OpenCC 自己的字典能证明的异体映射。
//!   实测得 31 行，「馀 -> 余」在内，全部由同一套字典推出，无人工数据。
//! - **`tw2s` / `hk2s`**（拒绝）：相对 `t2s` 只多 7 / 8 行，却包含
//!   「著 -> 着」「么 -> 幺」——「著」在文言里是独立用字（宋词「睿化著鸿明」），
//!   改写它会把正确的输入改错。
//! - **`jp2t` 链**（拒绝）：它确实有「氷 -> 冰」，但同一份 JPVariants 还带
//!   「予 -> 豫」「連 -> 联」「緒 -> 緖」。「予」是文言常用字，这类改写是数据
//!   损坏而不是覆盖提升，故整个配置弃用。
//!
//! 剩下的唯一缺口「氷 -> 冰」由 [`VARIANT_SUPPLEMENT`] 单独补一行并逐行说明理由，
//! 且有测试盯着它不许长大、不许与字典重复。

#[cfg(test)]
mod tests;

use crate::ingest::corpus_error;
use crate::model::{CanonicalRecord, Script};
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use yunjian_core::Result;

/// 枚举候选字形所扫描的 CJK 表意文字区段。
///
/// `ferrous-opencc` 不公开字典内容，所以字级映射是**探测**出来的：逐码位跑一遍
/// 转换器，变了就是一条候选。这等价于从同一套字典里取表，且不依赖任何内部 API。
///
/// 表里含几个实测零命中的区段（康熙部首、兼容表意文字、扩展 G/H/I）。留着是因为
/// 整轮扫描不到 40 毫秒，而漏掉一个区段的代价是某个生僻异体永远搜不到——用确定性
/// 换一点常数时间是划算的。
const CJK_BLOCKS: [(u32, u32); 11] = [
    (0x2E80, 0x2EFF),   // CJK 部首补充
    (0x2F00, 0x2FDF),   // 康熙部首
    (0x3400, 0x4DBF),   // 扩展 A
    (0x4E00, 0x9FFF),   // 基本区
    (0xF900, 0xFAFF),   // 兼容表意文字
    (0x20000, 0x2A6DF), // 扩展 B
    (0x2A700, 0x2EBEF), // 扩展 C/D/E/F
    (0x2EBF0, 0x2EE5F), // 扩展 I
    (0x2F800, 0x2FA1F), // 兼容表意文字补充
    (0x30000, 0x3134F), // 扩展 G
    (0x31350, 0x323AF), // 扩展 H
];

/// OpenCC 中文侧配置覆盖不到、需人工补齐的异体映射。
///
/// 每一行都必须满足三件事，且都有测试守着：OpenCC 的 `t2s` 与繁体往返推导都推不
/// 出它；目标字自身已是简体（`t2s` 的不动点）；有一个仓库内的契约或实测缺口要求
/// 它存在。表长是审计对象——它是「字典漏了这一个」的补丁，不是第二本字典。
///
/// - `氷 -> 冰`：`氷` 是 `冰` 的异体。`crates/yunjian-core/tests/queries.toml` 的
///   契约 `q17-variant-bingsaichuan` 用「氷塞川」断言异体输入必须命中《行路难》，
///   而 `t2s`、`tw2s`、`hk2s` 与繁体往返推导对 `氷` 全都不动。唯一有这条映射的
///   `jp2t` 因同时带「予 -> 豫」「連 -> 联」而整体弃用（见模块文档第四节）。
pub const VARIANT_SUPPLEMENT: [(char, char); 1] = [('氷', '冰')];

/// 归一后的逐记录检索视图。
///
/// 刻意**不**把 `body` 挂到 [`CanonicalRecord`] 上：那是身份与来源记录，
/// `content_hash`、`work_group`、`edition_group` 都由它的字段算出，往里加一个派生
/// 字段就得重新论证那三个键的语义。归一是身份铸造之后的独立工序，产出独立视图，
/// 由 `stable_id` 与规范记录相连。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedRecord {
    pub stable_id: String,
    /// 规范简体正文。**全文索引只建在这一列上。**
    pub body: String,
    /// 与 `body` 对应的逐行切分，供下游推导首句与逐句末字。
    pub body_lines: Vec<String>,
    /// 源字形，与输入逐字节相同。
    pub body_original: String,
    /// 描述 `body_original` 的书写系统，沿用入库期的逐记录判定。
    pub script: Script,
}

/// 归一阶段产出的发现类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizationReason {
    /// 转换非往返稳定：同一段文本再归一一次、或经繁体往返后不回到原处。
    ///
    /// 这类记录**仍然入库**——它是待复核的告警，不是排除处置。
    ConversionUnstable,
}

impl NormalizationReason {
    /// 缺陷报告里使用的稳定字符串码。
    pub const fn as_reason_code(self) -> &'static str {
        match self {
            Self::ConversionUnstable => "conversion_unstable",
        }
    }
}

/// 一条归一发现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationFinding {
    pub stable_id: String,
    pub reason: NormalizationReason,
    pub detail: String,
}

/// `variant_map` 表的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantRow {
    pub src_char: char,
    pub dst_char: char,
}

/// 进语料库的 `variant_map` 表：运行时据此逐字改写查询。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantMap {
    rows: BTreeMap<char, char>,
}

impl VariantMap {
    /// 按 `src_char` 升序返回全部行。
    ///
    /// 顺序是确定的，因为构建产物要求逐字节可复现：同一份语料必须写出同样的表。
    pub fn rows(&self) -> Vec<VariantRow> {
        self.rows
            .iter()
            .map(|(src_char, dst_char)| VariantRow {
                src_char: *src_char,
                dst_char: *dst_char,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// 查一个字的目标字形。
    pub fn get(&self, src_char: char) -> Option<char> {
        self.rows.get(&src_char).copied()
    }

    /// 逐字改写查询，与运行时的 SQL 查表改写等价。
    ///
    /// 表里没有的字原样保留——包括标点、拉丁字母，以及归一后仍然存活的字形。
    pub fn rewrite(&self, query: &str) -> String {
        query
            .chars()
            .map(|character| self.get(character).unwrap_or(character))
            .collect()
    }

    /// 删掉一行并返回原目标字形。
    ///
    /// 存在的理由只有一个：让「繁体输入到底靠什么才能命中」成为可证伪的命题。
    /// 删掉 `國` 那一行之后，繁体改写测试必须变红；若它照样通过，说明命中来自某个
    /// 隐藏的运行时依赖，而不是这张表。
    pub fn remove(&mut self, src_char: char) -> Option<char> {
        self.rows.remove(&src_char)
    }
}

/// `variant_map` 生成过程的账目，使裁剪规则可被断言而不只是被相信。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantMapStats {
    /// 从字典探测到的字级候选总数（与语料无关）。
    pub candidates: usize,
    /// 最终入表行数。
    pub rows: usize,
    /// 因目标字未出现在归一后语料里而丢弃。
    pub dropped_target_absent: usize,
    /// 因源字在归一后语料里仍然存活而丢弃（会砸掉精确命中）。
    pub dropped_source_survives: usize,
    /// 因转换结果不是单字而丢弃：字级表表达不了一对多。
    pub dropped_not_single_char: usize,
}

/// 一次构建期归一的全部产出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizeOutcome {
    pub records: Vec<NormalizedRecord>,
    pub variant_map: VariantMap,
    pub stats: VariantMapStats,
    pub findings: Vec<NormalizationFinding>,
}

impl NormalizeOutcome {
    /// 命中某个发现类型的记录数。
    pub fn finding_count(&self, reason: NormalizationReason) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.reason == reason)
            .count()
    }
}

/// 构建期繁简归一器。
///
/// 两个转换器与一张残余异体表在构造时一次性备好，之后整批语料复用同一个实例；
/// 逐记录新建转换器会把一次全量构建变成几十万次字典装载。
pub struct Normalizer {
    to_simplified: OpenCC,
    to_traditional: OpenCC,
    /// `t2s` 单字不动、但由繁体往返或补充表可证的异体映射。
    ///
    /// 它同时用于两处：归一正文（在 `t2s` 之后逐字兜底）与作为 `variant_map` 的
    /// 候选。之所以能在 `t2s` 之后安全地逐字应用，是因为这些字按定义就是 `t2s`
    /// 单字不动的；若某条短语规则已经在上一步把它换掉了，这一步自然扫不到它。
    residual: BTreeMap<char, char>,
    /// 字级候选全表：`t2s` 直接映射并上残余异体表。
    candidates: BTreeMap<char, char>,
}

impl Normalizer {
    pub fn new() -> Result<Self> {
        let to_simplified = OpenCC::from_config(BuiltinConfig::T2s)
            .map_err(|error| corpus_error(format!("初始化繁转简失败：{error}")))?;
        let to_traditional = OpenCC::from_config(BuiltinConfig::S2t)
            .map_err(|error| corpus_error(format!("初始化简转繁失败：{error}")))?;
        let mut normalizer = Self {
            to_simplified,
            to_traditional,
            residual: BTreeMap::new(),
            candidates: BTreeMap::new(),
        };
        normalizer.residual = normalizer.derive_residual_variants();
        normalizer.candidates = normalizer.derive_candidates();
        Ok(normalizer)
    }

    /// 短语感知的繁转简，不含残余异体兜底。
    ///
    /// 单独暴露是为了让「`t2s` 到底覆盖到哪里」可被测试直接观察，从而使
    /// [`VARIANT_SUPPLEMENT`] 的每一行都有一条可证伪的理由。
    pub fn simplify(&self, text: &str) -> String {
        self.to_simplified.convert(text)
    }

    /// 正文归一：先短语感知转换，再用残余异体表逐字兜底。
    pub fn canonicalize(&self, text: &str) -> String {
        self.simplify(text)
            .chars()
            .map(|character| self.residual.get(&character).copied().unwrap_or(character))
            .collect()
    }

    /// 字级候选总表，与语料无关。
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// 残余异体映射，供测试断言其来源与规模。
    pub fn residual_variants(&self) -> Vec<VariantRow> {
        self.residual
            .iter()
            .map(|(src_char, dst_char)| VariantRow {
                src_char: *src_char,
                dst_char: *dst_char,
            })
            .collect()
    }

    /// 转换是否往返稳定；不稳定时返回可读的原因。
    ///
    /// 两项检查：再归一一次是否是不动点；经简转繁再转回来是否回到原处。后者才是
    /// 抓异体缺口的那一项——「馀」正是这样被抓出来的：`t2s` 不动它，而
    /// `t2s(s2t(馀))` 得到「余」。
    fn instability(&self, body: &str) -> Option<String> {
        let again = self.canonicalize(body);
        if again != body {
            return Some(format!("再次归一后不同：{again}"));
        }
        let round_trip = self.canonicalize(&self.to_traditional.convert(body));
        if round_trip != body {
            return Some(format!("经繁体往返后不同：{round_trip}"));
        }
        None
    }

    /// 从字典探测繁体往返可证的残余异体映射，并合入补充表。
    fn derive_residual_variants(&self) -> BTreeMap<char, char> {
        let mut residual = BTreeMap::new();
        for character in cjk_candidates() {
            let source = character.to_string();
            if self.to_simplified.convert(&source) != source {
                continue;
            }
            let round_trip = self
                .to_simplified
                .convert(&self.to_traditional.convert(&source));
            let Some(target) = single_char(&round_trip) else {
                continue;
            };
            if target == character {
                continue;
            }
            // 目标字自己必须还有一个不同的繁体对应，这是它位于简体一侧的证据。
            // 少了这一条就会收进 `昵 -> 暱` 这种反向映射：`t2s` 不认识「暱」，
            // 于是往返把简体的「昵」推成了繁体的「暱」。
            let target_text = target.to_string();
            if self.to_traditional.convert(&target_text) == target_text {
                continue;
            }
            residual.insert(character, target);
        }
        for (source, target) in VARIANT_SUPPLEMENT {
            residual.insert(source, target);
        }
        residual
    }

    /// 字级候选全表：`t2s` 直接映射并上残余异体表。
    fn derive_candidates(&self) -> BTreeMap<char, char> {
        let mut candidates = BTreeMap::new();
        for character in cjk_candidates() {
            let source = character.to_string();
            let converted = self.to_simplified.convert(&source);
            if converted == source {
                continue;
            }
            match single_char(&converted) {
                Some(target) => {
                    candidates.insert(character, target);
                }
                None => {
                    // 一对多映射在字级表里无法表达。计数交给 `normalize`，那里才
                    // 知道该候选是否与本次语料相关。
                }
            }
        }
        for (source, target) in &self.residual {
            candidates.insert(*source, *target);
        }
        candidates
    }

    /// 对整批规范记录做归一，同时产出 `variant_map` 与发现列表。
    pub fn normalize(&self, records: &[CanonicalRecord]) -> Result<NormalizeOutcome> {
        let mut normalized = Vec::with_capacity(records.len());
        let mut findings = Vec::new();
        let mut indexed_chars = BTreeSet::new();

        for record in records {
            let body_lines: Vec<String> = record
                .body_lines
                .iter()
                .map(|line| self.canonicalize(line))
                .collect();
            let body = body_lines.join("\n");
            if record.body_original != record.body_lines.join("\n") {
                return Err(corpus_error(format!(
                    "body_original 与 body_lines 不一致：{}",
                    record.stable_id
                )));
            }
            indexed_chars.extend(body.chars());
            if let Some(detail) = self.instability(&body) {
                findings.push(NormalizationFinding {
                    stable_id: record.stable_id.clone(),
                    reason: NormalizationReason::ConversionUnstable,
                    detail,
                });
            }
            normalized.push(NormalizedRecord {
                stable_id: record.stable_id.clone(),
                body,
                body_lines,
                body_original: record.body_original.clone(),
                script: record.script,
            });
        }

        let (variant_map, stats) = self.build_variant_map(&indexed_chars);
        Ok(NormalizeOutcome {
            records: normalized,
            variant_map,
            stats,
            findings,
        })
    }

    /// 用归一后语料出现的字裁剪候选表。
    fn build_variant_map(&self, indexed_chars: &BTreeSet<char>) -> (VariantMap, VariantMapStats) {
        let mut rows = BTreeMap::new();
        let mut stats = VariantMapStats {
            candidates: self.candidates.len(),
            ..VariantMapStats::default()
        };
        for (source, target) in &self.candidates {
            if !indexed_chars.contains(target) {
                stats.dropped_target_absent += 1;
                continue;
            }
            if indexed_chars.contains(source) {
                stats.dropped_source_survives += 1;
                continue;
            }
            rows.insert(*source, *target);
        }
        stats.dropped_not_single_char = self.multi_char_candidate_count();
        stats.rows = rows.len();
        (VariantMap { rows }, stats)
    }

    /// 转换结果不是单字、因而进不了字级表的候选数。
    fn multi_char_candidate_count(&self) -> usize {
        cjk_candidates()
            .filter(|character| {
                let source = character.to_string();
                let converted = self.to_simplified.convert(&source);
                converted != source && single_char(&converted).is_none()
            })
            .count()
    }
}

/// 待探测的候选码位。
fn cjk_candidates() -> impl Iterator<Item = char> {
    CJK_BLOCKS
        .into_iter()
        .flat_map(|(low, high)| low..=high)
        .filter_map(char::from_u32)
}

/// 恰好一个字符时返回它，否则返回 `None`。
fn single_char(text: &str) -> Option<char> {
    let mut characters = text.chars();
    let first = characters.next()?;
    characters.next().is_none().then_some(first)
}
