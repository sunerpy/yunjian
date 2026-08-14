//! 每日时间预算、积压、压力与保持率报告。

use std::collections::BTreeMap;

/// 每日队列中的任务种类，枚举顺序不是优先级来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    /// 当日内十分钟或一小时再学习。
    Relearning,
    /// 早于今天到期的联片。
    Overdue,
    /// 今天或未来到期的联片。
    Scheduled,
    /// 尚未建立的联片。
    New,
}

/// 带回退耗时估计的队列任务。
#[derive(Debug, Clone, PartialEq)]
pub struct EstimatedTask {
    /// 联片或再学习任务标识。
    pub id: String,
    /// 任务种类。
    pub kind: QueueKind,
    /// 装箱使用的预计分钟数。
    pub estimated_minutes: f32,
    /// FSRS 到期日；再学习和新联片没有该值。
    pub due_day: Option<i64>,
}

/// 过去三十天到期正式复习的观察样本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionObservation {
    /// 非 Again 的正式复习次数。
    pub non_again: usize,
    /// 到期正式复习样本总数。
    pub sample_size: usize,
}

/// 每日装箱参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetConfig {
    /// 每日可承诺的总分钟数。
    pub daily_minutes: f32,
    /// 每日新联片上限。
    pub new_chunk_limit: usize,
    /// 与观察保持率并列展示的运营目标。
    pub retention_target: f32,
}

/// 生成每日计划所需的任务集合。
#[derive(Debug, Clone, PartialEq)]
pub struct DailyQueueInput {
    /// 当前本地 Unix 日序号。
    pub today: i64,
    /// 已到触发时刻的当日内再学习。
    pub relearning: Vec<EstimatedTask>,
    /// 到期或逾期联片。
    pub scheduled: Vec<EstimatedTask>,
    /// 可在余额内引入的新联片。
    pub new_chunks: Vec<EstimatedTask>,
    /// 未来七日内的已排程联片。
    pub future: Vec<EstimatedTask>,
    /// 过去三十天的真实正式复习结果。
    pub retention: RetentionObservation,
}

/// 未装入今日计划的到期压力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacklogReport {
    /// 未装入计划的到期联片数。
    pub count: usize,
    /// 未装入项中最老逾期天数；只有今日到期项时为零。
    pub oldest_overdue_days: i64,
    /// 按当前估计清理这些联片所需分钟数。
    pub estimated_clear_minutes: f32,
}

/// 某一未来日期的预计压力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DailyPressure {
    /// Unix 日序号。
    pub day: i64,
    /// 该日到期联片数。
    pub count: usize,
    /// 该日预计分钟数。
    pub estimated_minutes: f32,
}

/// 观察保持率及其真实样本量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObservedRetention {
    /// 非 Again 比例；零样本时不外推。
    pub rate: Option<f32>,
    /// 真实样本量。
    pub sample_size: usize,
}

/// 一次每日预算装箱的完整可见报告。
#[derive(Debug, Clone, PartialEq)]
pub struct DailyQueueReport {
    /// 预算内按优先级排列的任务。
    pub planned: Vec<EstimatedTask>,
    /// 计划任务预计总分钟数。
    pub planned_minutes: f32,
    /// 全部到期与逾期联片数。
    pub due_total: usize,
    /// 装入计划的到期与逾期联片数。
    pub due_planned: usize,
    /// 未装入计划的到期与逾期联片数。
    pub due_unplanned: usize,
    /// 未装入计划的到期压力。
    pub backlog: BacklogReport,
    /// 实际装入计划的新联片数。
    pub new_chunks_planned: usize,
    /// 明日起七日的 FSRS 压力。
    pub next_seven_days: Vec<DailyPressure>,
    /// 过去三十天观察保持率。
    pub observed_retention: ObservedRetention,
    /// 配置的运营保持率目标。
    pub retention_target: f32,
}

/// 按再学习、逾期、今日到期、新联片的顺序装入每日时间预算。
#[must_use]
pub fn plan_daily_queue(input: DailyQueueInput, config: BudgetConfig) -> DailyQueueReport {
    let budget = finite_non_negative(config.daily_minutes);
    let mut relearning = input.relearning;
    relearning.sort_by(|left, right| left.id.cmp(&right.id));
    let mut due = input.scheduled;
    due.sort_by_key(|task| (task.due_day.unwrap_or(input.today), task.id.clone()));
    let due_total = due.len();

    let mut planned = Vec::new();
    let mut planned_minutes = 0.0;
    for task in relearning {
        try_pack(task, budget, &mut planned_minutes, &mut planned);
    }
    for task in &due {
        try_pack(task.clone(), budget, &mut planned_minutes, &mut planned);
    }
    let due_planned = planned
        .iter()
        .filter(|task| matches!(task.kind, QueueKind::Overdue | QueueKind::Scheduled))
        .count();
    let planned_due_ids = planned
        .iter()
        .filter(|task| matches!(task.kind, QueueKind::Overdue | QueueKind::Scheduled))
        .map(|task| task.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let unplanned_due = due
        .iter()
        .filter(|task| !planned_due_ids.contains(task.id.as_str()))
        .collect::<Vec<_>>();

    let obligations_exhaust_budget = due_planned < due_total;
    let mut new_chunks_planned = 0;
    if !obligations_exhaust_budget {
        for task in input.new_chunks.into_iter().take(config.new_chunk_limit) {
            if try_pack(task, budget, &mut planned_minutes, &mut planned) {
                new_chunks_planned += 1;
            }
        }
    }

    let oldest_overdue_days = unplanned_due
        .iter()
        .filter_map(|task| task.due_day)
        .map(|day| input.today.saturating_sub(day).max(0))
        .max()
        .unwrap_or(0);
    let estimated_clear_minutes = unplanned_due
        .iter()
        .map(|task| finite_non_negative(task.estimated_minutes))
        .sum();
    let next_seven_days = seven_day_pressure(input.today, &input.future);
    let rate = (input.retention.sample_size > 0).then(|| {
        input.retention.non_again.min(input.retention.sample_size) as f32
            / input.retention.sample_size as f32
    });

    DailyQueueReport {
        planned,
        planned_minutes,
        due_total,
        due_planned,
        due_unplanned: due_total.saturating_sub(due_planned),
        backlog: BacklogReport {
            count: unplanned_due.len(),
            oldest_overdue_days,
            estimated_clear_minutes,
        },
        new_chunks_planned,
        next_seven_days,
        observed_retention: ObservedRetention {
            rate,
            sample_size: input.retention.sample_size,
        },
        retention_target: config.retention_target.clamp(0.0, 1.0),
    }
}

/// 首版冷启动耗时回退，只使用正文字数和是否首次学习。
#[must_use]
pub fn estimate_minutes(content_characters: usize, first_learning: bool) -> f32 {
    let reading_minutes = content_characters as f32 * 0.08;
    let first_learning_minutes = if first_learning { 1.5 } else { 0.0 };
    (reading_minutes + first_learning_minutes).max(0.5)
}

fn try_pack(
    task: EstimatedTask,
    budget: f32,
    used: &mut f32,
    planned: &mut Vec<EstimatedTask>,
) -> bool {
    let minutes = finite_non_negative(task.estimated_minutes);
    if *used + minutes > budget {
        return false;
    }
    *used += minutes;
    planned.push(task);
    true
}

fn seven_day_pressure(today: i64, future: &[EstimatedTask]) -> Vec<DailyPressure> {
    let mut aggregate = BTreeMap::<i64, (usize, f32)>::new();
    for task in future {
        let Some(day) = task.due_day else {
            continue;
        };
        if day <= today || day > today.saturating_add(7) {
            continue;
        }
        let entry = aggregate.entry(day).or_default();
        entry.0 += 1;
        entry.1 += finite_non_negative(task.estimated_minutes);
    }
    aggregate
        .into_iter()
        .map(|(day, (count, estimated_minutes))| DailyPressure {
            day,
            count,
            estimated_minutes,
        })
        .collect()
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
