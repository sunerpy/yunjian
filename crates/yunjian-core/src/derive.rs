//! 首启在本机派生的检索结构：`ngram`、`poem_fts`、`poem_last_char`。
//!
//! # 为什么它们不随包
//!
//! 三者共同的性质是**都由 `poem.body` 确定性派生**：给定同一份 `poem` 表，任何机器上
//! 派生出来的内容逐行相同。于是它们属于运行时而非工件——随包等于把一份可以在本机
//! 几分钟算出来的东西塞进每个用户的下载里。
//!
//! 实测（唐宋 474162 首）：三者移出后随包库 4464 MiB -> 604 MiB，
//! gzip 446 MiB -> 213 MiB，从超预算 1.49 倍变为预算内。
//!
//! **这不是功能缩减。** 首启构建完成后，`crates/yunjian-core/tests/queries.toml` 的
//! 37 条契约与随包时逐条相同，包括「明月」这类两字查询走 `ngram_gram_idx` 覆盖索引的
//! 物理路径——FTS5 trigram 在三字以下推不出任何约束，那条路径只能靠候选表。
//!
//! # 索引裁决的语义
//!
//! `corpus/reports/index-mode.json` 描述的是**运行时应有的索引形态**，不是随包工件的
//! 形态。构建期把 `chosen_mode` 写进 `corpus_meta.index_detail_mode`，首启按那一列
//! 建 `poem_fts`。裁决因此仍然有牙齿：改掉它就改掉了运行时真正建出来的索引，37 条
//! 契约立刻变红。
//!
//! # 原子性
//!
//! 每一步的灌数据都在一个事务里完成，且三张结构在开工前被整体丢弃再重建，所以构建
//! 被打断时留下的是「缺表」而不是「表在、内容只灌了一半」——前者会被
//! [`derived_indexes_present`] 判为未完成并重跑，后者会被误判为已完成。

use crate::text::{content_chars, is_punctuation};
use crate::{Error, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// `ngram` 与 `poem_last_char` 的 DDL。签入的这一份是唯一事实来源；本模块内唯一
/// 格式化出来的 DDL 是 `poem_fts`，因为它的 `detail` 模式来自 `corpus_meta`。
pub const DERIVED_SCHEMA_SQL: &str = include_str!("../schema-derived.sql");

/// 首启派生出来的结构名。随包工件里这些都**不应存在**。
pub const DERIVED_TABLES: [&str; 3] = ["ngram", "poem_fts", "poem_last_char"];

/// 首启派生的阶段。
///
/// 之所以是**四**个而不是三个：读 `poem` 表本身在唐宋规模上就要读完 519 MiB 正文，
/// 把它算进第一张表的时间会让「n-gram 慢」这个结论掺进一段与 n-gram 无关的 I/O。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeriveStep {
    /// 读出 `poem` 的 `stable_id` 与 `body`。
    Scan,
    /// 灌 `ngram`。实测占总时长 85%（487.5 s / 571.8 s）。
    Ngram,
    /// 灌 `poem_last_char`。
    LastChar,
    /// 建 `poem_fts` 并 rebuild / optimize / integrity-check。
    Fts,
}

impl DeriveStep {
    /// 供 UI 直接展示的中文步骤名。
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Scan => "读取诗词正文",
            Self::Ngram => "构建候选索引",
            Self::LastChar => "构建尾字索引",
            Self::Fts => "构建全文索引",
        }
    }
}

/// 一次派生进度事件。
///
/// `total == 0` 表示该步的总量未知，UI 应当显示不确定进度而不是 0%。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeriveProgress {
    /// 当前派生阶段。
    pub step: DeriveStep,
    /// 当前阶段已处理的作品数。
    pub done: u64,
    /// 当前阶段的作品总数；未知时为零。
    pub total: u64,
}

impl DeriveProgress {
    /// 已完成比例，`None` 表示总量未知。
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        (self.total > 0).then(|| (self.done as f64 / self.total as f64).clamp(0.0, 1.0))
    }
}

/// 逐首汇报的步长。
///
/// 不逐首汇报：唐宋 474162 首乘四步等于近两百万次回调，而回调对面可能是 FFI 边界或
/// 一次界面重绘。按 1024 首汇报把它压到每步四百多次，同时仍然足够让进度条平滑。
/// 每一步结束时**另外补一次精确值**，所以最终显示不会停在 473k/474k。
const PROGRESS_STRIDE: u64 = 1024;

/// 一次首启构建的实测量。
///
/// 逐步分开记录，因为首启进度要按步显示，而「哪一步慢」在只有总时长时无法回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedBuildStats {
    /// 扫描的作品数。
    pub poems: u64,
    /// 写入的 1/2 字候选记录数。
    pub grams: u64,
    /// 写入的逐句末字记录数。
    pub last_chars: u64,
    /// 构建候选索引的耗时。
    pub ngram_elapsed: Duration,
    /// 构建尾字索引的耗时。
    pub last_char_elapsed: Duration,
    /// 构建全文索引的耗时。
    pub fts_elapsed: Duration,
    /// 全部派生步骤的总耗时。
    pub elapsed: Duration,
}

/// 三张结构是否都已就位。
///
/// 判据是**表与索引都在**：只有表没有索引时查询会退化成全表扫描，那和没建是一个
/// 后果，所以不能算已完成。
pub fn derived_indexes_present(connection: &Connection) -> Result<bool> {
    for table in DERIVED_TABLES {
        if count_object(connection, "table", table)? != 1 {
            return Ok(false);
        }
    }
    for index in ["ngram_gram_idx", "poem_last_char_idx"] {
        if count_object(connection, "index", index)? != 1 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 在**可写**的语料库副本上派生三张结构，回传实测量。
///
/// 传入的连接必须是读写的：随包工件在应用数据目录里落地成一份可写副本后才走这里，
/// 只读打开的那一份不可能建表。
///
/// 幂等：已存在的派生结构会被丢弃后重建，因此中断后重跑是安全的。
///
/// 需要进度回调时用 [`build_derived_indexes_with_progress`]——唐宋规模实测 571.8 s，
/// 没有反馈的十分钟等价于界面卡死。
pub fn build_derived_indexes(connection: &mut Connection) -> Result<DerivedBuildStats> {
    build_derived_indexes_with_progress(connection, &mut |_| {})
}

/// 同 [`build_derived_indexes`]，但逐步汇报进度。
///
/// 回调按固定批次节流，且每步结束补一次精确值。
/// `Fts` 一步只有首尾两次事件：`INSERT INTO poem_fts(poem_fts) VALUES('rebuild')` 在
/// SQLite 内部完成，中途拿不到任何可汇报的量——**刻意不伪造一个匀速动画**，那会把
/// 「还剩多久」这件事从「不知道」变成「错」。
pub fn build_derived_indexes_with_progress(
    connection: &mut Connection,
    progress: &mut dyn FnMut(DeriveProgress),
) -> Result<DerivedBuildStats> {
    let started = Instant::now();
    let detail_mode = index_detail_mode(connection)?;

    let poems = {
        progress(DeriveProgress {
            step: DeriveStep::Scan,
            done: 0,
            total: 0,
        });
        let mut statement =
            connection.prepare("SELECT stable_id, body FROM poem ORDER BY stable_id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let total = rows.len() as u64;
        progress(DeriveProgress {
            step: DeriveStep::Scan,
            done: total,
            total,
        });
        rows
    };
    let total_poems = poems.len() as u64;

    connection.execute_batch(
        "DROP TABLE IF EXISTS poem_fts;
         DROP INDEX IF EXISTS ngram_gram_idx;
         DROP TABLE IF EXISTS ngram;
         DROP INDEX IF EXISTS poem_last_char_idx;
         DROP TABLE IF EXISTS poem_last_char;",
    )?;
    connection.execute_batch(DERIVED_SCHEMA_SQL)?;

    let ngram_started = Instant::now();
    let mut grams_written: u64 = 0;
    {
        let transaction = connection.transaction()?;
        {
            let mut insert =
                transaction.prepare("INSERT INTO ngram(gram, stable_id) VALUES (?1, ?2)")?;
            for (index, (stable_id, body)) in poems.iter().enumerate() {
                for gram in derive_grams(body) {
                    insert.execute(rusqlite::params![gram, stable_id])?;
                    grams_written += 1;
                }
                report_stride(progress, DeriveStep::Ngram, index as u64 + 1, total_poems);
            }
        }
        transaction.commit()?;
    }
    let ngram_elapsed = ngram_started.elapsed();
    progress(DeriveProgress {
        step: DeriveStep::Ngram,
        done: total_poems,
        total: total_poems,
    });

    let last_char_started = Instant::now();
    let mut last_chars_written: u64 = 0;
    {
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO poem_last_char(poem_id, line_index, ch) VALUES (?1, ?2, ?3)",
            )?;
            for (index, (stable_id, body)) in poems.iter().enumerate() {
                for (line_index, character) in derive_last_chars(body) {
                    insert.execute(rusqlite::params![
                        stable_id,
                        line_index as i64,
                        character.to_string()
                    ])?;
                    last_chars_written += 1;
                }
                report_stride(
                    progress,
                    DeriveStep::LastChar,
                    index as u64 + 1,
                    total_poems,
                );
            }
        }
        transaction.commit()?;
    }
    let last_char_elapsed = last_char_started.elapsed();
    progress(DeriveProgress {
        step: DeriveStep::LastChar,
        done: total_poems,
        total: total_poems,
    });

    let fts_started = Instant::now();
    progress(DeriveProgress {
        step: DeriveStep::Fts,
        done: 0,
        total: total_poems,
    });
    build_fts(connection, &detail_mode)?;
    let fts_elapsed = fts_started.elapsed();
    progress(DeriveProgress {
        step: DeriveStep::Fts,
        done: total_poems,
        total: total_poems,
    });

    let stats = DerivedBuildStats {
        poems: poems.len() as u64,
        grams: grams_written,
        last_chars: last_chars_written,
        ngram_elapsed,
        last_char_elapsed,
        fts_elapsed,
        elapsed: started.elapsed(),
    };
    verify_derived_indexes(connection)?;
    Ok(stats)
}

/// 按 `corpus_meta.index_detail_mode` 建 external-content 的 trigram FTS5 表。
///
/// `content='poem'` 消掉影子内容表（否则那份副本可能超过文件的一半）；
/// 不启用 `remove_diacritics`，否则 trigram 上的 LIKE/GLOB 无法走索引。
fn build_fts(connection: &Connection, detail_mode: &str) -> Result<()> {
    validate_detail_mode(detail_mode)?;
    connection.execute_batch(&format!(
        "CREATE VIRTUAL TABLE poem_fts USING fts5(
             body,
             content='poem',
             content_rowid='rowid',
             tokenize='trigram',
             detail={detail_mode}
         );
         INSERT INTO poem_fts(poem_fts) VALUES('rebuild');
         INSERT INTO poem_fts(poem_fts) VALUES('optimize');
         INSERT INTO poem_fts(poem_fts) VALUES('integrity-check');"
    ))?;
    Ok(())
}

/// 断言三张结构可用：都在、覆盖到每一首诗，且 `poem_fts` 的形态与
/// `corpus_meta.index_detail_mode` 一致。
///
/// 「表存在」不等于「内容对」——一张空表同样能通过 schema 检查，却让每条查询零命中。
pub fn verify_derived_indexes(connection: &Connection) -> Result<()> {
    if !derived_indexes_present(connection)? {
        return Err(derive_error(
            "首启派生结构不完整：`ngram` / `poem_fts` / `poem_last_char` 或其索引缺失；\
             两字查询会退化为全表扫描",
        ));
    }

    let expected = derivable_poem_count(connection)?;
    for (table, key) in [("ngram", "stable_id"), ("poem_last_char", "poem_id")] {
        let covered: i64 = connection.query_row(
            &format!("SELECT count(*) FROM (SELECT DISTINCT {key} FROM {table})"),
            [],
            |row| row.get(0),
        )?;
        if covered != expected {
            return Err(derive_error(format!(
                "{table} 只覆盖 {covered} 首，应当覆盖 {expected} 首（有正文字符的）；派生漏了记录"
            )));
        }
    }

    let ddl: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='poem_fts'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "content='poem'",
        "content_rowid='rowid'",
        "tokenize='trigram'",
    ] {
        if !ddl.contains(required) {
            return Err(derive_error(format!(
                "poem_fts DDL 缺少 `{required}`：{ddl}"
            )));
        }
    }
    if ddl.contains("remove_diacritics") {
        return Err(derive_error(
            "poem_fts 不得启用 remove_diacritics，否则 trigram LIKE/GLOB 无法使用索引",
        ));
    }
    let expected = index_detail_mode(connection)?;
    let actual = detail_mode_from_ddl(&ddl)?;
    if actual != expected {
        return Err(derive_error(format!(
            "corpus_meta 记录 detail={expected}，实际建出 detail={actual}"
        )));
    }
    let shadow: i64 = connection.query_row(
        "SELECT count(*) FROM pragma_table_list WHERE name='poem_fts_content'",
        [],
        |row| row.get(0),
    )?;
    if shadow != 0 {
        return Err(derive_error(
            "poem_fts_content 影子内容表不应存在；poem_fts 必须使用 external-content",
        ));
    }
    let fts_tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema
         WHERE type='table' AND lower(sql) LIKE '%using fts5(%'",
        [],
        |row| row.get(0),
    )?;
    if fts_tables != 1 {
        return Err(derive_error(format!(
            "语料库必须恰有一个 FTS5 表，实际为 {fts_tables} 个"
        )));
    }
    Ok(())
}

/// 从 `corpus_meta` 读运行时索引形态。
///
/// 这一列是构建期把实测裁决刻进工件的结果，所以运行时不需要（也不应该）去找仓库里的
/// 裁决文件——工件自带它该长什么样。
pub fn index_detail_mode(connection: &Connection) -> Result<String> {
    let mode: String = connection
        .query_row(
            "SELECT index_detail_mode FROM corpus_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            derive_error(format!(
                "读取 corpus_meta.index_detail_mode 失败：{error}；\
                 首启构建必须知道裁决选定的索引形态"
            ))
        })?;
    validate_detail_mode(&mode)?;
    Ok(mode)
}

/// 能派生出内容的诗条数，即正文含至少一个正文字符的那些。
///
/// 为什么不是简单的 `count(*) FROM poem`：实测唐宋集合里有 **176 首的正文就是一个
/// `。`**（上游的空记录，`body` 非空所以过了质量门禁）。它们派生不出任何候选与尾字，
/// 那是正确行为——拿总首数当期望值会让首启构建在真实语料上必然失败。
///
/// 判据仍然可证伪：用的是[`content_chars`]这同一个折叠器，所以「漏了一首有正文的诗」
/// 一定被抓到。**刻意不在 SQL 里重写一遍标点集**——两份实现会漂移，而漂移的后果正是
/// 这个函数要防的那种静默偏差。
fn derivable_poem_count(connection: &Connection) -> Result<i64> {
    let mut statement = connection.prepare("SELECT body FROM poem")?;
    let count = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|body| content_chars(body).next().is_some())
        .count();
    Ok(count as i64)
}

fn count_object(connection: &Connection, kind: &str, name: &str) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type=?1 AND name=?2",
        rusqlite::params![kind, name],
        |row| row.get(0),
    )?)
}

/// 逐首派生候选：全部 1 字与全部相邻 2 字，按首去重。
///
/// 去重按**首**而不是全局：同一首里「明」出现三次只写一行，但两首都含「明」时两行
/// 都要有——候选表的用途正是「哪些诗可能含这个片段」。
fn derive_grams(body: &str) -> BTreeSet<String> {
    let characters = content_chars(body).collect::<Vec<_>>();
    let mut grams = BTreeSet::new();
    for (index, character) in characters.iter().enumerate() {
        grams.insert(character.to_string());
        if let Some(next) = characters.get(index + 1) {
            let mut bigram = String::with_capacity(8);
            bigram.push(*character);
            bigram.push(*next);
            grams.insert(bigram);
        }
    }
    grams
}

/// 句读切分符。
///
/// 五个句读符来自韵脚推导的规则（方案 todo 18：「split each poem's body into lines on
/// `，。！？；`」），换行符是因为 `poem.body` 由构建期切好的句子 join 而成
/// （`db.rs` 写入时 `body_lines.join("\n")`）。
///
/// **两者都要**：真实语料的正文既有换行也有句读，而 fixture 正文只有句读。只按换行切
/// 会让每首诗只剩最后一个尾字，只按句读切会在没有句读的行上失效。
const SENTENCE_SEPARATORS: [char; 6] = ['\n', '，', '。', '！', '？', '；'];

/// 逐句取句尾字。
///
/// `line_index` 是**有内容的句子**的序号（连续，不留空洞），与 `poem.last_chars` JSON
/// 的下标一致——那一列是同一份数据的可回读副本，两者对不上会让尾字检索与展示错位。
fn derive_last_chars(body: &str) -> Vec<(usize, char)> {
    body.split(|character| SENTENCE_SEPARATORS.contains(&character))
        .filter_map(last_character)
        .enumerate()
        .collect()
}

fn last_character(line: &str) -> Option<char> {
    line.chars()
        .rev()
        .find(|character| !character.is_whitespace() && !is_punctuation(*character))
}

fn validate_detail_mode(detail_mode: &str) -> Result<()> {
    if matches!(detail_mode, "none" | "column" | "full") {
        return Ok(());
    }
    Err(derive_error(format!("索引 detail 模式非法：{detail_mode}")))
}

fn detail_mode_from_ddl(ddl: &str) -> Result<&'static str> {
    ["none", "column", "full"]
        .into_iter()
        .find(|mode| ddl.contains(&format!("detail={mode}")))
        .ok_or_else(|| derive_error(format!("poem_fts DDL 缺少可识别的 detail 模式：{ddl}")))
}

fn report_stride(
    progress: &mut dyn FnMut(DeriveProgress),
    step: DeriveStep,
    done: u64,
    total: u64,
) {
    if done.is_multiple_of(PROGRESS_STRIDE) {
        progress(DeriveProgress { step, done, total });
    }
}

fn derive_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

#[cfg(test)]
mod tests;
