//! 八条代表性查询的墙钟 p50/p95 实测。
//!
//! 八条按方案点名的形态选取，每条对应一类真实用户输入，且**物理路径各不相同**——
//! 八条都走同一条索引就测不出任何东西：
//!
//! | 查询 | 形态 | 要证明的事 |
//! | --- | --- | --- |
//! | `two_char_ngram` | 两字「明月」 | trigram 推不出约束的那一类，靠 n-gram 覆盖索引救回 |
//! | `three_char_match` | 三字 MATCH | trigram 的正常工作区间 |
//! | `full_line_like` | 整句 LIKE | 长字面串靠 trigram 约束，不退化成全扫 |
//! | `author_lookup` | 作者等值 | 普通 B-tree，绝不碰 FTS |
//! | `first_line_prefix` | 首句前缀 | 有序区间扫描而不是 `LIKE 'x%'` |
//! | `rhyme_group_join` | 韵部连接 | `poem_rhyme_group` 上的连接吃到索引 |
//! | `tag_filter` | 标签过滤 | 规范化多对多表而不是拼接字符串列 |
//! | `cold_open_first_query` | 冷启动后首查 | 用户点开应用的第一次检索，**不预热** |
//!
//! # 绑定值为什么要从库里解析出来，而不是写死
//!
//! 写死一个作者名或韵部名，在某个规模上可能一条都命中不到，那量到的就是「空结果集
//! 有多快」——一个与产品无关的数字，而且它总是很快，会让报告偏乐观。所以除了
//! 「明月」这类必然存在的正文探针，等值类探针的绑定值一律取**当前库里最高频的那个
//! 值**（[`resolve`]）：候选集最大，是这类查询的近似最坏情形，且在三个规模之间
//! 可比（形态相同，只是数据变多）。
//!
//! 解析不到值时（例如标签表在当前 schema 阶段还没有数据来源）该条探针仍然照跑并
//! 如实记为零命中，同时在 `kind` 里写明「表为空」——**不假装它测到了东西**。

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use super::{QueryMeasurement, REPRESENTATIVE_QUERY_COUNT, WARMUP, round3};

/// 两字探针字。「明月」是最典型的两字输入，且在唐宋语料里高频，因此候选集大——
/// 这正是要压力测试的情形；换一个低频词会让数字偏乐观。
///
/// 正文归一后是简体（`NormalizedRecord::body` 是规范简体），所以简体探针在
/// **归一后**的正文上必然可命中，即便上游 全唐诗 存的是繁体「明月」。
const TWO_CHAR: &str = "明月";
const THREE_CHAR: &str = "明月光";

const COLD_PROBE_ID: &str = "cold_open_first_query";

struct Probe {
    id: &'static str,
    kind: String,
    sql: &'static str,
    binds: Vec<String>,
}

pub(super) fn measure_all(
    connection: &Connection,
    db_path: &Path,
    repeats: usize,
) -> Result<Vec<QueryMeasurement>> {
    let probes = resolve(connection)?;
    debug_assert_eq!(probes.len(), REPRESENTATIVE_QUERY_COUNT);
    let mut out = Vec::with_capacity(probes.len());
    for probe in &probes {
        if probe.id == COLD_PROBE_ID {
            out.push(measure_cold(db_path, probe, repeats)?);
        } else {
            out.push(measure_warm(connection, probe, repeats)?);
        }
    }
    Ok(out)
}

/// 组装八条探针，等值类的绑定值取库里最高频的实际值。
fn resolve(connection: &Connection) -> Result<Vec<Probe>> {
    let ngram_sql = "SELECT p.stable_id FROM ngram n JOIN poem p ON p.stable_id = n.stable_id \
                     WHERE n.gram = ?1 AND p.body LIKE ?2";
    let author = most_frequent(connection, "SELECT author FROM poem GROUP BY author")?;
    let rhyme = most_frequent_pair(
        connection,
        "SELECT rhyme_book, rhyme_group FROM poem_rhyme_group GROUP BY rhyme_book, rhyme_group",
    )?;
    let tag = most_frequent(connection, "SELECT tag FROM poem_tag GROUP BY tag")?;
    let prefix = first_line_prefix(connection)?;
    let full_line = longest_shared_first_line(connection)?;

    Ok(vec![
        Probe {
            id: "two_char_ngram",
            kind: format!("两字查询「{TWO_CHAR}」经 n-gram 覆盖索引"),
            sql: ngram_sql,
            binds: vec![TWO_CHAR.to_owned(), format!("%{TWO_CHAR}%")],
        },
        Probe {
            id: "three_char_match",
            kind: format!("三字 FTS5 MATCH（trigram）「{THREE_CHAR}」"),
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.poem_fts MATCH ?1",
            binds: vec![format!("\"{THREE_CHAR}\"")],
        },
        Probe {
            id: "full_line_like",
            kind: describe_equality(
                "整句 LIKE（trigram 约束）",
                full_line.as_deref(),
                "poem.first_line",
            ),
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.body LIKE ?1",
            binds: vec![format!("%{}%", full_line.unwrap_or_default())],
        },
        Probe {
            id: "author_lookup",
            kind: describe_equality("作者等值（B-tree）", author.as_deref(), "author"),
            sql: "SELECT stable_id FROM poem WHERE author = ?1",
            binds: vec![author.unwrap_or_default()],
        },
        Probe {
            id: "first_line_prefix",
            kind: describe_equality(
                "首句前缀（有序区间，非 LIKE）",
                prefix.as_ref().map(|(low, _)| low.as_str()),
                "first_line",
            ),
            sql: "SELECT stable_id FROM poem WHERE first_line >= ?1 AND first_line < ?2",
            binds: match prefix {
                Some((low, high)) => vec![low, high],
                None => vec![String::new(), String::new()],
            },
        },
        Probe {
            id: "rhyme_group_join",
            kind: describe_equality(
                "韵部连接",
                rhyme.as_ref().map(|(_, group)| group.as_str()),
                "poem_rhyme_group",
            ),
            sql: "SELECT g.poem_id FROM poem_rhyme_group g \
                  JOIN poem p ON p.stable_id = g.poem_id \
                  WHERE g.rhyme_book = ?1 AND g.rhyme_group = ?2",
            binds: match rhyme {
                Some((book, group)) => vec![book, group],
                None => vec![String::new(), String::new()],
            },
        },
        Probe {
            id: "tag_filter",
            kind: describe_equality("标签过滤（规范化多对多表）", tag.as_deref(), "poem_tag"),
            sql: "SELECT poem_id FROM poem_tag WHERE tag = ?1",
            binds: vec![tag.unwrap_or_default()],
        },
        Probe {
            id: COLD_PROBE_ID,
            kind: format!("冷启动后首查（每轮重开连接，不预热）「{TWO_CHAR}」"),
            sql: ngram_sql,
            binds: vec![TWO_CHAR.to_owned(), format!("%{TWO_CHAR}%")],
        },
    ])
}

/// 说明这条等值探针绑定了什么值；表为空时**明说表为空**，不让读报告的人把
/// 「空结果集很快」误读成「这条查询很快」。
fn describe_equality(label: &str, value: Option<&str>, table: &str) -> String {
    match value {
        Some(value) => format!("{label}，绑定库内最高频值「{value}」"),
        None => format!("{label}，{table} 表为空，本条为零命中基线"),
    }
}

/// 取某个分组查询里计数最大的那个值。
///
/// 用最高频值而不是任取一个：等值查询的成本正比于命中行数，最高频值给出这类查询在
/// 当前语料上的近似最坏情形。取任意值会让数字随抽样漂移，三个规模之间就没法比。
fn most_frequent(connection: &Connection, group_sql: &str) -> Result<Option<String>> {
    let counted = counted_sql(group_sql);
    let mut statement = connection.prepare(&counted)?;
    let mut rows = statement.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get::<_, String>(0)?)),
        None => Ok(None),
    }
}

fn most_frequent_pair(
    connection: &Connection,
    group_sql: &str,
) -> Result<Option<(String, String)>> {
    let counted = counted_sql(group_sql);
    let mut statement = connection.prepare(&counted)?;
    let mut rows = statement.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
        None => Ok(None),
    }
}

/// 把一条 `SELECT cols FROM t GROUP BY cols` 改写成「按行数降序取第一组」。
fn counted_sql(group_sql: &str) -> String {
    format!("{group_sql} ORDER BY count(*) DESC LIMIT 1")
}

/// 首句前缀的下界与上界。
///
/// 上界是「把前缀最后一个字符加一」，于是 `>= low AND < high` 等价于前缀匹配，
/// 且用得上 B-tree 的有序性；`LIKE 'x%'` 在某些排序规则下会退化成扫描。
fn first_line_prefix(connection: &Connection) -> Result<Option<(String, String)>> {
    let mut statement = connection.prepare(
        "SELECT substr(first_line, 1, 2) AS prefix FROM poem \
         WHERE length(first_line) >= 2 GROUP BY prefix ORDER BY count(*) DESC LIMIT 1",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let low: String = row.get(0)?;
    Ok(Some((low.clone(), upper_bound(&low))))
}

/// 取一条**真实存在且被多首诗共用**的整句，作为整句 LIKE 探针的绑定值。
///
/// 为什么不写死「床前明月光」：上游 全唐诗 的 静夜思 底本作「牀前看月光」，而且入库
/// 正文是繁体经归一后的简体——任何手写的整句都可能一条都命中不到，那量到的就是
/// 「空结果集有多快」，一个总是很快、与产品无关的数字。
///
/// 取被最多首共用的首句，因为整句 LIKE 的成本正比于命中行数：共用最多的那句给出这类
/// 查询在当前语料上的近似最坏情形，且三个规模之间可比。要求长度 >= 5 是为了让
/// trigram 真的能推出约束（少于 3 字就退化成全扫，那测的是另一条路径）。
fn longest_shared_first_line(connection: &Connection) -> Result<Option<String>> {
    let mut statement = connection.prepare(
        "SELECT first_line FROM poem WHERE length(first_line) >= 5 \
         GROUP BY first_line ORDER BY count(*) DESC, length(first_line) DESC LIMIT 1",
    )?;
    let mut rows = statement.query([])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get::<_, String>(0)?)),
        None => Ok(None),
    }
}

fn upper_bound(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    match chars.pop() {
        None => String::new(),
        Some(last) => {
            let bumped = char::from_u32(last as u32 + 1).unwrap_or(last);
            chars.push(bumped);
            chars.into_iter().collect()
        }
    }
}

fn measure_warm(
    connection: &Connection,
    probe: &Probe,
    repeats: usize,
) -> Result<QueryMeasurement> {
    let plan = explain(connection, probe)?;
    for _ in 0..WARMUP {
        run_once(connection, probe)?;
    }
    let mut samples = Vec::with_capacity(repeats);
    let mut hits = 0;
    for _ in 0..repeats {
        let started = Instant::now();
        hits = run_once(connection, probe)?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(summarize(probe, hits, samples, plan))
}

/// 冷启动首查：每一轮都**重开连接**，因此每个样本都是「刚打开语料库的第一次检索」。
///
/// 重开连接清掉的是 SQLite 自己的 page cache；操作系统的 page cache 清不掉（需要
/// root 写 `/proc/sys/vm/drop_caches`），所以这个数是「进程内冷、系统内热」的下界。
/// 报告必须按这个口径解读，不能当成真正的开机首查延迟。
fn measure_cold(db_path: &Path, probe: &Probe, repeats: usize) -> Result<QueryMeasurement> {
    let mut samples = Vec::with_capacity(repeats);
    let mut hits = 0;
    let mut plan = Vec::new();
    for round in 0..repeats {
        let started = Instant::now();
        let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("冷启动重开失败 {}", db_path.display()))?;
        connection.pragma_update(None, "query_only", true)?;
        hits = run_once(&connection, probe)?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        if round == 0 {
            plan = explain(&connection, probe)?;
        }
    }
    Ok(summarize(probe, hits, samples, plan))
}

fn summarize(
    probe: &Probe,
    hits: usize,
    mut samples: Vec<f64>,
    plan: Vec<String>,
) -> QueryMeasurement {
    samples.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    QueryMeasurement {
        id: probe.id.to_owned(),
        kind: probe.kind.clone(),
        sql_shape: probe.sql.split_whitespace().collect::<Vec<_>>().join(" "),
        hits,
        p50_ms: round3(percentile(&samples, 0.50)),
        p95_ms: round3(percentile(&samples, 0.95)),
        explain_query_plan: plan,
    }
}

fn run_once(connection: &Connection, probe: &Probe) -> Result<usize> {
    let mut statement = connection.prepare(probe.sql)?;
    let rows = statement.query_map(binds(probe).as_slice(), |row| row.get::<_, String>(0))?;
    let mut count = 0;
    for row in rows {
        row?;
        count += 1;
    }
    Ok(count)
}

fn explain(connection: &Connection, probe: &Probe) -> Result<Vec<String>> {
    let sql = format!("EXPLAIN QUERY PLAN {}", probe.sql);
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(binds(probe).as_slice(), |row| row.get::<_, String>(3))?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

fn binds(probe: &Probe) -> Vec<&dyn rusqlite::ToSql> {
    probe
        .binds
        .iter()
        .map(|bind| bind as &dyn rusqlite::ToSql)
        .collect()
}

/// 最近秩百分位：`ceil(p * n) - 1`。
///
/// 不做插值——插值出的数字在样本里并不存在，而门禁判定的应当是真实观测到的延迟。
/// 与 `index_spike` 的定义一致，两份报告的 p95 才可以互相比对。
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)]
}
