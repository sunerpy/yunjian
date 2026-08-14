//! FSRS-6 复习排程与打字评分到四档等级的映射。

use crate::RetentionObservation;
use crate::score::TypedScore;
use fsrs::{ComputeParametersInput, FSRS, FSRSItem, FSRSReview, ItemState, MemoryState};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use yunjian_core::{Error, GradingConfig, Result};

const DESIRED_RETENTION: f32 = 0.9;
const SECONDS_PER_DAY: u64 = 86_400;
const MINIMUM_OPTIMIZATION_REVIEWS: usize = 8;

/// 云笺采用的 FSRS-6 参数；第 21 项是固定为 `0.2` 的遗忘曲线 decay。
pub const FSRS6_PARAMETERS: [f32; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835,
    0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.2,
];

/// FSRS 的四档复习等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FsrsGrade {
    /// 未能回忆，需要尽快再次复习。
    Again,
    /// 回忆困难。
    Hard,
    /// 正常回忆。
    Good,
    /// 轻松且准确地回忆。
    Easy,
}

impl FsrsGrade {
    /// 所有可由用户直接选择的等级。
    pub const ALL: [Self; 4] = [Self::Again, Self::Hard, Self::Good, Self::Easy];

    const fn rating(self) -> u32 {
        match self {
            Self::Again => 1,
            Self::Hard => 2,
            Self::Good => 3,
            Self::Easy => 4,
        }
    }

    fn from_rating(rating: u32) -> Result<Self> {
        match rating {
            1 => Ok(Self::Again),
            2 => Ok(Self::Hard),
            3 => Ok(Self::Good),
            4 => Ok(Self::Easy),
            _ => Err(Error::Recite(format!("复习库含非法等级：{rating}"))),
        }
    }
}

/// 按固定优先级把打字评分映射为唯一的 FSRS 等级。
#[must_use]
pub fn grade_typed(score: &TypedScore, first_attempt: bool, grading: &GradingConfig) -> FsrsGrade {
    if score.is_rejected || score.completeness < grading.again_completeness_below {
        FsrsGrade::Again
    } else if score.accuracy_lenient < grading.hard_accuracy_lenient_below
        || score.ops_summary.rerecitation_count > grading.hard_rerecitation_above
    {
        FsrsGrade::Hard
    } else if score.accuracy_strict >= grading.easy_accuracy_strict_at_least
        && first_attempt
        && score.ops_summary.rerecitation_count <= grading.hard_rerecitation_above
    {
        FsrsGrade::Easy
    } else {
        FsrsGrade::Good
    }
}

/// 一首作品当前持久化的复习状态。
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewState {
    /// 语料中的稳定作品标识。
    pub stable_id: String,
    /// FSRS 当前记忆稳定度。
    pub stability: f32,
    /// FSRS 当前记忆难度。
    pub difficulty: f32,
    /// 下次到期的 Unix 日序号。
    pub due_day: i64,
    /// 最近复习的 Unix 日序号。
    pub last_review_day: i64,
    /// 最近一次排出的间隔天数。
    pub scheduled_days: u32,
    /// 最近一次由用户或打字映射提交的等级。
    pub last_grade: FsrsGrade,
}

/// 一次正式复习的单次提交能力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTicket {
    /// 复习库内的票据标识。
    pub id: i64,
    /// 本票据允许提交的联片稳定标识。
    pub stable_id: String,
    /// 本票据绑定的 Unix 日序号。
    pub review_day: i64,
}

/// 当日内再学习的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PracticeOutcome {
    /// 本次无提示回忆通过。
    Passed,
    /// 本次无提示回忆未通过。
    Failed,
}

/// 一次待完成的当日内再学习任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelearningTicket {
    /// 复习库内的任务标识。
    pub id: i64,
    /// 所属联片稳定标识。
    pub stable_id: String,
    /// 应出现的 Unix 秒时间戳。
    pub due_at: i64,
}

/// 正式提交后的 FSRS 状态和可选再学习任务。
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSubmission {
    /// 唯一一次 FSRS 推进产生的状态。
    pub review: ReviewState,
    /// Again/Hard 时产生的十分钟再学习任务。
    pub relearning: Option<RelearningTicket>,
}

impl ReviewState {
    fn memory(&self) -> MemoryState {
        MemoryState {
            stability: self.stability,
            difficulty: self.difficulty,
        }
    }
}

/// 使用独立可写 SQLite 文件保存用户复习状态的排程器。
pub struct Scheduler {
    connection: Connection,
    fsrs: FSRS,
}

impl Scheduler {
    /// 打开或创建应用复习库。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        initialize_schema(&connection)?;
        let parameters = load_parameters(&connection)?.unwrap_or(FSRS6_PARAMETERS.to_vec());
        let fsrs = FSRS::new(&parameters).map_err(fsrs_error)?;
        Ok(Self { connection, fsrs })
    }

    /// 返回今天已经到期的全部作品状态。
    pub fn due_today(&self) -> Result<Vec<ReviewState>> {
        self.due_on(unix_day_now())
    }

    /// 返回指定 Unix 日序号时已经到期的全部作品状态。
    pub fn due_on(&self, day: i64) -> Result<Vec<ReviewState>> {
        let mut statement = self.connection.prepare(
            "SELECT stable_id, stability, difficulty, due_day, last_review_day, \
             scheduled_days, last_grade FROM review_state WHERE due_day <= ?1 \
             ORDER BY due_day, stable_id",
        )?;
        let rows = statement.query_map([day], review_state_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// 读取一首作品的当前复习状态。
    pub fn state(&self, stable_id: &str) -> Result<Option<ReviewState>> {
        self.connection
            .query_row(
                "SELECT stable_id, stability, difficulty, due_day, last_review_day, \
                 scheduled_days, last_grade FROM review_state WHERE stable_id = ?1",
                [stable_id],
                review_state_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    /// 返回在指定 Unix 秒时间点已触发且尚未完成的再学习任务。
    pub fn pending_relearning_at(&self, unix_seconds: i64) -> Result<Vec<RelearningTicket>> {
        let mut statement = self.connection.prepare(
            "SELECT id, stable_id, due_at FROM relearning
             WHERE completed_at IS NULL AND due_at <= ?1 ORDER BY due_at, id",
        )?;
        let rows = statement.query_map([unix_seconds], |row| {
            Ok(RelearningTicket {
                id: row.get(0)?,
                stable_id: row.get(1)?,
                due_at: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// 返回从指定日序号起、确实在到期后提交的正式复习保持率样本。
    pub fn retention_observation_since(&self, first_day: i64) -> Result<RetentionObservation> {
        let (non_again, sample_size) = self.connection.query_row(
            "SELECT COALESCE(SUM(CASE WHEN rating <> 1 THEN 1 ELSE 0 END), 0), COUNT(*)
             FROM review_log WHERE reviewed_day >= ?1 AND was_due = 1",
            [first_day],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        Ok(RetentionObservation {
            non_again: usize::try_from(non_again)
                .map_err(|_| Error::Recite(format!("保持率成功样本数超出平台范围：{non_again}")))?,
            sample_size: usize::try_from(sample_size)
                .map_err(|_| Error::Recite(format!("保持率样本数超出平台范围：{sample_size}")))?,
        })
    }

    /// 保存仅用于预算装箱的预计耗时，不改变 FSRS 状态。
    pub fn set_estimated_minutes(&self, stable_id: &str, minutes: f32) -> Result<()> {
        validate_stable_id(stable_id)?;
        if !minutes.is_finite() || minutes <= 0.0 {
            return Err(Error::Recite("预计耗时必须是大于零的有限分钟数".to_owned()));
        }
        self.connection.execute(
            "INSERT INTO task_estimate(stable_id, estimated_minutes) VALUES (?1, ?2)
             ON CONFLICT(stable_id) DO UPDATE SET estimated_minutes = excluded.estimated_minutes",
            params![stable_id, minutes],
        )?;
        Ok(())
    }

    /// 读取预算装箱耗时；旧排程没有记录时返回 `None`。
    pub fn estimated_minutes(&self, stable_id: &str) -> Result<Option<f32>> {
        self.connection
            .query_row(
                "SELECT estimated_minutes FROM task_estimate WHERE stable_id = ?1",
                [stable_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    /// 以当前日期提交一次用户选择或打字映射得到的等级。
    pub fn review(&mut self, stable_id: &str, grade: FsrsGrade) -> Result<ReviewState> {
        self.review_at(stable_id, grade, unix_day_now())
    }

    /// 以指定 Unix 日序号提交等级，供导入历史和确定性测试使用。
    pub fn review_at(
        &mut self,
        stable_id: &str,
        grade: FsrsGrade,
        review_day: i64,
    ) -> Result<ReviewState> {
        validate_stable_id(stable_id)?;
        let fsrs = &self.fsrs;
        let transaction = self.connection.transaction()?;
        let state = review_in_transaction(&transaction, fsrs, stable_id, grade, review_day)?;
        transaction.commit()?;
        Ok(state)
    }

    /// 为一次独立回答签发单次提交票据。
    pub fn issue_review_ticket_at(
        &mut self,
        stable_id: &str,
        review_day: i64,
        issued_at: i64,
    ) -> Result<ReviewTicket> {
        validate_stable_id(stable_id)?;
        self.connection.execute(
            "INSERT INTO review_ticket(stable_id, review_day, issued_at) VALUES (?1, ?2, ?3)",
            params![stable_id, review_day, issued_at],
        )?;
        Ok(ReviewTicket {
            id: self.connection.last_insert_rowid(),
            stable_id: stable_id.to_owned(),
            review_day,
        })
    }

    /// 消费一次能力票据并原子推进 FSRS；同一票据第二次提交必然失败。
    pub fn submit_review_ticket_at(
        &mut self,
        ticket: &ReviewTicket,
        grade: FsrsGrade,
        review_day: i64,
        submitted_at: i64,
    ) -> Result<ReviewSubmission> {
        if ticket.review_day != review_day {
            return Err(Error::Recite("复习票据与提交日期不一致".to_owned()));
        }
        let fsrs = &self.fsrs;
        let transaction = self.connection.transaction()?;
        let claimed = transaction.execute(
            "UPDATE review_ticket SET submitted_at = ?1
             WHERE id = ?2 AND stable_id = ?3 AND review_day = ?4 AND submitted_at IS NULL",
            params![submitted_at, ticket.id, ticket.stable_id, review_day],
        )?;
        if claimed != 1 {
            return Err(Error::Recite("复习票据不存在或已经提交".to_owned()));
        }
        let review =
            review_in_transaction(&transaction, fsrs, &ticket.stable_id, grade, review_day)?;
        let relearning = if matches!(grade, FsrsGrade::Again | FsrsGrade::Hard) {
            Some(insert_relearning(
                &transaction,
                &ticket.stable_id,
                submitted_at.saturating_add(600),
                0,
            )?)
        } else {
            None
        };
        transaction.commit()?;
        Ok(ReviewSubmission { review, relearning })
    }

    /// 记录一次再学习练习，不调用 FSRS，也不改变 `due_day`。
    pub fn record_relearning_at(
        &mut self,
        relearning_id: &i64,
        outcome: PracticeOutcome,
        completed_at: i64,
    ) -> Result<Option<RelearningTicket>> {
        let transaction = self.connection.transaction()?;
        let pending = transaction
            .query_row(
                "SELECT stable_id, stage FROM relearning WHERE id = ?1 AND completed_at IS NULL",
                [relearning_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u8>(1)?)),
            )
            .optional()?;
        let Some((stable_id, stage)) = pending else {
            return Err(Error::Recite("再学习任务不存在或已经完成".to_owned()));
        };
        transaction.execute(
            "UPDATE relearning SET completed_at = ?1, passed = ?2 WHERE id = ?3",
            params![
                completed_at,
                outcome == PracticeOutcome::Passed,
                relearning_id
            ],
        )?;
        transaction.execute(
            "INSERT INTO practice_event(stable_id, occurred_at, kind, passed)
             VALUES (?1, ?2, 'relearning', ?3)",
            params![stable_id, completed_at, outcome == PracticeOutcome::Passed],
        )?;
        let next = match (stage, outcome) {
            (0, PracticeOutcome::Passed) => Some(insert_relearning(
                &transaction,
                &stable_id,
                completed_at.saturating_add(3_600),
                1,
            )?),
            (1, PracticeOutcome::Passed) => None,
            (_, PracticeOutcome::Failed) => Some(insert_relearning(
                &transaction,
                &stable_id,
                completed_at.saturating_add(600),
                0,
            )?),
            _ => return Err(Error::Recite(format!("复习库含非法再学习阶段：{stage}"))),
        };
        transaction.commit()?;
        Ok(next)
    }

    /// 返回正式 FSRS 复习记录数。
    pub fn review_count(&self) -> Result<usize> {
        count_rows(&self.connection, "review_log")
    }

    /// 返回不推进 FSRS 的练习事件数。
    pub fn practice_event_count(&self) -> Result<usize> {
        count_rows(&self.connection, "practice_event")
    }

    /// 历史量达到训练底线时优化并持久化参数；数据不足时返回 `None`。
    pub fn optimize_parameters(&mut self) -> Result<Option<Vec<f32>>> {
        let histories = load_histories(&self.connection)?;
        let review_count = histories.values().map(Vec::len).sum::<usize>();
        if review_count < MINIMUM_OPTIMIZATION_REVIEWS {
            return Ok(None);
        }
        let train_set = histories
            .into_values()
            .map(|reviews| FSRSItem { reviews })
            .collect();
        let mut parameters = fsrs::compute_parameters(ComputeParametersInput {
            train_set,
            ..ComputeParametersInput::default()
        })
        .map_err(fsrs_error)?;
        if parameters.len() != FSRS6_PARAMETERS.len() {
            return Err(Error::Recite(format!(
                "FSRS 参数数量应为 21，实际为 {}",
                parameters.len()
            )));
        }
        parameters[20] = FSRS6_PARAMETERS[20];
        self.fsrs = FSRS::new(&parameters).map_err(fsrs_error)?;
        store_parameters(&self.connection, &parameters)?;
        Ok(Some(parameters))
    }
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS review_state(
             stable_id TEXT PRIMARY KEY NOT NULL,
             stability REAL NOT NULL,
             difficulty REAL NOT NULL,
             due_day INTEGER NOT NULL,
             last_review_day INTEGER NOT NULL,
             scheduled_days INTEGER NOT NULL,
             last_grade INTEGER NOT NULL CHECK(last_grade BETWEEN 1 AND 4)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS review_log(
             id INTEGER PRIMARY KEY,
             stable_id TEXT NOT NULL,
             reviewed_day INTEGER NOT NULL,
             rating INTEGER NOT NULL CHECK(rating BETWEEN 1 AND 4),
             delta_days INTEGER NOT NULL,
             was_due INTEGER CHECK(was_due IN (0, 1))
         );
         CREATE INDEX IF NOT EXISTS review_log_stable_id_idx
             ON review_log(stable_id, reviewed_day, id);
         CREATE TABLE IF NOT EXISTS fsrs_parameter(
             position INTEGER PRIMARY KEY CHECK(position BETWEEN 0 AND 20),
             value REAL NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS review_ticket(
             id INTEGER PRIMARY KEY,
             stable_id TEXT NOT NULL,
             review_day INTEGER NOT NULL,
             issued_at INTEGER NOT NULL,
             submitted_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS relearning(
             id INTEGER PRIMARY KEY,
             stable_id TEXT NOT NULL,
             due_at INTEGER NOT NULL,
             stage INTEGER NOT NULL CHECK(stage IN (0, 1)),
             completed_at INTEGER,
             passed INTEGER CHECK(passed IN (0, 1))
         );
         CREATE INDEX IF NOT EXISTS relearning_due_idx
             ON relearning(completed_at, due_at, id);
         CREATE TABLE IF NOT EXISTS practice_event(
             id INTEGER PRIMARY KEY,
             stable_id TEXT NOT NULL,
             occurred_at INTEGER NOT NULL,
             kind TEXT NOT NULL,
             passed INTEGER NOT NULL CHECK(passed IN (0, 1))
         );
         CREATE TABLE IF NOT EXISTS task_estimate(
             stable_id TEXT PRIMARY KEY NOT NULL,
             estimated_minutes REAL NOT NULL CHECK(estimated_minutes > 0)
         );",
    )?;
    let has_was_due = connection
        .prepare("PRAGMA table_info(review_log)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == "was_due");
    if !has_was_due {
        connection.execute(
            "ALTER TABLE review_log ADD COLUMN was_due INTEGER CHECK(was_due IN (0, 1))",
            [],
        )?;
    }
    Ok(())
}

fn validate_stable_id(stable_id: &str) -> Result<()> {
    if stable_id.trim().is_empty() {
        return Err(Error::Recite("stable_id 不得为空".to_owned()));
    }
    Ok(())
}

fn count_rows(connection: &Connection, table: &str) -> Result<usize> {
    let count = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })?;
    usize::try_from(count).map_err(|_| Error::Recite(format!("{table} 行数超出平台范围：{count}")))
}

fn review_in_transaction(
    transaction: &Transaction<'_>,
    fsrs: &FSRS,
    stable_id: &str,
    grade: FsrsGrade,
    review_day: i64,
) -> Result<ReviewState> {
    validate_stable_id(stable_id)?;
    let previous = transaction
        .query_row(
            "SELECT stable_id, stability, difficulty, due_day, last_review_day, \
             scheduled_days, last_grade FROM review_state WHERE stable_id = ?1",
            [stable_id],
            review_state_from_row,
        )
        .optional()?;
    let days_elapsed = previous
        .as_ref()
        .map(|state| elapsed_days(state.last_review_day, review_day))
        .unwrap_or(0);
    let was_due = previous
        .as_ref()
        .is_some_and(|state| state.due_day <= review_day);
    let next_states = fsrs
        .next_states(
            previous.as_ref().map(ReviewState::memory),
            DESIRED_RETENTION,
            days_elapsed,
        )
        .map_err(fsrs_error)?;
    let next = select_state(next_states, grade);
    let scheduled_days = rounded_interval(next.interval);
    let state = ReviewState {
        stable_id: stable_id.to_owned(),
        stability: next.memory.stability,
        difficulty: next.memory.difficulty,
        due_day: review_day.saturating_add(i64::from(scheduled_days)),
        last_review_day: review_day,
        scheduled_days,
        last_grade: grade,
    };
    transaction.execute(
        "INSERT INTO review_state(
            stable_id, stability, difficulty, due_day, last_review_day,
            scheduled_days, last_grade
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(stable_id) DO UPDATE SET
            stability = excluded.stability,
            difficulty = excluded.difficulty,
            due_day = excluded.due_day,
            last_review_day = excluded.last_review_day,
            scheduled_days = excluded.scheduled_days,
            last_grade = excluded.last_grade",
        params![
            state.stable_id,
            state.stability,
            state.difficulty,
            state.due_day,
            state.last_review_day,
            state.scheduled_days,
            state.last_grade.rating(),
        ],
    )?;
    transaction.execute(
        "INSERT INTO review_log(stable_id, reviewed_day, rating, delta_days, was_due)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![stable_id, review_day, grade.rating(), days_elapsed, was_due],
    )?;
    Ok(state)
}

fn insert_relearning(
    transaction: &Transaction<'_>,
    stable_id: &str,
    due_at: i64,
    stage: u8,
) -> Result<RelearningTicket> {
    transaction.execute(
        "INSERT INTO relearning(stable_id, due_at, stage) VALUES (?1, ?2, ?3)",
        params![stable_id, due_at, stage],
    )?;
    Ok(RelearningTicket {
        id: transaction.last_insert_rowid(),
        stable_id: stable_id.to_owned(),
        due_at,
    })
}

fn review_state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewState> {
    let rating = row.get::<_, u32>(6)?;
    let last_grade = FsrsGrade::from_rating(rating).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(ReviewState {
        stable_id: row.get(0)?,
        stability: row.get(1)?,
        difficulty: row.get(2)?,
        due_day: row.get(3)?,
        last_review_day: row.get(4)?,
        scheduled_days: row.get(5)?,
        last_grade,
    })
}

fn select_state(states: fsrs::NextStates, grade: FsrsGrade) -> ItemState {
    match grade {
        FsrsGrade::Again => states.again,
        FsrsGrade::Hard => states.hard,
        FsrsGrade::Good => states.good,
        FsrsGrade::Easy => states.easy,
    }
}

fn rounded_interval(interval: f32) -> u32 {
    interval.round().max(1.0).min(u32::MAX as f32) as u32
}

fn elapsed_days(previous: i64, current: i64) -> u32 {
    current
        .saturating_sub(previous)
        .clamp(0, i64::from(u32::MAX)) as u32
}

fn unix_day_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / SECONDS_PER_DAY) as i64)
        .unwrap_or(0)
}

fn load_histories(connection: &Connection) -> Result<BTreeMap<String, Vec<FSRSReview>>> {
    let mut statement = connection.prepare(
        "SELECT stable_id, rating, delta_days FROM review_log ORDER BY stable_id, reviewed_day, id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            FSRSReview {
                rating: row.get(1)?,
                delta_t: row.get(2)?,
            },
        ))
    })?;
    let mut histories = BTreeMap::<String, Vec<FSRSReview>>::new();
    for row in rows {
        let (stable_id, review) = row?;
        histories.entry(stable_id).or_default().push(review);
    }
    Ok(histories)
}

fn load_parameters(connection: &Connection) -> Result<Option<Vec<f32>>> {
    let mut statement =
        connection.prepare("SELECT position, value FROM fsrs_parameter ORDER BY position")?;
    let values = statement
        .query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, f32>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() != FSRS6_PARAMETERS.len()
        || values
            .iter()
            .enumerate()
            .any(|(expected, (actual, value))| expected as u32 != *actual || !value.is_finite())
    {
        return Err(Error::Recite(
            "复习库中的 FSRS 参数不是完整的 21 项有限值".to_owned(),
        ));
    }
    Ok(Some(values.into_iter().map(|(_, value)| value).collect()))
}

fn store_parameters(connection: &Connection, parameters: &[f32]) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM fsrs_parameter", [])?;
    {
        let mut statement =
            transaction.prepare("INSERT INTO fsrs_parameter(position, value) VALUES (?1, ?2)")?;
        for (position, value) in parameters.iter().enumerate() {
            statement.execute(params![position as u32, value])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn fsrs_error(error: fsrs::FSRSError) -> Error {
    Error::Recite(format!("FSRS 排程失败：{error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{OpsSummary, RelativeRhythm, VoicePracticeFeedback};

    #[test]
    fn schedule_source_has_no_voice_automatic_grading_path() {
        let source = include_str!("schedule.rs")
            .split_once(concat!("#[cfg(", "test)]"))
            .expect("定位测试模块边界")
            .0;
        assert!(!source.contains("VoicePracticeFeedback"));
        assert!(!source.contains("RelativeRhythm"));
        assert!(!source.contains("spoke"));
        assert!(!source.contains("pause_count"));

        let feedback = VoicePracticeFeedback::new(true, 0, RelativeRhythm::Similar);
        assert!(feedback.spoke);
    }

    #[test]
    fn rerecitation_threshold_is_configurable_and_caps_the_grade() {
        let grading = GradingConfig {
            hard_rerecitation_above: 1,
            ..GradingConfig::default()
        };
        let score = TypedScore {
            completeness: 1.0,
            accuracy_strict: 1.0,
            accuracy_lenient: 1.0,
            fluency: 1.0,
            is_rejected: false,
            ops_summary: OpsSummary {
                rerecitation_count: 2,
                ..OpsSummary::default()
            },
        };
        assert_eq!(grade_typed(&score, true, &grading), FsrsGrade::Hard);
    }
}
