//! 打分口径的单元测试。
//!
//! 这里刻意不碰数据库：四项权重与 Jaccard 口径是纯函数，把它们的断言压在语料 fixture 上会
//! 让「权重改了」和「语料改了」两种失败长得一样。

use super::*;
use yunjian_core::{
    DynastyLabel, PoemFeatures, PoemRecord, RhymeBook, RhymeConfidence, RhymeGroupMembership,
    RhymeTone,
};

fn features(
    body: &str,
    tags: &[&str],
    ci_tune: Option<&str>,
    rhyme: &[(RhymeBook, &str, RhymeTone)],
) -> PoemFeatures {
    PoemFeatures {
        poem: PoemRecord {
            stable_id: format!("test:{body}"),
            content_hash: "hash".to_owned(),
            title: "题".to_owned(),
            title_raw: "题".to_owned(),
            ci_tune: ci_tune.map(str::to_owned),
            author: "某".to_owned(),
            dynasty: DynastyLabel {
                canonical: "唐".to_owned(),
                raw: "唐".to_owned(),
            },
            genre: "shi".to_owned(),
            body: body.to_owned(),
            body_original: body.to_owned(),
            script: "simplified".to_owned(),
            first_line: body.to_owned(),
            last_chars: Vec::new(),
            line_count: 1,
            char_count: 0,
            work_group: format!("wg:{body}"),
            edition_group: format!("eg:{body}"),
        },
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        rhyme_groups: rhyme
            .iter()
            .map(|(book, group, tone)| RhymeGroupMembership {
                book: *book,
                group: (*group).to_owned(),
                tone: *tone,
                confidence: RhymeConfidence::Unambiguous,
            })
            .collect(),
    }
}

fn no_stopwords() -> BTreeSet<char> {
    BTreeSet::new()
}

#[test]
fn the_four_weights_sum_to_one_so_a_score_is_readable_as_a_fraction() {
    let weights = weights();
    let sum = weights.shared_tags
        + weights.same_rhyme_group
        + weights.same_ci_tune
        + weights.character_overlap;
    assert!((sum - 1.0).abs() < 1e-9, "四项权重之和应为 1.0，实为 {sum}");
}

#[test]
fn an_identical_poem_scores_one() {
    let stopwords = no_stopwords();
    let subject = features(
        "床前明月光",
        &["月", "思乡"],
        Some("水调歌头"),
        &[(RhymeBook::Pingshui, "七阳", RhymeTone::Level)],
    );
    let profile = Profile::new(&subject, &stopwords);
    let scored = total(&score(&profile, &profile));
    assert!(
        (scored - 1.0).abs() < 1e-9,
        "完全相同的两篇应得满分，实为 {scored}"
    );
}

#[test]
fn nothing_in_common_scores_zero() {
    let stopwords = no_stopwords();
    let left = Profile::new(&features("甲乙丙", &["月"], None, &[]), &stopwords);
    let right = Profile::new(&features("丁戊己", &["战乱"], None, &[]), &stopwords);
    assert!(total(&score(&left, &right)).abs() < 1e-9, "毫无交集应得 0");
}

#[test]
fn excluding_frequent_characters_removes_the_overlap_they_would_have_manufactured() {
    let left = features("不人山甲", &[], None, &[]);
    let right = features("不人山乙", &[], None, &[]);

    let unfiltered = score(
        &Profile::new(&left, &no_stopwords()),
        &Profile::new(&right, &no_stopwords()),
    );
    assert!(
        unfiltered.character_overlap > 0.0,
        "不排除高频字时「不人山」应造出重叠分"
    );

    let stopwords: BTreeSet<char> = ['不', '人', '山'].into_iter().collect();
    let filtered = score(
        &Profile::new(&left, &stopwords),
        &Profile::new(&right, &stopwords),
    );
    assert!(
        filtered.character_overlap.abs() < 1e-9,
        "排除高频字后只剩「甲」「乙」两个不同字，重叠分必须归零，实为 {}",
        filtered.character_overlap
    );
}

#[test]
fn two_untagged_poems_are_not_treated_as_sharing_every_tag() {
    let stopwords = no_stopwords();
    let left = Profile::new(&features("甲", &[], None, &[]), &stopwords);
    let right = Profile::new(&features("乙", &[], None, &[]), &stopwords);
    assert!(
        score(&left, &right).shared_tags.abs() < 1e-9,
        "两侧都没有标签是缺数据，不是完全一致"
    );
}

#[test]
fn a_rhyme_group_match_requires_the_same_tone_not_just_the_same_group() {
    let stopwords = no_stopwords();
    let level = Profile::new(
        &features(
            "甲",
            &[],
            None,
            &[(RhymeBook::Cilin, "第一部", RhymeTone::Level)],
        ),
        &stopwords,
    );
    let oblique = Profile::new(
        &features(
            "乙",
            &[],
            None,
            &[(RhymeBook::Cilin, "第一部", RhymeTone::Oblique)],
        ),
        &stopwords,
    );
    assert!(
        score(&level, &oblique).same_rhyme_group.abs() < 1e-9,
        "同韵部但平仄不同不相押，不得记同韵部分"
    );

    let same = Profile::new(
        &features(
            "丙",
            &[],
            None,
            &[(RhymeBook::Cilin, "第一部", RhymeTone::Level)],
        ),
        &stopwords,
    );
    assert!(
        (score(&level, &same).same_rhyme_group - WEIGHT_SAME_RHYME_GROUP).abs() < 1e-9,
        "同书同韵部同声调应记满分量"
    );
}

#[test]
fn a_rhyme_group_match_requires_the_same_book() {
    let stopwords = no_stopwords();
    let pingshui = Profile::new(
        &features(
            "甲",
            &[],
            None,
            &[(RhymeBook::Pingshui, "一东", RhymeTone::Level)],
        ),
        &stopwords,
    );
    let cilin = Profile::new(
        &features(
            "乙",
            &[],
            None,
            &[(RhymeBook::Cilin, "一东", RhymeTone::Level)],
        ),
        &stopwords,
    );
    assert!(
        score(&pingshui, &cilin).same_rhyme_group.abs() < 1e-9,
        "韵部名相同但韵书不同不构成同韵部"
    );
}

#[test]
fn two_poems_without_a_ci_tune_do_not_count_as_sharing_one() {
    let stopwords = no_stopwords();
    let left = Profile::new(&features("甲", &[], None, &[]), &stopwords);
    let right = Profile::new(&features("乙", &[], None, &[]), &stopwords);
    assert!(
        score(&left, &right).same_ci_tune.abs() < 1e-9,
        "两首都是诗（无词牌），不该判为同词牌"
    );
}

#[test]
fn each_component_never_exceeds_its_declared_weight() {
    let stopwords = no_stopwords();
    let left = features(
        "床前明月光",
        &["月"],
        Some("水调歌头"),
        &[(RhymeBook::Pingshui, "七阳", RhymeTone::Level)],
    );
    let right = features(
        "举头望明月",
        &["月", "思乡"],
        Some("水调歌头"),
        &[(RhymeBook::Pingshui, "七阳", RhymeTone::Level)],
    );
    let components = score(
        &Profile::new(&left, &stopwords),
        &Profile::new(&right, &stopwords),
    );
    assert!(components.shared_tags <= WEIGHT_SHARED_TAGS);
    assert!(components.same_rhyme_group <= WEIGHT_SAME_RHYME_GROUP);
    assert!(components.same_ci_tune <= WEIGHT_SAME_CI_TUNE);
    assert!(components.character_overlap <= WEIGHT_CHARACTER_OVERLAP);
    let scored = total(&components);
    assert!(
        (0.0..=1.0).contains(&scored),
        "得分应落在 [0,1]，实为 {scored}"
    );
}

#[test]
fn the_axes_a_candidate_matched_are_reported_per_axis() {
    let stopwords = no_stopwords();
    let source = features(
        "床前明月光",
        &["月"],
        Some("水调歌头"),
        &[(RhymeBook::Pingshui, "七阳", RhymeTone::Level)],
    );
    let candidate = features("举头望明月", &["月"], Some("念奴娇"), &[]);
    let axes = Profile::new(&source, &stopwords).axes_against(
        &Profile::new(&candidate, &stopwords),
        true,
        false,
    );
    let keys: Vec<&str> = axes.iter().map(|axis| axis.as_key()).collect();
    assert_eq!(
        keys,
        vec!["theme", "author"],
        "只有共享标签与同作者成立，词牌不同、韵部一侧为空、朝代未匹配"
    );
}
