//! 相关作品的打分口径。
//!
//! # 为什么是固定权重加权和而不是 embedding
//!
//! 三条理由，任一条都足以否掉 embedding：
//!
//! 1. **可审计**。每条结果都回传四项分量，用户能逐项核对「为什么这两首算相似」。向量近邻
//!    给不出这个交代，只能给一个无从复核的数。
//! 2. **离线且确定**。不需要模型文件、不需要 API key、不需要网络；同一份语料上结果逐字节
//!    可复现。
//! 3. **不引入版权与出处问题**。embedding 模型的训练语料无从核实，而本项目的全部内容都必须
//!    能追到公有领域来源。
//!
//! # 四项权重
//!
//! | 项 | 权重 | 口径 |
//! |---|---:|---|
//! | 共享标签 | 0.40 | 标签集合的 Jaccard 相似度 |
//! | 同韵部 | 0.25 | 与本篇任一韵部归属同书 + 同韵部 + 同声调则记满，否则 0 |
//! | 同词牌 | 0.20 | 两侧都有词牌且相等则记满，否则 0 |
//! | 字面重叠 | 0.15 | 正文字集合的 Jaccard，**先排除语料里文档频率最高的 200 字** |
//!
//! 四项之和落在 `[0, 1]`。
//!
//! # 为什么字面重叠必须排除高频字
//!
//! 不排除的话，任意两首唐诗都会因为共享「不」「人」「山」「日」这类字而拿到一个虚高的
//! 重叠分——那不是相似，那是汉语的字频分布。排除口径是**文档频率**（出现在多少首作品里）
//! 而不是字频：一个字在单篇里重复十次不该让它变成停用字，出现在半个语料里才该。
//!
//! 高频字表**从语料自身算出**（见 `yunjian_core::frequent_content_chars`），不是写死的
//! 清单：语料换了，停用字表跟着换，不会出现「表针对唐诗、语料换成了宋词」这种错配。

use crate::schema::{SimilarityAxis, SimilarityComponents, SimilarityWeights};
use std::collections::BTreeSet;
use yunjian_core::{PoemFeatures, RhymeGroupMembership};

/// 共享标签项的权重。
pub const WEIGHT_SHARED_TAGS: f64 = 0.40;

/// 同韵部项的权重。
pub const WEIGHT_SAME_RHYME_GROUP: f64 = 0.25;

/// 同词牌项的权重。
pub const WEIGHT_SAME_CI_TUNE: f64 = 0.20;

/// 字面重叠项的权重。
pub const WEIGHT_CHARACTER_OVERLAP: f64 = 0.15;

/// 字面重叠项排除掉的高频字个数。
///
/// 200 是方案定的数。它的量级理由：常用字表的前 200 字已覆盖古典诗词正文的大部分出现次数，
/// 再往后排除就会开始删掉真正有区分度的意象字（「月」「雪」「舟」这类）。
pub const FREQUENT_CHAR_EXCLUSIONS: usize = 200;

/// 打分口径，供结果回传。
#[must_use]
pub const fn weights() -> SimilarityWeights {
    SimilarityWeights {
        shared_tags: WEIGHT_SHARED_TAGS,
        same_rhyme_group: WEIGHT_SAME_RHYME_GROUP,
        same_ci_tune: WEIGHT_SAME_CI_TUNE,
        character_overlap: WEIGHT_CHARACTER_OVERLAP,
    }
}

/// 打分方法的一句话说明，随结果回传。
pub const METHOD: &str = "固定权重加权和：共享标签 0.4 + 同韵部 0.25 + 同词牌 0.2 + \
     字面重叠 0.15（已排除语料里文档频率最高的 200 字）。可逐项复核，非 embedding 模型。";

/// 一侧参与比对的属性，已预先算好集合形态。
#[derive(Debug, Clone)]
pub struct Profile {
    tags: BTreeSet<String>,
    rhyme_keys: BTreeSet<String>,
    ci_tune: Option<String>,
    content_chars: BTreeSet<char>,
}

impl Profile {
    /// 从批量读出的属性集构建，`stopwords` 为要从字面重叠里排除的高频字。
    #[must_use]
    pub fn new(features: &PoemFeatures, stopwords: &BTreeSet<char>) -> Self {
        Self {
            tags: features.tags.iter().cloned().collect(),
            rhyme_keys: features.rhyme_groups.iter().map(rhyme_key).collect(),
            ci_tune: features.poem.ci_tune.clone(),
            content_chars: yunjian_core::content_chars(&features.poem.body)
                .filter(|character| !stopwords.contains(character))
                .collect(),
        }
    }

    /// 本篇落在哪些候选轴上与另一篇相关。
    #[must_use]
    pub fn axes_against(
        &self,
        other: &Self,
        author_matches: bool,
        dynasty_matches: bool,
    ) -> Vec<SimilarityAxis> {
        SimilarityAxis::all()
            .into_iter()
            .filter(|axis| match axis {
                SimilarityAxis::Theme => !self.tags.is_disjoint(&other.tags),
                SimilarityAxis::Rhyme => !self.rhyme_keys.is_disjoint(&other.rhyme_keys),
                SimilarityAxis::Tune => self.same_ci_tune(other),
                SimilarityAxis::Author => author_matches,
                SimilarityAxis::Dynasty => dynasty_matches,
            })
            .collect()
    }

    fn same_ci_tune(&self, other: &Self) -> bool {
        match (self.ci_tune.as_deref(), other.ci_tune.as_deref()) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }
}

/// 逐项打分。四项之和即 [`SimilarityComponents`] 求和后的得分。
#[must_use]
pub fn score(source: &Profile, candidate: &Profile) -> SimilarityComponents {
    SimilarityComponents {
        shared_tags: WEIGHT_SHARED_TAGS * jaccard(&source.tags, &candidate.tags),
        same_rhyme_group: if source.rhyme_keys.is_disjoint(&candidate.rhyme_keys) {
            0.0
        } else {
            WEIGHT_SAME_RHYME_GROUP
        },
        same_ci_tune: if source.same_ci_tune(candidate) {
            WEIGHT_SAME_CI_TUNE
        } else {
            0.0
        },
        character_overlap: WEIGHT_CHARACTER_OVERLAP
            * jaccard(&source.content_chars, &candidate.content_chars),
    }
}

/// 四项求和。
#[must_use]
pub fn total(components: &SimilarityComponents) -> f64 {
    components.shared_tags
        + components.same_rhyme_group
        + components.same_ci_tune
        + components.character_overlap
}

/// 两个集合的 Jaccard 相似度；两侧都空时为 `0.0` 而不是 `1.0`。
///
/// **空集不该相似。** 两首都没有标签只说明标签这条通路对它们没有信息，把它算成 1.0 会让
/// 「都缺数据」冒充「完全一致」——那正是虚高分数的经典来源。
fn jaccard<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    if intersection == 0 {
        return 0.0;
    }
    let union = left.len() + right.len() - intersection;
    #[expect(
        clippy::cast_precision_loss,
        reason = "集合基数远小于 f64 的精确整数上界（2^53）"
    )]
    {
        intersection as f64 / union as f64
    }
}

/// 韵部归属的比对键：**书、韵部、声调三者齐备**。
///
/// 声调必须进键。词林正韵的「第一部」同时有平声与仄声，平仄不同则不相押；只比韵部会把
/// 不相押的两首报成同韵。
fn rhyme_key(membership: &RhymeGroupMembership) -> String {
    format!(
        "{}/{}/{}",
        membership.book.as_key(),
        membership.group,
        membership.tone.as_key()
    )
}

#[cfg(test)]
mod tests;
