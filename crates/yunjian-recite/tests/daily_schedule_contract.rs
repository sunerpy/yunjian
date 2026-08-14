use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use yunjian_recite::{
    BudgetConfig, DailyQueueInput, EstimatedTask, FsrsGrade, PracticeOutcome, QueueKind,
    RetentionObservation, Scheduler, plan_daily_queue,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yunjian-learning-{tag}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建排程测试目录");
        Self(path)
    }

    fn database(&self) -> PathBuf {
        self.0.join("recite.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn one_capability_ticket_can_advance_fsrs_exactly_once() {
    let dir = TempDir::new("one-ticket");
    let mut scheduler = Scheduler::open(dir.database()).expect("打开复习库");
    let ticket = scheduler
        .issue_review_ticket_at("poem:v1:0-2", 20_000, 600)
        .expect("签发正式复习票据");

    let first = scheduler
        .submit_review_ticket_at(&ticket, FsrsGrade::Hard, 20_000, 1_000)
        .expect("首次提交应成功");
    let due_day = first.review.due_day;
    assert_eq!(first.relearning.expect("困难后应进入再学习").due_at, 1_600);

    let error = scheduler
        .submit_review_ticket_at(&ticket, FsrsGrade::Easy, 20_000, 1_001)
        .expect_err("同一能力票据不得重复提交");
    assert!(error.to_string().contains("已经提交"), "{error}");
    assert_eq!(
        scheduler
            .state("poem:v1:0-2")
            .expect("读状态")
            .expect("状态存在")
            .due_day,
        due_day,
        "重复点击不得再次推进 due_day"
    );
    assert_eq!(scheduler.review_count().expect("统计正式复习"), 1);
}

#[test]
fn same_day_relearning_records_practice_without_calling_fsrs_again() {
    let dir = TempDir::new("relearning");
    let mut scheduler = Scheduler::open(dir.database()).expect("打开复习库");
    let ticket = scheduler
        .issue_review_ticket_at("poem:v1:0-2", 20_000, 600)
        .expect("签发票据");
    let formal = scheduler
        .submit_review_ticket_at(&ticket, FsrsGrade::Again, 20_000, 10_000)
        .expect("提交正式复习");
    let formal_due_day = formal.review.due_day;
    let first = formal.relearning.expect("重来后应进入再学习");
    assert_eq!(first.due_at, 10_600);

    let second = scheduler
        .record_relearning_at(&first.id, PracticeOutcome::Passed, 10_600)
        .expect("第一次再学习通过")
        .expect("通过后还应安排一小时巩固");
    assert_eq!(second.due_at, 14_200);

    assert!(
        scheduler
            .record_relearning_at(&second.id, PracticeOutcome::Passed, 14_200)
            .expect("第二次再学习通过")
            .is_none(),
        "一小时巩固通过后结束当日再学习"
    );
    assert_eq!(scheduler.review_count().expect("统计正式复习"), 1);
    assert_eq!(scheduler.practice_event_count().expect("统计练习事件"), 2);
    assert_eq!(
        scheduler
            .state("poem:v1:0-2")
            .expect("读状态")
            .expect("状态存在")
            .due_day,
        formal_due_day,
        "10 分钟和 1 小时练习不得推进 FSRS 到期日"
    );
}

fn task(id: &str, kind: QueueKind, minutes: f32, due_day: Option<i64>) -> EstimatedTask {
    EstimatedTask {
        id: id.to_owned(),
        kind,
        estimated_minutes: minutes,
        due_day,
    }
}

#[test]
fn daily_budget_prioritizes_obligations_and_exposes_every_unpacked_due_card() {
    let input = DailyQueueInput {
        today: 20_000,
        relearning: vec![task("relearn", QueueKind::Relearning, 3.0, None)],
        scheduled: vec![
            task("due", QueueKind::Scheduled, 4.0, Some(20_000)),
            task("oldest", QueueKind::Overdue, 5.0, Some(19_990)),
            task("newer", QueueKind::Overdue, 4.0, Some(19_999)),
        ],
        new_chunks: vec![task("new", QueueKind::New, 2.0, None)],
        future: vec![task("tomorrow", QueueKind::Scheduled, 6.0, Some(20_001))],
        retention: RetentionObservation {
            non_again: 3,
            sample_size: 4,
        },
    };
    let report = plan_daily_queue(
        input,
        BudgetConfig {
            daily_minutes: 12.0,
            new_chunk_limit: 3,
            retention_target: 0.85,
        },
    );

    assert_eq!(
        report
            .planned
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["relearn", "oldest", "newer"],
        "再学习优先，逾期按最老优先，义务占满预算时不发新卡"
    );
    assert_eq!(report.planned_minutes, 12.0);
    assert_eq!(report.due_total, 3);
    assert_eq!(report.due_planned, 2);
    assert_eq!(report.due_unplanned, 1);
    assert_eq!(report.backlog.count, 1);
    assert_eq!(report.backlog.oldest_overdue_days, 0);
    assert_eq!(report.backlog.estimated_clear_minutes, 4.0);
    assert_eq!(report.new_chunks_planned, 0);
    assert_eq!(report.next_seven_days[0].day, 20_001);
    assert_eq!(report.next_seven_days[0].estimated_minutes, 6.0);
    assert_eq!(report.observed_retention.rate, Some(0.75));
    assert_eq!(report.observed_retention.sample_size, 4);
    assert_eq!(report.retention_target, 0.85);
}

#[test]
fn cold_start_estimate_uses_only_content_length_and_first_learning() {
    let short_review = yunjian_recite::estimate_minutes(20, false);
    let long_review = yunjian_recite::estimate_minutes(80, false);
    let short_first = yunjian_recite::estimate_minutes(20, true);

    assert!(long_review > short_review);
    assert!(short_first > short_review);
    assert_eq!(short_review, yunjian_recite::estimate_minutes(20, false));
}
