use yunjian_recite::{
    CompleteRecitation, FsrsGrade, ReviewState, SEGMENTATION_VERSION, build_learning_objects,
    summarize_mastery,
};

const JING_YE_SI: &str = "床前明月光，疑是地上霜。举头望明月，低头思故乡。";
const CHANG_HEN_GE_EXCERPT: &str = concat!(
    "汉皇重色思倾国，御宇多年求不得。",
    "杨家有女初长成，养在深闺人未识。",
    "天生丽质难自弃。",
);

fn state(stable_id: &str, due_day: i64, grade: FsrsGrade) -> ReviewState {
    ReviewState {
        stable_id: stable_id.to_owned(),
        stability: 9.0,
        difficulty: 4.0,
        due_day,
        last_review_day: 20_000,
        scheduled_days: 9,
        last_grade: grade,
    }
}

#[test]
fn a_short_four_line_poem_is_one_fsrs_chunk_not_four_tiny_cards() {
    let objects = build_learning_objects("jing-ye-si", JING_YE_SI);

    assert_eq!(SEGMENTATION_VERSION, 1);
    assert_eq!(objects.whole.poem_id, "jing-ye-si");
    assert!(!objects.whole.enters_fsrs());
    assert_eq!(objects.chunks.len(), 1);
    assert_eq!(objects.chunks[0].line_range, 0..4);
    assert_eq!(objects.chunks[0].body, JING_YE_SI);
    assert!(objects.chunks[0].enters_fsrs());
    assert_eq!(objects.chunks[0].stable_id, "jing-ye-si:v1:0-4");

    let foot = objects.chunks[0].foot(2);
    assert_eq!(foot.chunk_id, objects.chunks[0].stable_id);
    assert_eq!(foot.foot_index, 2);
    assert_eq!(foot.stable_id, "jing-ye-si:v1:0-4:foot:2");
    assert!(!foot.enters_fsrs());
}

#[test]
fn adjacent_lines_form_chunks_and_a_trailing_line_joins_the_previous_chunk() {
    let first = build_learning_objects("chang-hen-ge", CHANG_HEN_GE_EXCERPT);
    let rebuilt = build_learning_objects("chang-hen-ge", CHANG_HEN_GE_EXCERPT);

    assert_eq!(first, rebuilt, "同一分段版本重建后必须逐字节稳定");
    assert_eq!(first.chunks.len(), 2);
    assert_eq!(first.chunks[0].line_range, 0..2);
    assert_eq!(first.chunks[1].line_range, 2..5);
    assert_eq!(first.chunks[0].stable_id, "chang-hen-ge:v1:0-2");
    assert_eq!(first.chunks[1].stable_id, "chang-hen-ge:v1:2-5");
    assert_eq!(
        first.chunks[1].body,
        "杨家有女初长成，养在深闺人未识。天生丽质难自弃。"
    );
}

#[test]
fn mastery_never_averages_away_an_unestablished_or_overdue_chunk() {
    let objects = build_learning_objects("chang-hen-ge", CHANG_HEN_GE_EXCERPT);
    let first = &objects.chunks[0].stable_id;
    let second = &objects.chunks[1].stable_id;

    let incomplete = summarize_mastery(
        &objects,
        &[state(first, 20_010, FsrsGrade::Easy)],
        20_001,
        None,
    );
    assert_eq!(incomplete.established, 1);
    assert_eq!(incomplete.total, 2);
    assert_eq!(incomplete.due, 0);
    assert!(incomplete.weak_points.is_empty());
    assert!(
        !incomplete.currently_solid,
        "缺一片就不能用高稳定度平均成稳固"
    );

    let overdue = summarize_mastery(
        &objects,
        &[
            state(first, 20_010, FsrsGrade::Easy),
            state(second, 19_999, FsrsGrade::Hard),
        ],
        20_001,
        Some(CompleteRecitation::Passed {
            occurred_day: 20_001,
        }),
    );
    assert_eq!(overdue.established, 2);
    assert_eq!(overdue.due, 1);
    assert_eq!(overdue.weak_points.len(), 1);
    assert_eq!(overdue.weak_points[0], second.as_str());
    assert_eq!(
        overdue.complete_recitation,
        Some(CompleteRecitation::Passed {
            occurred_day: 20_001
        })
    );
    assert!(!overdue.currently_solid, "完整背诵通过也不能遮住逾期薄弱片");

    let solid = summarize_mastery(
        &objects,
        &[
            state(first, 20_010, FsrsGrade::Easy),
            state(second, 20_005, FsrsGrade::Good),
        ],
        20_001,
        Some(CompleteRecitation::Passed {
            occurred_day: 20_001,
        }),
    );
    assert!(solid.currently_solid);
}
