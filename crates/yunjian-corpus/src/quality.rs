//! 重出分组与数据缺陷报告，以及一条**只建立在处置台账上**的守恒式。
//!
//! # 两个语义不同的工件
//!
//! - `corpus/reports/defects.json`：**一行一个 finding**。一条记录可以合法地
//!   产生多个 finding——同一首诗既是重出、又归属冲突、又长度可疑，就是三个。
//! - `corpus/reports/dispositions.json`：**一行一条输入记录**，取值只有
//!   [`Disposition::Shipped`] / [`Disposition::Quarantined`] /
//!   [`Disposition::Excluded`] 三种。
//!
//! # 守恒为什么不能建立在 finding 上
//!
//! 「保留下来的记录也会产生 finding」与「一条记录能产生三个 finding」这两件事
//! 同时成立，于是 `poem_count + defect_count == 输入行数` **在算术上就是假的**：
//! 左边把一条记录数了三次，右边只数一次。正确的不变量是逐输入处置台账：
//!
//! ```text
//! count(shipped) + count(quarantined) + count(excluded) == input_rows
//! poem_count == count(shipped)
//! ```
//!
//! 见 [`QualityReport::check_conservation`]。
//!
//! # 什么叫「一条 INPUT 记录」
//!
//! **一个可寻址的输入单元，即拥有唯一 `source_locator` 的一条候选记录，或一条
//! 被入库阶段挡下的记录。** 这个口径是刻意选的，因为另外两个看起来更自然的口径
//! 都不成立：
//!
//! - 它**不等于上游 JSON 数组元素数**。`幽梦影` 的一条上游条目会展开成 1 条正文
//!   加 N 条清人评语，各自有独立 locator；`蒙学` 一卷会展开成多篇。于是
//!   `input_records`（上游数组长度）小于产出条数，拿它当分母会让守恒式恒假。
//! - 它**不等于 CSV 行数**——Werneror 恰好相等，`chinese-poetry` 不相等，所以
//!   不能按来源各用一套口径再相加。
//!
//! 按 locator 计数则对两个来源都成立，且天然满足「一条输入恰好一行台账」。
//!
//! # 判重口径与 `work_group` 的关系（重要）
//!
//! [`CanonicalRecord::work_group`](crate::model::CanonicalRecord) 是**按原字形**
//! 算的身份分组键（`compute_work_group(body)`）。本阶段做重出与归属冲突检测时
//! 另算一个键：`compute_work_group(canonicalize(body))`，即先过
//! [`Normalizer::canonicalize`] 再算。理由与代价都写在
//! [`detection_group`] 上。工件里的 `work_group` 列是**这个检测键**。
//!
//! # 本阶段只报告，不编辑
//!
//! 不自动修正归属、不补缺字、不删除重出记录——互见/重出在诗词学里是真实现象，
//! 抹掉它就是抹掉信息。记录只被分组并标注。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use yunjian_core::{Error, Result};

use crate::ingest::werneror::{ExclusionReason, LossyField, WernerorOutcome};
use crate::ingest::{DefectReason, IngestOutcome};
use crate::model::{CanonicalRecord, Genre, LicenseClass, compute_work_group};
use crate::normalize::{NormalizationFinding, Normalizer};

/// 工件的 schema 版本。字段语义变化时递增，下游读到不认识的版本应硬失败。
pub const SCHEMA_VERSION: u32 = 1;

/// 正文少于这么多个汉字，不可能支撑韵脚推导与平仄分析。
pub const MIN_BODY_CJK_CHARS: usize = 4;

/// 诗/词/曲的单行汉字上限。
///
/// 超过这个长度几乎一定是上游丢了句读，整段挤成一行。韵脚推导读的是**行末字**，
/// 一行装下整首诗就等于把韵脚判到最后一个字上，错得不报错。散文体
/// （[`Genre::Fu`] / [`Genre::Wen`]）不受此限——那本来就是长句。
pub const MAX_VERSE_LINE_CJK_CHARS: usize = 64;

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 缺陷报告的原因码。
///
/// 同一个原因码可以既出现在**保留下来的记录**上（告警，处置仍是 `shipped`），
/// 又出现在**被挡下的记录**上（该记录的处置是 `quarantined` 或 `excluded`）。
/// 所以原因码不是处置的函数，两者必须分开存放——这正是本模块拆成两个工件的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    /// 上游把生僻 utf8mb4 字符替换成了半角 `?`，原字不可恢复。
    LossyChar,
    /// 繁简归一不是往返稳定的，可能踩到异体字缺口。
    ConversionUnstable,
    /// 同一作品分组里还有别的记录（互见/重出）。**不删除，只标注。**
    DuplicateInGroup,
    /// 同一作品分组里出现了不同作者。
    ConflictingAttribution,
    /// 正文长度不足以做韵律分析，或单行长到不可能是诗句。
    SuspectLength,
    /// 上游没给可归一的朝代标签。
    UnknownDynasty,
    /// 正文为空。
    EmptyBody,
    /// 按入库策略排除（近现代分桶、整文件现代内容、分桶标签不符等）。
    ExcludedByPolicy,
    /// 许可受限，不进可分发集合。
    RestrictedLicense,
    /// 韵脚没有唯一韵部，或尾字未见于适用韵书。
    RhymeUnresolved,
}

impl ReasonCode {
    /// 全部原因码。基线文件必须逐条覆盖它，缺一条即失败。
    pub const ALL: [Self; 10] = [
        Self::LossyChar,
        Self::ConversionUnstable,
        Self::DuplicateInGroup,
        Self::ConflictingAttribution,
        Self::SuspectLength,
        Self::UnknownDynasty,
        Self::EmptyBody,
        Self::ExcludedByPolicy,
        Self::RestrictedLicense,
        Self::RhymeUnresolved,
    ];

    /// 工件里的字面值。与 serde 的 `snake_case` 重命名逐字一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LossyChar => "lossy_char",
            Self::ConversionUnstable => "conversion_unstable",
            Self::DuplicateInGroup => "duplicate_in_group",
            Self::ConflictingAttribution => "conflicting_attribution",
            Self::SuspectLength => "suspect_length",
            Self::UnknownDynasty => "unknown_dynasty",
            Self::EmptyBody => "empty_body",
            Self::ExcludedByPolicy => "excluded_by_policy",
            Self::RestrictedLicense => "restricted_license",
            Self::RhymeUnresolved => "rhyme_unresolved",
        }
    }

    /// 默认容差百分比。
    ///
    /// `restricted_license` 与 `excluded_by_policy` 是 **0%**：前者只要出现一条
    /// 就说明受限内容漏进了流水线，后者是显式声明的策略排除数，上游新增一个近
    /// 现代分桶就该让构建停下来让人看一眼，而不是被 10% 容差吞掉。
    pub const fn default_tolerance_pct(self) -> u32 {
        match self {
            Self::RestrictedLicense | Self::ExcludedByPolicy => 0,
            _ => 10,
        }
    }
}

/// 逐输入记录的处置。**守恒断言只建立在这三个取值上。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// 进入可分发集合。
    Shipped,
    /// 留档待复核，不进主表（缺字隔离、许可受限）。
    Quarantined,
    /// 按策略拒收（近现代分桶、整文件现代内容、已被别的来源收录）。
    Excluded,
}

impl Disposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::Quarantined => "quarantined",
            Self::Excluded => "excluded",
        }
    }
}

/// `defects.json` 的一行：**一个 finding**，不是一条记录。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// 身份铸造之前就被挡下的输入单元没有 `stable_id`，故可空。
    pub stable_id: Option<String>,
    /// 判重口径下的作品分组键，见 [`detection_group`]。缺字记录不参与分组，故可空。
    pub work_group: Option<String>,
    pub reason_code: ReasonCode,
    pub detail: String,
    pub source: String,
}

/// `dispositions.json` 的一行：**恰好一条输入记录**。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispositionRow {
    pub source_locator: String,
    /// 仅在「入 id 之前就被排除」时为空。带着它是为了让跨报告完整性检查能直接
    /// join 两个文件，而不必绕注册表。
    pub stable_id: Option<String>,
    pub disposition: Disposition,
}

/// 入库阶段挡下的一个输入单元：有 locator，还没有 `stable_id`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedUnit {
    pub source: String,
    pub source_locator: String,
    /// 只允许 `Quarantined` 或 `Excluded`；填 `Shipped` 会被 [`analyze`] 硬拒。
    pub disposition: Disposition,
    pub reason: ReasonCode,
    pub detail: String,
    pub work_group: Option<String>,
}

/// 质量阶段的全部输入。
///
/// 刻意收成一个结构而不是一长串参数：`input_rows` 是这三段的长度之和，一处算出
/// 就再不重算，于是「台账漏了一行」在算术上无处可藏。
#[derive(Debug, Clone, Default)]
pub struct QualityInput<'a> {
    /// 已铸造身份、进入可分发集合的记录。
    pub shippable: &'a [CanonicalRecord],
    /// 已铸造身份但许可受限的记录，处置为 `quarantined`。
    pub restricted: &'a [CanonicalRecord],
    /// 入库前被挡下的输入单元。
    pub blocked: Vec<BlockedUnit>,
    /// 归一阶段的 finding，按 `stable_id` 关联到已铸造身份的记录。
    pub normalization: &'a [NormalizationFinding],
}

/// 三类处置的计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispositionCounts {
    pub shipped: usize,
    pub quarantined: usize,
    pub excluded: usize,
}

impl DispositionCounts {
    pub const fn total(self) -> usize {
        self.shipped + self.quarantined + self.excluded
    }
}

/// 一次质量分析的产物。两个工件都从它序列化出去。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityReport {
    pub schema_version: u32,
    /// 输入单元总数。**在建台账之前就算好**，所以它是台账的外部参照而不是回声。
    pub input_rows: usize,
    /// 可分发记录数。恒等于 `counts.shipped`。
    pub poem_count: usize,
    pub counts: DispositionCounts,
    /// 逐原因码的 **finding 计数**（不是记录数）。
    pub summary: BTreeMap<String, usize>,
    pub findings: Vec<Finding>,
    pub dispositions: Vec<DispositionRow>,
}

impl QualityReport {
    /// 逐原因码的 finding 数。
    pub fn finding_count(&self, reason: ReasonCode) -> usize {
        self.summary.get(reason.as_str()).copied().unwrap_or(0)
    }

    /// 带某个原因码的 finding。
    pub fn findings_with(&self, reason: ReasonCode) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.reason_code == reason)
            .collect()
    }

    /// 某条记录（按 `stable_id`）身上的全部 finding。一条记录带多个是正常的。
    pub fn findings_for(&self, stable_id: &str) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.stable_id.as_deref() == Some(stable_id))
            .collect()
    }

    pub fn extend_findings(&mut self, additions: impl IntoIterator<Item = Finding>) -> Result<()> {
        let additions = additions.into_iter().collect::<Vec<_>>();
        let known_ids = self
            .dispositions
            .iter()
            .filter_map(|row| row.stable_id.as_deref())
            .collect::<BTreeSet<_>>();
        if let Some(stable_id) = additions
            .iter()
            .filter_map(|finding| finding.stable_id.as_deref())
            .find(|stable_id| !known_ids.contains(stable_id))
        {
            return Err(corpus_error(format!(
                "新增 finding 指向未知 stable_id：{stable_id}"
            )));
        }
        for finding in additions {
            *self
                .summary
                .entry(finding.reason_code.as_str().to_owned())
                .or_default() += 1;
            self.findings.push(finding);
        }
        self.findings.sort();
        self.check_cross_report_integrity()
    }

    /// **守恒检查，只看处置台账。**
    ///
    /// 三条同时成立才算通过：
    ///
    /// 1. `count(dispositions) == input_rows`——一条输入恰好一行台账；
    /// 2. `shipped + quarantined + excluded == count(dispositions)`；
    /// 3. `poem_count == shipped`。
    ///
    /// 这里刻意**不**去比 finding 数：一条记录能带三个 finding，任何把 finding
    /// 计入守恒的写法都会在多 finding 记录上崩掉。
    pub fn check_conservation(&self) -> Result<()> {
        if self.dispositions.len() != self.input_rows {
            return Err(corpus_error(format!(
                "处置台账守恒失败：台账 {} 行，输入 {} 条。\
                 有输入既没进可分发集合、也没被隔离或排除，却没有留下台账行——\
                 记录静默消失了。",
                self.dispositions.len(),
                self.input_rows
            )));
        }
        if self.counts.total() != self.dispositions.len() {
            return Err(corpus_error(format!(
                "三类处置之和 {} 与台账行数 {} 不符",
                self.counts.total(),
                self.dispositions.len()
            )));
        }
        if self.poem_count != self.counts.shipped {
            return Err(corpus_error(format!(
                "poem_count {} 与 shipped {} 不符",
                self.poem_count, self.counts.shipped
            )));
        }
        Ok(())
    }

    /// 跨报告完整性：`defects.json` 里每个非空 `stable_id` 都必须出现在
    /// `dispositions.json`。
    ///
    /// 这是一次直接的 join，join 列就是两个文件都带的 `stable_id`——不必绕注册表。
    /// 空值不参与 join：那是「入 id 之前就被排除」的合法情形。
    pub fn check_cross_report_integrity(&self) -> Result<()> {
        let known: BTreeSet<&str> = self
            .dispositions
            .iter()
            .filter_map(|row| row.stable_id.as_deref())
            .collect();
        let orphans: BTreeSet<&str> = self
            .findings
            .iter()
            .filter_map(|finding| finding.stable_id.as_deref())
            .filter(|id| !known.contains(id))
            .collect();
        if orphans.is_empty() {
            return Ok(());
        }
        Err(corpus_error(format!(
            "defects.json 里有 {} 个 stable_id 不在 dispositions.json：{}。\
             缺陷指向了一条没有处置的记录，两份报告已经不是同一次构建的产物。",
            orphans.len(),
            orphans.into_iter().take(10).collect::<Vec<_>>().join("、")
        )))
    }
}

/// 判重与归属冲突检测用的作品分组键。
///
/// 口径是 `compute_work_group(canonicalize(body))`，比
/// [`CanonicalRecord::work_group`](crate::model::CanonicalRecord)（按原字形算）
/// **更宽**。这么选的理由与代价：
///
/// - **理由**：`chinese-poetry/全唐诗` 是繁体、Werneror 是简体，同一首诗按原字形
///   算键会分属两组，一条重出都判不出来；`餘/馀` 这类异体也同理。检测键不归一，
///   `duplicate_in_group` 这个原因码就基本不会命中，等于白设。
/// - **代价**：它与记录上存的 `work_group` 是**两个不同的键**，下游不得假设相等。
///   工件里的 `work_group` 列一律是这个检测键。
/// - **为什么不直接改 `CanonicalRecord::work_group`**：那是身份语义，改它会改变
///   已入库记录的分组键取值。虽然 `stable_id` 锚在 `source_locator` 上、不受影响，
///   但身份语义变更该单独论证，不该作为缺陷报告的副作用发生。
pub fn detection_group(normalizer: &Normalizer, record: &CanonicalRecord) -> String {
    compute_work_group(&normalizer.canonicalize(&record.body_lines.join("\n")))
}

fn cjk_char_count(text: &str) -> usize {
    text.chars().filter(|c| is_cjk(*c)).count()
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32,
        0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x20000..=0x2FA1F)
}

/// 把入库阶段的 [`DefectReason`] 折到本阶段的原因码与处置上。
///
/// 返回 `None` 的三个平仄类原因是**告警而非处置**：它们挂在已经入库的记录上，
/// 折成被挡下的输入单元会把同一条记录数两遍，正好是本阶段最该避免的错误。
const fn map_defect(reason: DefectReason) -> Option<(ReasonCode, Disposition)> {
    match reason {
        DefectReason::MissingBody => Some((ReasonCode::EmptyBody, Disposition::Excluded)),
        DefectReason::UnknownDynasty => Some((ReasonCode::UnknownDynasty, Disposition::Excluded)),
        DefectReason::ModernCommentaryInseparable | DefectReason::BucketLabelMismatch => {
            Some((ReasonCode::ExcludedByPolicy, Disposition::Excluded))
        }
        DefectReason::LossyCharacter => Some((ReasonCode::LossyChar, Disposition::Quarantined)),
        DefectReason::DuplicateInOtherSource => {
            Some((ReasonCode::DuplicateInGroup, Disposition::Excluded))
        }
        DefectReason::StrainsMisaligned
        | DefectReason::StrainsUnavailable
        | DefectReason::StrainsLineMismatch => None,
    }
}

/// 由 `chinese-poetry` 的入库产物折出被挡下的输入单元。
///
/// locator 用位置形态 `chinese-poetry:<相对路径>:<序号>`。这与入库器给位置型
/// 记录发的 locator 同构，且不会撞车：入库器在一个文件内，同一个序号要么发给一条
/// 产出记录、要么记一条缺陷，从不两者兼有（见 `ingest_youmengying` 的序号推进）。
/// 原生键资产的产出记录走 `chinese-poetry:<uuid>`，与位置形态天然不同名空间。
///
/// 整文件排除（`ModernCommentaryInseparable`）按该文件的上游条目数逐条展开；
/// 条目数不可数时（上游是对象而非数组）整文件记一个单元。
pub fn blocked_from_chinese_poetry(outcome: &IngestOutcome) -> Vec<BlockedUnit> {
    let source = crate::ingest::chinese_poetry::SOURCE_NAME;
    let mut units = Vec::new();
    for defect in &outcome.defects {
        let Some((reason, disposition)) = map_defect(defect.reason) else {
            continue;
        };
        if defect.reason == DefectReason::ModernCommentaryInseparable {
            let rows = outcome
                .tally(&defect.relative_path)
                .map_or(0, |tally| tally.input_records)
                .max(1);
            for ordinal in 0..rows {
                units.push(BlockedUnit {
                    source: source.to_owned(),
                    source_locator: format!("{source}:{}:{ordinal}", defect.relative_path),
                    disposition,
                    reason,
                    detail: defect.detail.clone(),
                    work_group: None,
                });
            }
            continue;
        }
        units.push(BlockedUnit {
            source: source.to_owned(),
            source_locator: format!("{source}:{}:{}", defect.relative_path, defect.ordinal),
            disposition,
            reason,
            detail: defect.detail.clone(),
            work_group: None,
        });
    }
    units
}

/// 由 Werneror 的入库产物折出被挡下的输入单元。
///
/// 四条来源，逐条对应一行 CSV，互不重叠：
///
/// 1. `quarantined`——缺字隔离。**同时**会有一条 `LossyCharacter` 缺陷指向同一行，
///    所以缺陷那边必须跳过它，否则同一行会被数两遍。
/// 2. `duplicates`——`chinese-poetry` 已收录，按逐来源取舍不重复入库。
/// 3. `defects`（去掉 `LossyCharacter`）——分桶标签不符、正文为空、朝代无法归一。
/// 4. `exclusions`——整个分桶被策略排除，按该文件的数据行数逐行展开。
pub fn blocked_from_werneror(outcome: &WernerorOutcome) -> Vec<BlockedUnit> {
    let source = crate::ingest::werneror::SOURCE_NAME;
    let mut units = Vec::new();

    for record in &outcome.quarantined {
        let fields = record
            .lossy_fields
            .iter()
            .map(|field| match field {
                LossyField::Title => "题目",
                LossyField::Author => "作者",
                LossyField::Body => "内容",
            })
            .collect::<Vec<_>>()
            .join("、");
        units.push(BlockedUnit {
            source: source.to_owned(),
            source_locator: record.source_locator.clone(),
            disposition: Disposition::Quarantined,
            reason: ReasonCode::LossyChar,
            detail: format!(
                "《{}》的 {fields} 含 CJK 上下文中的半角 `?`，原字不可恢复；\
                 整条隔离留档，不进主表也不自动补字",
                record.title_raw
            ),
            work_group: None,
        });
    }

    for record in &outcome.duplicates {
        units.push(BlockedUnit {
            source: source.to_owned(),
            source_locator: record.source_locator.clone(),
            disposition: Disposition::Excluded,
            reason: ReasonCode::DuplicateInGroup,
            detail: format!(
                "《{}》与 chinese-poetry 已收录作品同组（判重键 {}），\
                 按逐来源取舍不重复入库",
                record.title_raw, record.work_group
            ),
            work_group: Some(record.work_group.clone()),
        });
    }

    for defect in &outcome.defects {
        if defect.reason == DefectReason::LossyCharacter {
            continue;
        }
        let Some((reason, disposition)) = map_defect(defect.reason) else {
            continue;
        };
        units.push(BlockedUnit {
            source: source.to_owned(),
            source_locator: format!("{source}:{}:{}", defect.relative_path, defect.ordinal),
            disposition,
            reason,
            detail: defect.detail.clone(),
            work_group: None,
        });
    }

    for exclusion in &outcome.exclusions {
        let label = match exclusion.reason {
            ExclusionReason::ModernAuthorsLikelyInCopyright => "已知近现代/当代分桶，保护期未过",
            ExclusionReason::NotOnClassicalAllowList => "不在古典朝代白名单上",
        };
        for ordinal in 0..exclusion.rows {
            units.push(BlockedUnit {
                source: source.to_owned(),
                source_locator: format!("{source}:{}:{ordinal}", exclusion.file),
                disposition: Disposition::Excluded,
                reason: ReasonCode::ExcludedByPolicy,
                detail: format!("{}：{label}（{}）", exclusion.file, exclusion.detail),
                work_group: None,
            });
        }
    }

    units
}

struct GroupMember<'a> {
    record: &'a CanonicalRecord,
    source: &'a str,
}

/// 跑一遍质量分析，产出处置台账与缺陷清单。
///
/// `normalizer` 由调用方传入而不是内部新建：一次 `OpenCC` 装载实测 1.7 ms，
/// 逐记录新建会把一次全量构建变成几十万次字典装载；而归一阶段本来就有一个实例。
pub fn analyze(input: &QualityInput<'_>, normalizer: &Normalizer) -> Result<QualityReport> {
    // 先算输入数，再建台账。顺序是有意义的：input_rows 必须是台账的外部参照，
    // 而不是从台账回读出来的同一个数——否则守恒式恒真，什么也证明不了。
    let input_rows = input.shippable.len() + input.restricted.len() + input.blocked.len();

    let mut dispositions: Vec<DispositionRow> = Vec::with_capacity(input_rows);
    let mut findings: Vec<Finding> = Vec::new();
    let mut counts = DispositionCounts::default();

    // 已铸造身份的记录：可分发的与许可受限的。
    let mut members: BTreeMap<String, Vec<GroupMember<'_>>> = BTreeMap::new();
    let mut group_of: BTreeMap<&str, String> = BTreeMap::new();
    let mut source_of: BTreeMap<&str, &str> = BTreeMap::new();

    for (records, disposition) in [
        (input.shippable, Disposition::Shipped),
        (input.restricted, Disposition::Quarantined),
    ] {
        for record in records {
            if disposition == Disposition::Shipped
                && record.provenance.license_class == LicenseClass::Restricted
            {
                return Err(corpus_error(format!(
                    "许可受限的记录 {} 出现在可分发集合里；受限记录只能是 quarantined",
                    record.stable_id
                )));
            }
            let group = detection_group(normalizer, record);
            group_of.insert(record.stable_id.as_str(), group.clone());
            source_of.insert(
                record.stable_id.as_str(),
                record.provenance.source_name.as_str(),
            );
            members.entry(group).or_default().push(GroupMember {
                record,
                source: record.provenance.source_name.as_str(),
            });
            dispositions.push(DispositionRow {
                source_locator: record.source_locator.clone(),
                stable_id: Some(record.stable_id.clone()),
                disposition,
            });
            match disposition {
                Disposition::Shipped => counts.shipped += 1,
                Disposition::Quarantined => counts.quarantined += 1,
                Disposition::Excluded => counts.excluded += 1,
            }
            if disposition == Disposition::Quarantined {
                findings.push(Finding {
                    stable_id: Some(record.stable_id.clone()),
                    work_group: group_of.get(record.stable_id.as_str()).cloned(),
                    reason_code: ReasonCode::RestrictedLicense,
                    detail: format!(
                        "许可类别 restricted（{}），留档不分发",
                        record.provenance.license
                    ),
                    source: record.provenance.source_name.clone(),
                });
            }
        }
    }

    // 被挡下的输入单元：一行台账，一条 finding。
    for unit in &input.blocked {
        if unit.disposition == Disposition::Shipped {
            return Err(corpus_error(format!(
                "被挡下的输入单元 {} 的处置写成了 shipped；\
                 挡下的记录只能是 quarantined 或 excluded",
                unit.source_locator
            )));
        }
        dispositions.push(DispositionRow {
            source_locator: unit.source_locator.clone(),
            stable_id: None,
            disposition: unit.disposition,
        });
        match unit.disposition {
            Disposition::Shipped => counts.shipped += 1,
            Disposition::Quarantined => counts.quarantined += 1,
            Disposition::Excluded => counts.excluded += 1,
        }
        findings.push(Finding {
            stable_id: None,
            work_group: unit.work_group.clone(),
            reason_code: unit.reason,
            detail: unit.detail.clone(),
            source: unit.source.clone(),
        });
    }

    // locator 唯一性。重复的 locator 会让台账行数看起来对得上，实际却把两条输入
    // 折成一条，守恒式发现不了——所以它必须是独立的一道硬失败。
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for row in &dispositions {
        if !seen.insert(row.source_locator.as_str()) {
            return Err(corpus_error(format!(
                "处置台账里 source_locator 重复：{}。\
                 一条输入只能有一行台账，重复会让守恒式漏掉丢失的记录。",
                row.source_locator
            )));
        }
    }

    // 归一阶段的 finding，按 stable_id join。
    for finding in input.normalization {
        let Some(group) = group_of.get(finding.stable_id.as_str()) else {
            return Err(corpus_error(format!(
                "归一 finding 指向的 stable_id {} 不在本次记录集合里",
                finding.stable_id
            )));
        };
        findings.push(Finding {
            stable_id: Some(finding.stable_id.clone()),
            work_group: Some(group.clone()),
            reason_code: ReasonCode::ConversionUnstable,
            detail: finding.detail.clone(),
            source: source_of
                .get(finding.stable_id.as_str())
                .copied()
                .unwrap_or_default()
                .to_owned(),
        });
    }

    // 分组类 finding：重出与归属冲突。**一条记录一个 finding**，同组两条就是两条
    // finding，所以「记录数」与「finding 数」在这里必然分道扬镳。
    for (group, group_members) in &members {
        if group_members.len() < 2 {
            continue;
        }
        let authors: BTreeSet<&str> = group_members
            .iter()
            .map(|member| member.record.author.as_str())
            .collect();
        let locators = group_members
            .iter()
            .map(|member| member.record.source_locator.as_str())
            .collect::<Vec<_>>()
            .join("、");
        for member in group_members {
            findings.push(Finding {
                stable_id: Some(member.record.stable_id.clone()),
                work_group: Some(group.clone()),
                reason_code: ReasonCode::DuplicateInGroup,
                detail: format!(
                    "《{}》与同组另 {} 条记录重出（互见），全部保留只作标注；同组 locator：{locators}",
                    member.record.title,
                    group_members.len() - 1
                ),
                source: member.source.to_owned(),
            });
        }
        if authors.len() > 1 {
            let names = authors.iter().copied().collect::<Vec<_>>().join("、");
            for member in group_members {
                findings.push(Finding {
                    stable_id: Some(member.record.stable_id.clone()),
                    work_group: Some(group.clone()),
                    reason_code: ReasonCode::ConflictingAttribution,
                    detail: format!(
                        "《{}》同一作品分组内出现 {} 个作者：{names}。\
                         本阶段不自动裁定归属，只报告。",
                        member.record.title,
                        authors.len()
                    ),
                    source: member.source.to_owned(),
                });
            }
        }
    }

    // 刻意没有 `unknown_dynasty`：`model::prepare` 在铸造身份前就要求 `dynasty_raw`
    // 可归一，故带 `stable_id` 的记录不可能缺朝代标签，该码只能来自入库阶段。
    for records in [input.shippable, input.restricted] {
        for record in records {
            let group = group_of.get(record.stable_id.as_str()).cloned();
            let body = record.body_lines.join("\n");
            let total = cjk_char_count(&body);
            if record.body_lines.is_empty() || body.trim().is_empty() {
                findings.push(Finding {
                    stable_id: Some(record.stable_id.clone()),
                    work_group: group.clone(),
                    reason_code: ReasonCode::EmptyBody,
                    detail: format!("《{}》正文为空却进入了记录集合", record.title),
                    source: record.provenance.source_name.clone(),
                });
            } else if total < MIN_BODY_CJK_CHARS {
                findings.push(Finding {
                    stable_id: Some(record.stable_id.clone()),
                    work_group: group.clone(),
                    reason_code: ReasonCode::SuspectLength,
                    detail: format!(
                        "《{}》正文只有 {total} 个汉字，少于 {MIN_BODY_CJK_CHARS}，\
                         不足以做韵脚与平仄分析",
                        record.title
                    ),
                    source: record.provenance.source_name.clone(),
                });
            } else if matches!(record.genre, Genre::Shi | Genre::Ci | Genre::Qu)
                && let Some((index, line_len)) = record
                    .body_lines
                    .iter()
                    .map(|line| cjk_char_count(line))
                    .enumerate()
                    .find(|(_, len)| *len > MAX_VERSE_LINE_CJK_CHARS)
            {
                findings.push(Finding {
                    stable_id: Some(record.stable_id.clone()),
                    work_group: group.clone(),
                    reason_code: ReasonCode::SuspectLength,
                    detail: format!(
                        "《{}》第 {} 行有 {line_len} 个汉字，超过 {MAX_VERSE_LINE_CJK_CHARS}，\
                         疑似上游丢了句读；韵脚推导读行末字会因此判错",
                        record.title,
                        index + 1
                    ),
                    source: record.provenance.source_name.clone(),
                });
            }
        }
    }

    findings.sort();
    dispositions.sort();

    let mut summary: BTreeMap<String, usize> = ReasonCode::ALL
        .iter()
        .map(|reason| (reason.as_str().to_owned(), 0))
        .collect();
    for finding in &findings {
        *summary
            .entry(finding.reason_code.as_str().to_owned())
            .or_default() += 1;
    }

    Ok(QualityReport {
        schema_version: SCHEMA_VERSION,
        input_rows,
        poem_count: counts.shipped,
        counts,
        summary,
        findings,
        dispositions,
    })
}

/// 一次质量流水线运行的产物。
pub struct PipelineOutcome {
    pub report: QualityReport,
    pub shippable: Vec<CanonicalRecord>,
    pub restricted: Vec<CanonicalRecord>,
}

/// 走完整链路：两个来源入库 → 铸造身份 → 繁简归一 → 质量分析。
///
/// **只读**：不往 `corpus/id_registry.jsonl` 追加任何事件。质量阶段是一次观察，
/// 拿一次观察去写只追加的身份日志会把 fixture 规模的 id 永久钉进注册表。
///
/// `werneror_buckets` 只决定「读哪些分桶」，策略仍由
/// [`CLASSICAL_BUCKETS`](crate::ingest::werneror::CLASSICAL_BUCKETS) 全量决定。
pub fn run_pipeline(
    chinese_poetry_dir: &std::path::Path,
    werneror_dir: &std::path::Path,
    werneror_buckets: &[crate::ingest::werneror::Bucket],
    extra_records: Vec<crate::model::RecordInput>,
    corpus_version: &str,
) -> Result<PipelineOutcome> {
    let cp = crate::ingest::chinese_poetry::ingest(chinese_poetry_dir)?;
    let detector = crate::ingest::ScriptDetector::new()?;
    let covered = crate::ingest::werneror::CoveredWorks::from_records(&detector, &cp.records);
    let wr = crate::ingest::werneror::ingest_buckets(werneror_dir, werneror_buckets, &covered)?;

    let mut inputs = cp.records.clone();
    inputs.extend(wr.records.clone());
    inputs.extend(extra_records);
    let rebuilt = crate::model::rebuild_corpus(
        &crate::model::RegistryState::default(),
        &[],
        inputs,
        corpus_version,
        &[],
    )?;

    let normalizer = Normalizer::new()?;
    let mut all = rebuilt.shippable_records.clone();
    all.extend(rebuilt.restricted_records.clone());
    let normalized = normalizer.normalize(&all)?;

    let mut blocked = blocked_from_chinese_poetry(&cp);
    blocked.extend(blocked_from_werneror(&wr));

    let report = analyze(
        &QualityInput {
            shippable: &rebuilt.shippable_records,
            restricted: &rebuilt.restricted_records,
            blocked,
            normalization: &normalized.findings,
        },
        &normalizer,
    )?;
    Ok(PipelineOutcome {
        report,
        shippable: rebuilt.shippable_records,
        restricted: rebuilt.restricted_records,
    })
}

/// fixture 规模补充记录的来源名与 revision。
///
/// 刻意不冒用 `chinese-poetry` 的 `source_rev`：`chibi.json` 确实来自 全唐诗
/// （上游 issue #232），但 `multi_finding.json` 是合成的残句，把它们一并挂到真实
/// 上游 revision 上就是在 provenance 里写假话。
pub const SUPPLEMENT_SOURCE: &str = "quality-fixture";
pub const SUPPLEMENT_REV: &str = "committed-fixture";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupplementRecord {
    author: String,
    paragraphs: Vec<String>,
    title: String,
    id: String,
}

/// 读 `crates/yunjian-corpus/tests/fixtures/quality/` 下的补充候选记录。
///
/// 为什么需要它：随仓的两份上游 fixture 只能覆盖 `lossy_char`、`empty_body`、
/// `excluded_by_policy` 三个原因码——真实小样本里既没有重出、也没有归属冲突、
/// 也没有异体不稳定。一份 6 个码恒为 0 的基线**无法发现某个码不再命中**，所以
/// fixture 范围刻意补齐这几类，让基线两个方向的漂移都能被抓到。
///
/// 文件名排序读取：`read_dir` 的顺序未定义，不排序则工件不可逐字节复现。
pub fn load_supplement(dir: &std::path::Path) -> Result<Vec<crate::model::RecordInput>> {
    use crate::model::{Dynasty, Provenance, ProvenanceKind, RecordInput, SourceLocator};

    let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", dir.display())))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    names.sort();
    if names.is_empty() {
        return Err(corpus_error(format!(
            "{} 下没有补充记录；基线范围会静默缩水",
            dir.display()
        )));
    }

    let detector = crate::ingest::ScriptDetector::new()?;
    let mut inputs = Vec::new();
    for path in names {
        let records: Vec<SupplementRecord> = read_json(&path)?;
        for record in records {
            let body_original = record.paragraphs.join("\n");
            inputs.push(RecordInput {
                source_locator: SourceLocator::native(SUPPLEMENT_SOURCE, &record.id),
                genre: Genre::Shi,
                title: record.title.clone(),
                title_raw: record.title,
                author: record.author,
                dynasty: Dynasty::Tang,
                dynasty_raw: "唐".to_owned(),
                body_lines: record.paragraphs,
                script: detector.detect(&body_original),
                body_original,
                provenance: Provenance {
                    source_name: SUPPLEMENT_SOURCE.to_owned(),
                    source_rev: SUPPLEMENT_REV.to_owned(),
                    license: "MIT".to_owned(),
                    license_class: LicenseClass::PublicDomain,
                    kind: ProvenanceKind::Original,
                },
            });
        }
    }
    Ok(inputs)
}

/// 基线里的一条逐原因码约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineEntry {
    pub reason_code: ReasonCode,
    pub expected: usize,
    pub tolerance_pct: u32,
}

impl BaselineEntry {
    /// 允许区间。
    ///
    /// 容差按 `expected * tolerance_pct / 100` **整数下取整**算，所以计数小于 10
    /// 时区间会收缩成精确相等。这是刻意的：fixture 规模的基线本来就该精确，
    /// 一条新增的缺陷不应该被 10% 容差吞掉。
    pub const fn allowed(&self) -> (usize, usize) {
        let delta = self.expected * self.tolerance_pct as usize / 100;
        (self.expected.saturating_sub(delta), self.expected + delta)
    }
}

/// 一处基线漂移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub reason_code: ReasonCode,
    pub expected: usize,
    pub actual: usize,
    pub tolerance_pct: u32,
    pub allowed_min: usize,
    pub allowed_max: usize,
}

impl std::fmt::Display for Drift {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}：实测 {}，基线 {}（容差 {}%，允许 {}..={}）",
            self.reason_code.as_str(),
            self.actual,
            self.expected,
            self.tolerance_pct,
            self.allowed_min,
            self.allowed_max
        )
    }
}

/// 提交在 `corpus/reports/baseline.json` 的回归基线。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema_version: u32,
    /// 基线适用的输入范围。`fixtures` 指随仓提交的 fixture；换成真实检出时
    /// 计数会差几个数量级，届时这份基线不适用。
    pub scope: String,
    pub input_rows: usize,
    pub poem_count: usize,
    pub note: String,
    pub codes: Vec<BaselineEntry>,
}

impl Baseline {
    /// 由一次实测生成基线，逐原因码套默认容差。
    pub fn from_report(
        scope: impl Into<String>,
        note: impl Into<String>,
        report: &QualityReport,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            scope: scope.into(),
            input_rows: report.input_rows,
            poem_count: report.poem_count,
            note: note.into(),
            codes: ReasonCode::ALL
                .iter()
                .map(|reason| BaselineEntry {
                    reason_code: *reason,
                    expected: report.finding_count(*reason),
                    tolerance_pct: reason.default_tolerance_pct(),
                })
                .collect(),
        }
    }

    fn entry(&self, reason: ReasonCode) -> Option<&BaselineEntry> {
        self.codes.iter().find(|entry| entry.reason_code == reason)
    }

    /// 逐原因码算漂移。
    pub fn drift(&self, report: &QualityReport) -> Vec<Drift> {
        ReasonCode::ALL
            .iter()
            .filter_map(|reason| {
                let entry = self.entry(*reason)?;
                let actual = report.finding_count(*reason);
                let (allowed_min, allowed_max) = entry.allowed();
                (actual < allowed_min || actual > allowed_max).then_some(Drift {
                    reason_code: *reason,
                    expected: entry.expected,
                    actual,
                    tolerance_pct: entry.tolerance_pct,
                    allowed_min,
                    allowed_max,
                })
            })
            .collect()
    }

    /// 基线门禁。任一原因码超容差，或输入行数变化，即失败并**点名**。
    pub fn check(&self, report: &QualityReport) -> Result<()> {
        if self.schema_version != report.schema_version {
            return Err(corpus_error(format!(
                "基线 schema 版本 {} 与报告 {} 不符",
                self.schema_version, report.schema_version
            )));
        }
        let missing: Vec<&str> = ReasonCode::ALL
            .iter()
            .filter(|reason| self.entry(**reason).is_none())
            .map(|reason| reason.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(corpus_error(format!(
                "基线缺少原因码：{}。新增原因码必须显式定基线，不能默认放行。",
                missing.join("、")
            )));
        }
        if self.input_rows != report.input_rows {
            return Err(corpus_error(format!(
                "输入行数漂移：实测 {}，基线 {}。输入集合变了，逐码计数不可比。",
                report.input_rows, self.input_rows
            )));
        }
        let drifts = self.drift(report);
        if drifts.is_empty() {
            return Ok(());
        }
        let detail = drifts
            .iter()
            .map(Drift::to_string)
            .collect::<Vec<_>>()
            .join("；");
        Err(corpus_error(format!(
            "缺陷基线漂移，{} 个原因码超出容差：{detail}",
            drifts.len()
        )))
    }
}

/// `defects.json` 的文件封装：**一行一个 finding**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefectsFile {
    pub schema_version: u32,
    pub scope: String,
    /// 逐原因码的 finding 计数。QA 场景直接 `jq` 这个字段。
    pub summary: BTreeMap<String, usize>,
    pub findings: Vec<Finding>,
}

/// `dispositions.json` 的文件封装：**一行一条输入记录**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispositionsFile {
    pub schema_version: u32,
    pub scope: String,
    pub input_rows: usize,
    pub poem_count: usize,
    pub counts: DispositionCounts,
    pub rows: Vec<DispositionRow>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|error| corpus_error(format!("解析 {} 失败：{error}", path.display())))
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| corpus_error(format!("创建目录 {} 失败：{error}", dir.display())))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| corpus_error(format!("序列化 {} 失败：{error}", path.display())))?;
    std::fs::write(path, format!("{json}\n"))
        .map_err(|error| corpus_error(format!("写出 {} 失败：{error}", path.display())))
}

impl DefectsFile {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        read_json(path.as_ref())
    }
}

impl DispositionsFile {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        read_json(path.as_ref())
    }
}

impl Baseline {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self> {
        read_json(path.as_ref())
    }

    pub fn store(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        write_json(path.as_ref(), self)
    }
}

/// 三个工件的相对路径，相对仓库根。
pub const DEFECTS_JSON: &str = "corpus/reports/defects.json";
pub const DEFECTS_MD: &str = "corpus/reports/defects.md";
pub const DISPOSITIONS_JSON: &str = "corpus/reports/dispositions.json";
pub const BASELINE_JSON: &str = "corpus/reports/baseline.json";

/// 把一次分析写成三个文件（两个 JSON 工件 + 人类可读的 Markdown）。
///
/// 写盘前先跑守恒与跨报告完整性：**不允许把一份自相矛盾的报告落到磁盘上**，
/// 否则下游读到的是一份看起来正常的坏数据。
pub fn write_artifacts(
    root: impl AsRef<std::path::Path>,
    scope: &str,
    report: &QualityReport,
) -> Result<()> {
    report.check_conservation()?;
    report.check_cross_report_integrity()?;
    let root = root.as_ref();
    write_json(
        &root.join(DEFECTS_JSON),
        &DefectsFile {
            schema_version: report.schema_version,
            scope: scope.to_owned(),
            summary: report.summary.clone(),
            findings: report.findings.clone(),
        },
    )?;
    write_json(
        &root.join(DISPOSITIONS_JSON),
        &DispositionsFile {
            schema_version: report.schema_version,
            scope: scope.to_owned(),
            input_rows: report.input_rows,
            poem_count: report.poem_count,
            counts: report.counts,
            rows: report.dispositions.clone(),
        },
    )?;
    let markdown = render_defects_markdown(report);
    let path = root.join(DEFECTS_MD);
    std::fs::write(&path, markdown)
        .map_err(|error| corpus_error(format!("写出 {} 失败：{error}", path.display())))
}

/// 渲染人类可读的缺陷报告。
pub fn render_defects_markdown(report: &QualityReport) -> String {
    let mut out = String::new();
    out.push_str("# 语料缺陷报告\n\n");
    out.push_str(
        "本文件由 `cargo run -p xtask -- corpus-quality` 生成，**不要手改**。\n\n\
         两份工件的区别是这一阶段的全部要点：本文件与 `defects.json` 是\
         **一行一个 finding**，一条记录可以合法地带多个；`dispositions.json` 才是\
         **一行一条输入记录**。守恒只建立在后者上。\n\n",
    );
    let _ = writeln!(out, "## 处置台账\n");
    let _ = writeln!(out, "| 项 | 数 |");
    let _ = writeln!(out, "| --- | ---: |");
    let _ = writeln!(out, "| 输入记录数 | {} |", report.input_rows);
    let _ = writeln!(out, "| shipped（可分发） | {} |", report.counts.shipped);
    let _ = writeln!(
        out,
        "| quarantined（隔离留档） | {} |",
        report.counts.quarantined
    );
    let _ = writeln!(out, "| excluded（策略排除） | {} |", report.counts.excluded);
    let _ = writeln!(out, "| poem_count | {} |", report.poem_count);
    let _ = writeln!(
        out,
        "\n守恒式：`{} + {} + {} == {}`。\n",
        report.counts.shipped, report.counts.quarantined, report.counts.excluded, report.input_rows
    );
    let _ = writeln!(out, "## 逐原因码 finding 数\n");
    let _ = writeln!(out, "| 原因码 | finding 数 |");
    let _ = writeln!(out, "| --- | ---: |");
    for reason in ReasonCode::ALL {
        let _ = writeln!(
            out,
            "| `{}` | {} |",
            reason.as_str(),
            report.finding_count(reason)
        );
    }
    let _ = writeln!(
        out,
        "\n**finding 总数 {} 与记录数无关**，不要相加。\n",
        report.findings.len()
    );
    let _ = writeln!(out, "## 明细\n");
    for reason in ReasonCode::ALL {
        let items = report.findings_with(reason);
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### `{}`（{} 条）\n", reason.as_str(), items.len());
        let _ = writeln!(out, "| stable_id | work_group | source | 详情 |");
        let _ = writeln!(out, "| --- | --- | --- | --- |");
        for finding in items.iter().take(50) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} |",
                finding.stable_id.as_deref().unwrap_or("—"),
                finding.work_group.as_deref().unwrap_or("—"),
                finding.source,
                finding.detail.replace('|', "\\|").replace('\n', " ")
            );
        }
        if items.len() > 50 {
            let _ = writeln!(out, "\n（余 {} 条见 `defects.json`）\n", items.len() - 50);
        } else {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests;
