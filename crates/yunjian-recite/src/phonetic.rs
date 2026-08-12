//! 拼音级近音宽容层。
//!
//! 本层复核 [`crate::align`] 标为替换的位置：若参考字与尝试字在拼音层足够接近，
//! 就把它重分类为近音替换，按部分而非全额计入准确度。它宽容的是**识别器听错**，
//! 不是真的记错，所以只处理替换，漏读与增读一律不宽容。

use crate::align::{AlignOp, Alignment, align_normalized};
use crate::score::{Poem, TypedAttempt, TypedScore, score_typed};
use inputx_phonetic_edit::{EditCostTable, MANDARIN_DEFAULT, edit_distance};
use pinyin::{Pinyin, ToPinyinMulti};

/// 判定为近音替换的加权拼音距离上界（含）。
///
/// `MANDARIN_DEFAULT` 的模糊对代价只有 0.2 与 0.3 两档，因此可达距离是
/// 0.0（同音）、0.2、0.3（各一次模糊替换）、0.4 起（两次及以上，或掺入整字节编辑）。
/// 0.35 落在 0.3 与 0.4 之间的空档上：**至多一次模糊替换**算近音，两次起不算，
/// 且判据不贴任何浮点边界。
pub const NEAR_HOMOPHONE_MAX_DISTANCE: f64 = 0.35;

/// 近音替换计入字符错误率的权重；`1.0` 等于不宽容，`0.0` 等于完全免罚。
pub const NEAR_HOMOPHONE_ERROR_WEIGHT: f32 = 0.5;

/// 一处替换在拼音层的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionClass {
    /// 读音相近，按 [`NEAR_HOMOPHONE_ERROR_WEIGHT`] 部分计入错误。
    NearHomophone,
    /// 读音不相近，或任一侧没有拼音数据，按整字错误计入。
    Distinct,
}

/// 单处替换的近音复核结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubstitutionReview {
    /// 归一化参考文本中的字符位置。
    pub reference_index: usize,
    /// 归一化尝试文本中的字符位置。
    pub attempt_index: usize,
    /// 应读字符。
    pub reference: char,
    /// 实际字符。
    pub attempt: char,
    /// 拼音层判定。
    pub class: SubstitutionClass,
    /// 全读音组合中的最小加权距离；任一侧无拼音数据时为 `None`。
    pub distance: Option<f64>,
}

/// 一次键入尝试的严格评分与逐处替换的近音复核。
#[derive(Debug, Clone, PartialEq)]
pub struct PhoneticReview {
    /// 评分；其中 `accuracy_lenient` 已按近音替换重算，`accuracy_strict` 不变。
    pub score: TypedScore,
    /// 按对齐顺序排列的替换复核项；不含漏读与增读。
    pub substitutions: Vec<SubstitutionReview>,
}

impl PhoneticReview {
    /// 被重分类为近音替换的处数。
    #[must_use]
    pub fn near_homophone_count(&self) -> usize {
        self.substitutions
            .iter()
            .filter(|review| review.class == SubstitutionClass::NearHomophone)
            .count()
    }

    /// 仍按整字错误计入的替换处数。
    #[must_use]
    pub fn distinct_substitution_count(&self) -> usize {
        self.substitutions
            .iter()
            .filter(|review| review.class == SubstitutionClass::Distinct)
            .count()
    }
}

/// 取两个字全部读音组合中的最小加权拼音距离。
///
/// 读音经 `pinyin` 的 `ToPinyinMulti` 取全部读音，**匹配任一读音即算匹配**：
/// 古典读音不由现代语境消歧，而 Unihan 还会注入广韵层读音，所以逐读音择优是
/// 唯一不会把「读对了但不是常用音」判成错的做法。任一侧没有拼音数据时返回 `None`。
#[must_use]
pub fn nearest_reading_distance(reference: char, attempt: char) -> Option<f64> {
    nearest_reading_distance_with(reference, attempt, &MANDARIN_DEFAULT)
}

/// 判定一处替换属于近音替换还是整字错误。
#[must_use]
pub fn classify_substitution(reference: char, attempt: char) -> SubstitutionClass {
    classify_distance(nearest_reading_distance(reference, attempt))
}

/// 对一次键入尝试做严格评分，并在拼音层复核其中的替换。
///
/// `accuracy_strict` 直接取自 [`score_typed`]，本层只在其上加回近音替换免掉的那部分，
/// 因此 `accuracy_lenient >= accuracy_strict` 由构造保证，两个口径也不会各算一遍而漂移。
#[must_use]
pub fn review_typed(reference: &Poem, attempt: &TypedAttempt) -> PhoneticReview {
    let strict = score_typed(reference, attempt);
    let alignment = align_normalized(reference.as_str(), attempt.as_str());
    review_alignment(strict, &alignment)
}

fn review_alignment(strict: TypedScore, alignment: &Alignment) -> PhoneticReview {
    let substitutions = alignment
        .ops
        .iter()
        .filter_map(review_op)
        .collect::<Vec<_>>();
    let near_count = substitutions
        .iter()
        .filter(|review| review.class == SubstitutionClass::NearHomophone)
        .count();
    PhoneticReview {
        score: TypedScore {
            accuracy_lenient: lenient_accuracy(
                strict.accuracy_strict,
                near_count,
                alignment.reference_len,
            ),
            ..strict
        },
        substitutions,
    }
}

fn review_op(op: &AlignOp) -> Option<SubstitutionReview> {
    // 漏读没有尝试字、增读没有参考字，拼音层根本凑不出可比对的一对读音。
    // 这个 let-else 就是「绝不宽容漏读或增读」的执行机制：其余变体一律出局。
    let AlignOp::Substitution {
        reference_index,
        attempt_index,
        reference,
        attempt,
    } = op
    else {
        return None;
    };
    let distance = nearest_reading_distance(*reference, *attempt);
    Some(SubstitutionReview {
        reference_index: *reference_index,
        attempt_index: *attempt_index,
        reference: *reference,
        attempt: *attempt,
        class: classify_distance(distance),
        distance,
    })
}

fn classify_distance(distance: Option<f64>) -> SubstitutionClass {
    match distance {
        Some(distance) if distance <= NEAR_HOMOPHONE_MAX_DISTANCE => {
            SubstitutionClass::NearHomophone
        }
        _ => SubstitutionClass::Distinct,
    }
}

fn lenient_accuracy(accuracy_strict: f32, near_count: usize, reference_len: usize) -> f32 {
    if reference_len == 0 || near_count == 0 {
        return accuracy_strict;
    }
    let forgiven = 1.0 - NEAR_HOMOPHONE_ERROR_WEIGHT;
    let credit = forgiven * near_count as f32 / reference_len as f32;
    (accuracy_strict + credit).clamp(0.0, 1.0)
}

fn nearest_reading_distance_with(
    reference: char,
    attempt: char,
    table: &EditCostTable,
) -> Option<f64> {
    let reference_readings = readings(reference);
    let attempt_readings = readings(attempt);
    if reference_readings.is_empty() || attempt_readings.is_empty() {
        return None;
    }
    reference_readings
        .iter()
        .flat_map(|left| {
            attempt_readings
                .iter()
                .map(move |right| edit_distance(left, right, table))
        })
        .min_by(f64::total_cmp)
}

fn readings(character: char) -> Vec<&'static str> {
    character
        .to_pinyin_multi()
        .map(|multi| multi.into_iter().map(Pinyin::plain).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use yunjian_core::{CorpusConfig, CorpusHandle, SCHEMA_VERSION};

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
            "yunjian-recite-phonetic-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建近音 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开近音 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path) {
        let connection = Connection::open(path).expect("创建近音 fixture 数据库");
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
            .expect("创建近音 fixture schema");
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
        connection.close().expect("关闭近音 fixture 数据库");
    }

    fn review(handle: &CorpusHandle, reference: &str, attempt: &str) -> PhoneticReview {
        review_typed(
            &Poem::new(handle, reference).expect("构造参考诗文"),
            &TypedAttempt::new(handle, attempt).expect("构造键入尝试"),
        )
    }

    const RIVER: &str = "轻舟已过万重山";

    #[test]
    fn a_homophone_of_zhong_is_reclassified_as_near_substitution() {
        let fixture = fixture();
        let review = review(&fixture.handle, RIVER, "轻舟已过万虫山");

        assert_eq!(review.score.ops_summary.substitution_count, 1);
        assert_eq!(review.near_homophone_count(), 1);
        assert_eq!(review.distinct_substitution_count(), 0);
        let substitution = review.substitutions[0];
        assert_eq!(substitution.reference, '重');
        assert_eq!(substitution.attempt, '虫');
        assert_eq!(substitution.class, SubstitutionClass::NearHomophone);
        assert_eq!(substitution.distance, Some(0.0));
    }

    #[test]
    fn a_semantically_different_character_stays_a_full_substitution() {
        let fixture = fixture();
        let review = review(&fixture.handle, RIVER, "轻舟已过万重水");

        assert_eq!(review.score.ops_summary.substitution_count, 1);
        assert_eq!(review.near_homophone_count(), 0);
        assert_eq!(review.distinct_substitution_count(), 1);
        let substitution = review.substitutions[0];
        assert_eq!(substitution.reference, '山');
        assert_eq!(substitution.attempt, '水');
        assert_eq!(substitution.class, SubstitutionClass::Distinct);
        assert_eq!(substitution.distance, Some(2.0));
        assert_eq!(review.score.accuracy_lenient, review.score.accuracy_strict);
    }

    #[test]
    fn any_reading_may_carry_the_match_even_when_the_first_does_not() {
        let readings_of_zhong = readings('重');
        let readings_of_chong = readings('虫');
        assert_eq!(readings_of_zhong, ["zhong", "chong", "tong"]);
        assert_eq!(readings_of_chong, ["chong", "hui"]);

        let first_only = edit_distance(
            readings_of_zhong[0],
            readings_of_chong[0],
            &MANDARIN_DEFAULT,
        );
        assert!(
            first_only > NEAR_HOMOPHONE_MAX_DISTANCE,
            "只比首读音会得到 {first_only}，本就不该判为近音"
        );
        assert_eq!(nearest_reading_distance('重', '虫'), Some(0.0));
        assert_eq!(
            classify_substitution('重', '虫'),
            SubstitutionClass::NearHomophone
        );
    }

    #[test]
    fn the_mandarin_fuzzy_pairs_are_what_make_a_near_match_near() {
        assert_eq!(nearest_reading_distance('山', '三'), Some(0.3));
        assert_eq!(nearest_reading_distance('山', '伤'), Some(0.2));
        for (reference, attempt) in [('山', '三'), ('山', '伤')] {
            let plain = nearest_reading_distance_with(reference, attempt, &EditCostTable::EMPTY)
                .expect("两侧都有读音");
            assert!(
                plain > NEAR_HOMOPHONE_MAX_DISTANCE,
                "空表下 {reference}/{attempt} 应回落到整字代价，实测 {plain}"
            );
        }
    }

    #[test]
    fn a_deletion_is_never_reclassified_phonetically() {
        let fixture = fixture();
        let review = review(&fixture.handle, RIVER, "轻舟已过万山");

        assert_eq!(review.score.ops_summary.deletion_count, 1);
        assert_eq!(review.score.ops_summary.substitution_count, 0);
        assert!(review.substitutions.is_empty());
        assert_eq!(review.near_homophone_count(), 0);
        assert_eq!(review.score.accuracy_lenient, review.score.accuracy_strict);
    }

    #[test]
    fn an_insertion_is_never_reclassified_even_when_it_sounds_like_the_reference() {
        let fixture = fixture();
        let review = review(&fixture.handle, RIVER, "轻舟已过万重虫山");

        assert_eq!(review.score.ops_summary.insertion_count, 1);
        assert_eq!(review.score.ops_summary.substitution_count, 0);
        assert_eq!(nearest_reading_distance('重', '虫'), Some(0.0));
        assert!(review.substitutions.is_empty());
        assert_eq!(review.score.accuracy_lenient, review.score.accuracy_strict);
    }

    #[test]
    fn lenient_accuracy_never_falls_below_strict_accuracy() {
        let fixture = fixture();
        let attempts = [
            RIVER,
            "轻舟已过万虫山",
            "轻舟已过万重水",
            "轻舟已过万虫水",
            "轻舟已过万山",
            "轻舟已过万重虫山",
            "轻州已过完虫山",
            "白日依山尽",
            "",
        ];
        for attempt in attempts {
            let review = review(&fixture.handle, RIVER, attempt);
            assert!(
                review.score.accuracy_lenient >= review.score.accuracy_strict,
                "尝试「{attempt}」的宽容准确度 {} 低于严格准确度 {}",
                review.score.accuracy_lenient,
                review.score.accuracy_strict
            );
            assert!((0.0..=1.0).contains(&review.score.accuracy_lenient));
        }
    }

    // 容差取 1e-6：半额免罚（0.9286）与不免罚（0.8571）、全额免罚（1.0）相距约 0.07,
    // 比容差大四个数量级，所以它只吸收 f32 末位差，不会放过算错权重。
    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "{what}：实际 {actual}，期望 {expected}"
        );
    }

    #[test]
    fn a_near_substitution_is_partially_forgiven_not_fully() {
        let fixture = fixture();
        let review = review(&fixture.handle, RIVER, "轻舟已过万虫山");

        let reference_len = RIVER.chars().count() as f32;
        assert_close(
            review.score.accuracy_strict,
            1.0 - 1.0 / reference_len,
            "严格准确度按整字计一次错",
        );
        assert_close(
            review.score.accuracy_lenient,
            1.0 - NEAR_HOMOPHONE_ERROR_WEIGHT / reference_len,
            "宽容准确度按半额计一次错",
        );
        assert!(
            review.score.accuracy_lenient < 1.0,
            "部分计入，不是全额免罚"
        );
        assert!(review.score.accuracy_lenient > review.score.accuracy_strict);
    }

    #[test]
    fn a_character_without_a_reading_keeps_the_substitution_whole() {
        let fixture = fixture();
        let review = review(&fixture.handle, "一", "1");

        assert_eq!(nearest_reading_distance('一', '1'), None);
        assert_eq!(review.substitutions.len(), 1);
        assert_eq!(review.substitutions[0].distance, None);
        assert_eq!(review.substitutions[0].class, SubstitutionClass::Distinct);
        assert_eq!(review.score.accuracy_lenient, review.score.accuracy_strict);
    }

    #[test]
    fn strict_accuracy_and_every_other_component_are_left_untouched() {
        let fixture = fixture();
        let poem = Poem::new(&fixture.handle, RIVER).expect("构造参考诗文");
        let attempt = TypedAttempt::new(&fixture.handle, "轻舟已过万虫山").expect("构造键入尝试");
        let strict = score_typed(&poem, &attempt);
        let review = review_typed(&poem, &attempt);

        assert_eq!(review.score.completeness, strict.completeness);
        assert_eq!(review.score.accuracy_strict, strict.accuracy_strict);
        assert_eq!(review.score.fluency, strict.fluency);
        assert_eq!(review.score.is_rejected, strict.is_rejected);
        assert_eq!(review.score.ops_summary, strict.ops_summary);
        assert_ne!(review.score.accuracy_lenient, strict.accuracy_lenient);
    }
}
