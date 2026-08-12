//! FSRS-6 复习排程与打字评分到四档等级的映射。

use crate::score::TypedScore;
use fsrs::{ComputeParametersInput, FSRS, FSRSItem, FSRSReview, ItemState, MemoryState};
use rusqlite::{Connection, OptionalExtension, params};
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
        if stable_id.trim().is_empty() {
            return Err(Error::Recite("stable_id 不得为空".to_owned()));
        }
        let previous = self.state(stable_id)?;
        let days_elapsed = previous
            .as_ref()
            .map(|state| elapsed_days(state.last_review_day, review_day))
            .unwrap_or(0);
        let next_states = self
            .fsrs
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

        let transaction = self.connection.transaction()?;
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
            "INSERT INTO review_log(stable_id, reviewed_day, rating, delta_days)
             VALUES (?1, ?2, ?3, ?4)",
            params![stable_id, review_day, grade.rating(), days_elapsed],
        )?;
        transaction.commit()?;
        Ok(state)
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
             delta_days INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS review_log_stable_id_idx
             ON review_log(stable_id, reviewed_day, id);
         CREATE TABLE IF NOT EXISTS fsrs_parameter(
             position INTEGER PRIMARY KEY CHECK(position BETWEEN 0 AND 20),
             value REAL NOT NULL
         ) WITHOUT ROWID;",
    )?;
    Ok(())
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
