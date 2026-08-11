//! 用户查询归一化与物理查询计划选择。

use crate::{CorpusHandle, DerivedState, Result, is_punctuation};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

const TRIGRAM_CHARS: usize = 3;
const WHOLE_LINE_MIN_CHARS: usize = 5;

/// 两字候选路径使用的 SQL。
///
/// `ngram.gram` 必须是首个约束，随后才回表核验正文；否则两字查询会退化成扫描全部诗。
pub const NGRAM_CANDIDATES_SQL: &str = "SELECT p.stable_id FROM ngram AS n \
JOIN poem AS p ON p.stable_id = n.stable_id \
WHERE n.gram = ?1 AND p.body LIKE ?2";

/// 归一化之后选出的物理查询计划。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryPlan {
    /// 归一化后没有正文字符，不执行 SQL。
    Empty,
    /// 1-2 字先查辅助 n-gram 候选表，再回表核验。
    NgramCandidates {
        /// 用于候选表等值查找的 1/2 字 gram。
        gram: String,
        /// 回表核验正文的 `LIKE` 模式。
        like_pattern: String,
    },
    /// 走 FTS5 trigram `MATCH`；表达式不得带前缀通配符 `*`。
    Match {
        /// 传给 FTS5 `MATCH` 的短语表达式。
        expression: String,
    },
    /// 走 FTS5 trigram 约束的 `LIKE`；索引路径不得发出 `ESCAPE`。
    Like {
        /// 由 FTS5 trigram 约束的 `LIKE` 模式。
        pattern: String,
    },
    /// 索引无法形成约束，调用方必须展示警告。
    FullScan {
        /// 无法形成索引约束的 `LIKE` 模式。
        pattern: String,
        /// 必须展示给调用方的退化原因。
        warning: String,
    },
    /// 由元数据检索模块走普通 B-tree。
    Meta {
        /// 供 B-tree 元数据查询使用的归一化文本。
        normalized: String,
    },
}

impl QueryPlan {
    /// 与 `tests/queries.toml` 的 `expect_plan` 取值对齐。
    #[must_use]
    pub const fn contract_name(&self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::NgramCandidates { .. } => "Ngram",
            Self::Match { .. } => "Match",
            Self::Like { .. } => "Like",
            Self::FullScan { .. } => "FullScan",
            Self::Meta { .. } => "Meta",
        }
    }

    /// 需要向调用方展示的退化警告。
    #[must_use]
    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::FullScan { warning, .. } => Some(warning),
            _ => None,
        }
    }
}

/// 按语料内的 `variant_map` 归一化用户输入。
pub fn normalize_query(handle: &CorpusHandle, query: &str) -> Result<String> {
    let connection = handle.connect()?;
    let mut rewrite =
        connection.prepare_cached("SELECT dst_char FROM variant_map WHERE src_char = ?1")?;
    let mut normalized = String::with_capacity(query.len());
    for character in query.trim().chars() {
        if character.is_whitespace() || is_stripped_query_punctuation(character) {
            continue;
        }
        let replacement = rewrite
            .query_row([character.to_string()], |row| row.get::<_, String>(0))
            .optional()?;
        if let Some(replacement) = replacement {
            normalized.push_str(&replacement);
        } else {
            normalized.push(character);
        }
    }
    Ok(normalized)
}

/// 为正文查询选择长度敏感的物理路径。
pub fn plan_query(handle: &CorpusHandle, query: &str) -> Result<QueryPlan> {
    let normalized = normalize_query(handle, query)?;
    if normalized.is_empty() {
        return Ok(QueryPlan::Empty);
    }

    let has_wildcard = normalized.contains(['%', '_']);
    if has_wildcard {
        let pattern = contains_pattern(&normalized);
        if max_literal_run(&normalized) < TRIGRAM_CHARS {
            return Ok(full_scan(
                pattern,
                "查询模式没有连续三个字面字符，trigram 索引无法形成约束",
            ));
        }
        return Ok(QueryPlan::Like { pattern });
    }

    let length = normalized.chars().count();
    if length < TRIGRAM_CHARS {
        return Ok(match handle.derived() {
            DerivedState::Ready { .. } => QueryPlan::NgramCandidates {
                gram: normalized.clone(),
                like_pattern: contains_pattern(&normalized),
            },
            DerivedState::Unavailable { reason } => full_scan(
                contains_pattern(&normalized),
                &format!("辅助 n-gram 索引不可用：{reason}"),
            ),
        });
    }
    if length == TRIGRAM_CHARS {
        return Ok(QueryPlan::Match {
            expression: phrase_expression(&normalized),
        });
    }
    if handle.index_detail_mode() == "full" && length >= WHOLE_LINE_MIN_CHARS {
        return Ok(QueryPlan::Match {
            expression: phrase_expression(&normalized),
        });
    }
    Ok(QueryPlan::Like {
        pattern: contains_pattern(&normalized),
    })
}

/// 为已由调用方识别出的元数据查询生成显式计划。
pub fn plan_metadata_query(handle: &CorpusHandle, query: &str) -> Result<QueryPlan> {
    let normalized = normalize_query(handle, query)?;
    if normalized.is_empty() {
        Ok(QueryPlan::Empty)
    } else {
        Ok(QueryPlan::Meta { normalized })
    }
}

/// 把字面量安全嵌进 `LIKE` 模式。
///
/// 该辅助函数只供“字面量查询”使用；黄金契约中显式出现的 `%` / `_` 是模式语法，
/// 由 [`plan_query`] 保留其通配语义。
#[must_use]
pub fn escape_like_literal(literal: &str) -> String {
    let mut escaped = String::with_capacity(literal.len());
    for character in literal.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn is_stripped_query_punctuation(character: char) -> bool {
    is_punctuation(character) && !matches!(character, '%' | '_' | '·')
}

fn contains_pattern(normalized: &str) -> String {
    format!("%{normalized}%")
}

fn phrase_expression(normalized: &str) -> String {
    format!("\"{normalized}\"")
}

fn max_literal_run(pattern: &str) -> usize {
    pattern
        .split(['%', '_'])
        .map(|segment| segment.chars().count())
        .max()
        .unwrap_or(0)
}

fn full_scan(pattern: String, warning: &str) -> QueryPlan {
    QueryPlan::FullScan {
        pattern,
        warning: warning.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CorpusConfig, DerivedState, SCHEMA_VERSION};
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

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

    fn fixture(detail_mode: &str, broken_poem_schema: bool) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-query-plan-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建查询 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path, detail_mode, broken_poem_schema);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开查询 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path, detail_mode: &str, broken_poem_schema: bool) {
        let connection = Connection::open(path).expect("创建查询 fixture 数据库");
        let poem_schema = if broken_poem_schema {
            "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL);"
        } else {
            "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);"
        };
        connection
            .execute_batch(&format!(
                "{poem_schema}
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
                 );"
            ))
            .expect("创建查询 fixture schema");
        if !broken_poem_schema {
            connection
                .execute(
                    "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
                    params!["fixture:jingyesi", "床前明月光，疑是地上霜。"],
                )
                .expect("写 fixture 诗");
        }
        for (source, destination) in [("國", "国"), ("舉", "举"), ("頭", "头")] {
            connection
                .execute(
                    "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                    params![source, destination],
                )
                .expect("写 variant_map");
        }
        connection
            .execute(
                "INSERT INTO corpus_meta VALUES \
                 (1, ?1, 'fixture-v1', '2026-08-11T00:00:00Z', ?2, ?3, \
                  'first_launch', '10k', 'ok')",
                params![SCHEMA_VERSION, i64::from(!broken_poem_schema), detail_mode],
            )
            .expect("写 corpus_meta");
        connection.close().expect("关闭查询 fixture 数据库");
    }

    #[test]
    fn normalization_is_corpus_driven_and_keeps_pattern_syntax() {
        let fixture = fixture("full", false);
        assert_eq!(
            normalize_query(&fixture.handle, "  國，破\n山河在  ").expect("归一化"),
            "国破山河在"
        );
        assert_eq!(
            normalize_query(&fixture.handle, "明%光_·").expect("归一化模式"),
            "明%光_·"
        );
        assert_eq!(escape_like_literal("%_\\"), "\\%\\_\\\\");
    }

    #[test]
    fn length_and_runtime_detail_mode_choose_all_three_routes() {
        let full = fixture("full", false);
        let none = fixture("none", false);
        let cases = [
            ("李", "Ngram"),
            ("明月", "Ngram"),
            ("明月光", "Match"),
            ("床前明月光", "Match"),
        ];
        for (query, expected) in cases {
            let actual = plan_query(&full.handle, query).expect("规划 full 查询");
            assert_eq!(actual.contract_name(), expected, "query={query}");
        }
        assert_eq!(
            plan_query(&none.handle, "床前明月光")
                .expect("规划 detail=none 查询")
                .contract_name(),
            "Like"
        );
    }

    #[test]
    fn empty_patterns_and_unconstrained_patterns_are_explicit() {
        let fixture = fixture("full", false);
        assert_eq!(
            plan_query(&fixture.handle, "，。？！")
                .expect("规划空查询")
                .contract_name(),
            "Empty"
        );
        let plan = plan_query(&fixture.handle, "明%光").expect("规划无约束模式");
        assert_eq!(plan.contract_name(), "FullScan");
        assert!(
            plan.warning().is_some_and(|warning| !warning.is_empty()),
            "FullScan 必须携带调用方可见警告：{plan:?}"
        );
    }

    #[test]
    fn unavailable_derived_indexes_degrade_short_queries_with_the_reason() {
        let fixture = fixture("full", true);
        let reason = match fixture.handle.derived() {
            DerivedState::Unavailable { reason } => reason,
            state => panic!("损坏 fixture 应让派生不可用，实际 {state:?}"),
        };
        let plan = plan_query(&fixture.handle, "明月").expect("规划退化查询");
        assert_eq!(plan.contract_name(), "FullScan");
        assert!(
            plan.warning()
                .is_some_and(|warning| warning.contains(reason)),
            "退化警告必须解释派生失败原因：{plan:?}"
        );
    }

    #[test]
    fn generated_match_terms_have_no_prefix_wildcard() {
        let fixture = fixture("full", false);
        let QueryPlan::Match { expression } =
            plan_query(&fixture.handle, "明月光").expect("规划 MATCH")
        else {
            panic!("三字查询必须走 MATCH");
        };
        assert!(!expression.contains('*'), "trigram MATCH 不得追加 *");
        assert_eq!(expression, "\"明月光\"");
    }

    #[test]
    fn detail_none_whole_line_uses_indexed_like_and_returns_the_fixture_poem() {
        let fixture = fixture("none", false);
        let plan = plan_query(&fixture.handle, "床前明月光").expect("规划 detail=none 整句");
        let connection = fixture.handle.connect().expect("打开只读查询连接");
        let stable_id = match plan {
            QueryPlan::Like { pattern } => connection.query_row(
                "SELECT p.stable_id
                 FROM poem_fts AS f
                 JOIN poem AS p ON p.rowid = f.rowid
                 WHERE f.body LIKE ?1",
                [pattern],
                |row| row.get::<_, String>(0),
            ),
            QueryPlan::Match { expression } => connection.query_row(
                "SELECT p.stable_id
                 FROM poem_fts AS f
                 JOIN poem AS p ON p.rowid = f.rowid
                 WHERE f.poem_fts MATCH ?1",
                [expression],
                |row| row.get::<_, String>(0),
            ),
            other => panic!("detail=none 的整句只能走 LIKE 或错误的 MATCH，实际 {other:?}"),
        }
        .expect("执行 detail=none 整句黄金查询");
        assert_eq!(stable_id, "fixture:jingyesi");
    }

    #[test]
    fn two_character_sql_uses_the_ngram_constraint_without_scanning_poem() {
        let fixture = fixture("full", false);
        let connection = fixture.handle.connect().expect("打开只读查询连接");
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {NGRAM_CANDIDATES_SQL}"))
            .expect("prepare explain");
        let lines = statement
            .query_map(params!["明月", "%明月%"], |row| row.get::<_, String>(3))
            .expect("执行 explain")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("收集 explain");
        assert!(
            lines
                .iter()
                .any(|line| { line.contains("ngram_gram_idx") && line.contains("gram=?") }),
            "两字路径必须带 gram=? 覆盖索引约束：{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("SCAN p")),
            "两字路径不得扫描 poem：{lines:?}"
        );
    }

    #[test]
    fn metadata_plans_stay_out_of_the_full_text_router() {
        let fixture = fixture("full", false);
        assert_eq!(
            plan_metadata_query(&fixture.handle, "  國破  ").expect("规划元数据查询"),
            QueryPlan::Meta {
                normalized: "国破".to_owned()
            }
        );
    }
}
