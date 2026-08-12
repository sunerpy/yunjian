use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;
use yunjian_core::{Config, GradingConfig};
use yunjian_recite::{FSRS6_PARAMETERS, FsrsGrade, OpsSummary, Scheduler, TypedScore, grade_typed};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yunjian-schedule-{tag}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建排程测试目录");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn score(
    completeness: f32,
    accuracy_strict: f32,
    accuracy_lenient: f32,
    rerecitation_count: usize,
    is_rejected: bool,
) -> TypedScore {
    TypedScore {
        completeness,
        accuracy_strict,
        accuracy_lenient,
        fluency: 1.0,
        is_rejected,
        ops_summary: OpsSummary {
            rerecitation_count,
            ..OpsSummary::default()
        },
    }
}

fn rank(grade: FsrsGrade) -> u8 {
    match grade {
        FsrsGrade::Again => 1,
        FsrsGrade::Hard => 2,
        FsrsGrade::Good => 3,
        FsrsGrade::Easy => 4,
    }
}

#[test]
fn grading_is_a_total_single_valued_function_over_the_boundary_grid() {
    let grading = GradingConfig::default();
    let completeness = [0.0, 0.59, 0.6, 0.9, 1.0];
    let strict = [0.0, 0.84, 0.85, 0.96, 0.97, 1.0];
    let lenient = [0.0, 0.84, 0.85, 0.96, 0.97, 1.0];

    let mut visited = 0;
    for completeness in completeness {
        for accuracy_strict in strict {
            for accuracy_lenient in lenient {
                for rerecitation_count in [0, 1, 3] {
                    for first_attempt in [false, true] {
                        for is_rejected in [false, true] {
                            let grade = grade_typed(
                                &score(
                                    completeness,
                                    accuracy_strict,
                                    accuracy_lenient,
                                    rerecitation_count,
                                    is_rejected,
                                ),
                                first_attempt,
                                &grading,
                            );
                            assert!((1..=4).contains(&rank(grade)));
                            visited += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(visited, 5 * 6 * 6 * 3 * 2 * 2);
}

#[test]
fn grading_priority_covers_the_named_product_cases() {
    let grading = GradingConfig::default();

    assert_eq!(
        grade_typed(&score(1.0, 1.0, 1.0, 0, false), true, &grading),
        FsrsGrade::Easy
    );
    assert_eq!(
        grade_typed(&score(0.4, 1.0, 1.0, 0, false), true, &grading),
        FsrsGrade::Again
    );
    assert_eq!(
        grade_typed(&score(1.0, 0.95, 0.95, 1, false), true, &grading),
        FsrsGrade::Hard
    );
    assert_eq!(
        grade_typed(&score(1.0, 0.96, 0.96, 0, false), true, &grading),
        FsrsGrade::Good
    );
    assert_eq!(
        grade_typed(&score(1.0, 1.0, 1.0, 0, true), true, &grading),
        FsrsGrade::Again,
        "拒绝优先于完美数值"
    );
}

#[test]
fn strictly_more_errors_never_improve_the_grade() {
    let grading = GradingConfig::default();
    let attempts = [
        score(1.0, 1.0, 1.0, 0, false),
        score(1.0, 0.96, 0.96, 0, false),
        score(1.0, 0.95, 0.95, 1, false),
        score(1.0, 0.84, 0.84, 1, false),
        score(0.59, 0.84, 0.84, 1, false),
        score(0.4, 0.4, 0.4, 2, true),
    ];

    let grades = attempts.map(|attempt| rank(grade_typed(&attempt, true, &grading)));
    for pair in grades.windows(2) {
        assert!(
            pair[1] <= pair[0],
            "错误增多却从 {} 升到 {}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn thresholds_are_read_from_recite_grading_config() {
    let config: Config = toml::from_str(
        r#"
        [recite.grading]
        again_completeness_below = 0.7
        hard_accuracy_lenient_below = 0.9
        easy_accuracy_strict_at_least = 0.99
        "#,
    )
    .expect("解析 recite.grading");

    assert_eq!(config.recite.grading.again_completeness_below, 0.7);
    assert_eq!(config.recite.grading.hard_accuracy_lenient_below, 0.9);
    assert_eq!(config.recite.grading.easy_accuracy_strict_at_least, 0.99);
    assert_eq!(
        grade_typed(
            &score(0.65, 1.0, 1.0, 0, false),
            true,
            &config.recite.grading,
        ),
        FsrsGrade::Again
    );
    assert_eq!(
        grade_typed(
            &score(1.0, 0.98, 0.98, 0, false),
            true,
            &config.recite.grading,
        ),
        FsrsGrade::Good
    );
}

#[test]
fn fsrs_uses_twenty_one_parameters_with_decay_zero_point_two() {
    assert_eq!(FSRS6_PARAMETERS.len(), 21);
    assert_eq!(FSRS6_PARAMETERS[20], 0.2);
    assert!(
        FSRS6_PARAMETERS
            .iter()
            .all(|parameter| parameter.is_finite())
    );
}

#[test]
fn user_chosen_review_entry_point_accepts_all_four_grades() {
    for (index, grade) in FsrsGrade::ALL.into_iter().enumerate() {
        let dir = TempDir::new("all-grades");
        let mut scheduler = Scheduler::open(dir.join("app.db")).expect("打开排程库");
        let stable_id = format!("poem-{index}");

        let state = scheduler
            .review_at(&stable_id, grade, 20_000)
            .expect("提交用户选择的等级");

        assert_eq!(state.stable_id, stable_id);
        assert_eq!(state.last_grade, grade);
        assert!(state.due_day > 20_000);
    }
}

#[test]
fn due_today_returns_only_cards_due_no_later_than_the_requested_day() {
    let dir = TempDir::new("due-today");
    let mut scheduler = Scheduler::open(dir.join("app.db")).expect("打开排程库");
    scheduler
        .review_at("poem-due", FsrsGrade::Again, 20_000)
        .expect("写入到期卡片");
    scheduler
        .review_at("poem-later", FsrsGrade::Easy, 20_000)
        .expect("写入较晚卡片");

    let first_due_day = scheduler
        .state("poem-due")
        .expect("读取状态")
        .expect("状态存在")
        .due_day;
    let due = scheduler.due_on(first_due_day).expect("查询到期卡片");

    assert!(due.iter().any(|state| state.stable_id == "poem-due"));
    assert!(
        due.iter().all(|state| state.due_day <= first_due_day),
        "不得返回尚未到期的卡片"
    );
}

#[test]
fn history_survives_content_hash_changes_because_storage_is_keyed_by_stable_id() {
    let dir = TempDir::new("stable-id");
    let database = dir.join("app.db");
    let stable_id = "1111111111111111";

    let before = {
        let mut scheduler = Scheduler::open(&database).expect("打开排程库");
        scheduler
            .review_at(stable_id, FsrsGrade::Good, 20_000)
            .expect("首次复习")
    };
    simulate_corpus_rebuild(&dir, stable_id, "aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb");
    let after = Scheduler::open(&database)
        .expect("重开排程库")
        .state(stable_id)
        .expect("读取状态")
        .expect("历史仍存在");

    assert_eq!(after, before);
    let connection = Connection::open(database).expect("检查 app 库 schema");
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='review_state'",
            [],
            |row| row.get(0),
        )
        .expect("review_state 表存在");
    assert!(sql.contains("stable_id"));
    assert!(!sql.contains("content_hash"));
}

fn simulate_corpus_rebuild(dir: &TempDir, stable_id: &str, before: &str, after: &str) {
    let corpus = dir.join("corpus.db");
    write_corpus(&corpus, stable_id, before);
    std::fs::remove_file(&corpus).expect("移除旧语料库");
    write_corpus(&corpus, stable_id, after);
}

fn write_corpus(path: &Path, stable_id: &str, content_hash: &str) {
    let connection = Connection::open(path).expect("创建语料库夹具");
    connection
        .execute_batch(
            "CREATE TABLE poem(
                stable_id TEXT PRIMARY KEY NOT NULL,
                content_hash TEXT NOT NULL
            );",
        )
        .expect("创建 poem 表");
    connection
        .execute(
            "INSERT INTO poem(stable_id, content_hash) VALUES (?1, ?2)",
            [stable_id, content_hash],
        )
        .expect("写入诗词身份");
}
