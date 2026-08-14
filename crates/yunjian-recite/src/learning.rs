//! 背诵学习对象：整首汇总、联片排程与音步练习引用。

use std::collections::BTreeMap;
use std::ops::Range;

use crate::{FsrsGrade, ReviewState};
use yunjian_core::{content_chars, split_metrical_lines};

/// 当前联片算法版本。版本进入联片稳定身份，算法变更时必须递增。
pub const SEGMENTATION_VERSION: u32 = 1;

/// 一首作品的汇总对象；它不进入 FSRS。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WholePoem {
    /// 语料中的作品稳定标识。
    pub poem_id: String,
}

impl WholePoem {
    /// 整首只汇总进度，不创建 FSRS 卡。
    #[must_use]
    pub const fn enters_fsrs(&self) -> bool {
        false
    }
}

/// 一个可独立回忆和排程的联片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningChunk {
    /// `poem_id + segmentation_version + line_range` 组成的稳定身份。
    pub stable_id: String,
    /// 所属作品。
    pub poem_id: String,
    /// 分段算法版本。
    pub segmentation_version: u32,
    /// 韵律行半开区间。
    pub line_range: Range<usize>,
    /// 保留原句读的联片正文。
    pub body: String,
}

impl LearningChunk {
    /// 只有联片承载 FSRS 记忆判断。
    #[must_use]
    pub const fn enters_fsrs(&self) -> bool {
        true
    }

    /// 构造一个音步练习引用。真实音步边界仍由 voice crate 的 `FootMark` 提供。
    #[must_use]
    pub fn foot(&self, foot_index: usize) -> FootPracticeRef {
        FootPracticeRef {
            stable_id: format!("{}:foot:{foot_index}", self.stable_id),
            chunk_id: self.stable_id.clone(),
            foot_index,
        }
    }
}

/// 音步层的示范/局部练习引用；它不进入 FSRS。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootPracticeRef {
    /// 供练习事件引用的稳定标识。
    pub stable_id: String,
    /// 所属联片稳定标识。
    pub chunk_id: String,
    /// 联片内音步序号。
    pub foot_index: usize,
}

impl FootPracticeRef {
    /// 音步是动作支架，不创建 FSRS 卡。
    #[must_use]
    pub const fn enters_fsrs(&self) -> bool {
        false
    }
}

/// 一首作品确定性派生出的三层学习对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningObjects {
    /// 整首汇总对象。
    pub whole: WholePoem,
    /// 按朗读顺序排列的联片。
    pub chunks: Vec<LearningChunk>,
}

/// 最近一次不带文字提示的整首检查结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteRecitation {
    /// 整首检查通过。
    Passed {
        /// 发生时的 Unix 日序号。
        occurred_day: i64,
    },
    /// 整首检查未通过。
    Failed {
        /// 发生时的 Unix 日序号。
        occurred_day: i64,
    },
}

/// 作品页需要展示的掌握度；不以平均分隐藏薄弱片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterySummary {
    /// 已完成首次独立回忆的联片数。
    pub established: usize,
    /// 联片总数。
    pub total: usize,
    /// 当前到期或逾期联片数。
    pub due: usize,
    /// 最近等级为 Again/Hard 的联片稳定标识。
    pub weak_points: Vec<String>,
    /// 所有联片建立后最近一次整首检查结果。
    pub complete_recitation: Option<CompleteRecitation>,
    /// 仅在每片已建立且没有到期/逾期片时成立。
    pub currently_solid: bool,
}

/// 按版本化规则把作品构造成整首与联片对象。
#[must_use]
pub fn build_learning_objects(poem_id: &str, body: &str) -> LearningObjects {
    let lines = metrical_lines_with_punctuation(body);
    debug_assert_eq!(
        lines.len(),
        split_metrical_lines(body).count(),
        "保留句读的切分必须与核心韵律行口径一致"
    );
    let content_len = content_chars(body).count();
    let ranges = chunk_ranges(lines.len(), content_len);
    let chunks = ranges
        .into_iter()
        .map(|line_range| {
            let chunk_body = lines[line_range.clone()].concat();
            LearningChunk {
                stable_id: format!(
                    "{poem_id}:v{SEGMENTATION_VERSION}:{}-{}",
                    line_range.start, line_range.end
                ),
                poem_id: poem_id.to_owned(),
                segmentation_version: SEGMENTATION_VERSION,
                line_range,
                body: chunk_body,
            }
        })
        .collect();
    LearningObjects {
        whole: WholePoem {
            poem_id: poem_id.to_owned(),
        },
        chunks,
    }
}

/// 汇总联片状态，不计算会让强片抵消弱片的平均值。
#[must_use]
pub fn summarize_mastery(
    objects: &LearningObjects,
    states: &[ReviewState],
    today: i64,
    complete_recitation: Option<CompleteRecitation>,
) -> MasterySummary {
    let by_id = states
        .iter()
        .map(|state| (state.stable_id.as_str(), state))
        .collect::<BTreeMap<_, _>>();
    let mut established = 0;
    let mut due = 0;
    let mut weak_points = Vec::new();
    for chunk in &objects.chunks {
        let Some(state) = by_id.get(chunk.stable_id.as_str()) else {
            continue;
        };
        established += 1;
        if state.due_day <= today {
            due += 1;
        }
        if matches!(state.last_grade, FsrsGrade::Again | FsrsGrade::Hard) {
            weak_points.push(chunk.stable_id.clone());
        }
    }
    let total = objects.chunks.len();
    let all_established = established == total && total > 0;
    MasterySummary {
        established,
        total,
        due,
        weak_points,
        complete_recitation: all_established.then_some(complete_recitation).flatten(),
        currently_solid: all_established && due == 0,
    }
}

fn chunk_ranges(line_count: usize, content_len: usize) -> Vec<Range<usize>> {
    if line_count == 0 {
        return Vec::new();
    }
    if line_count == 4 && content_len <= 32 {
        return std::iter::once(0..4).collect();
    }
    if line_count <= 3 {
        return std::iter::once(0..line_count).collect();
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while start + 1 < line_count {
        ranges.push(start..start + 2);
        start += 2;
    }
    if start < line_count {
        if let Some(last) = ranges.last_mut() {
            last.end = line_count;
        } else {
            ranges.push(start..line_count);
        }
    }
    ranges
}

fn metrical_lines_with_punctuation(body: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, character) in body.char_indices() {
        if !matches!(character, '\n' | '，' | '。' | '！' | '？' | '；') {
            continue;
        }
        let end = if character == '\n' {
            index
        } else {
            index + character.len_utf8()
        };
        push_content_line(&mut lines, &body[start..end]);
        start = index + character.len_utf8();
    }
    push_content_line(&mut lines, &body[start..]);
    lines
}

fn push_content_line(lines: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim();
    if content_chars(candidate).next().is_some() {
        lines.push(candidate.to_owned());
    }
}
