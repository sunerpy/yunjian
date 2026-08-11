//! 首启构建之后，37 条黄金查询契约必须逐条与随包时相同。
//!
//! # 这个文件为什么在 `tests/` 而不是 `src/`
//!
//! 它证明的是**跨 crate 的**性质：随包工件由 `yunjian-corpus` 产出，三张检索结构由
//! `yunjian-core` 在首启派生，而契约要在「两步都做完」之后才成立。放在集成测试里，
//! 它只能通过两个 crate 的公开 API，与应用真正走的路径一致。
//!
//! # 它守的是什么
//!
//! 「不随包 `ngram` / `poem_fts` / `poem_last_char`」这个决定的全部风险在于它可能是
//! 一次功能缩减。本文件把「不是缩减」变成可执行断言：先建出随包形态（此时候选表
//! 根本不存在），再跑首启构建，然后逐条跑满 37 条契约并核对物理路径。
//! `without_the_first_launch_build_...` 是它的可证伪对照——把结构拿掉，两字查询
//! 连准备都过不去。

use rusqlite::{Connection, params, params_from_iter};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const VERDICT_JSON: &[u8] = include_bytes!("../../../corpus/reports/index-mode.json");
const QUERIES_TOML: &str = include_str!("../../yunjian-core/tests/queries.toml");
const POEMS_TOML: &str = include_str!("../../yunjian-core/tests/fixtures/poems.toml");

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Verdict {
    schema_version: u32,
    chosen_mode: String,
    ngram_aux_enabled: bool,
    justification: String,
    selection_rule: serde_json::Value,
    environment: serde_json::Value,
    corpus: serde_json::Value,
    contract: serde_json::Value,
    results: serde_json::Value,
    scale_projection: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema_version: u32,
    fixture_file: String,
    #[serde(rename = "query")]
    queries: Vec<ContractEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractEntry {
    id: String,
    query: String,
    class: String,
    expect_plan: String,
    expect_top_id: String,
    expect_min_hits: usize,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    schema_version: u32,
    #[serde(rename = "variant")]
    variants: Vec<FixtureVariant>,
    #[serde(rename = "poem")]
    poems: Vec<FixturePoem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureVariant {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixturePoem {
    stable_id: String,
    title: String,
    author: String,
    dynasty: String,
    ci_tune: String,
    body: String,
    first_line: String,
    last_chars: Vec<String>,
    rhyme_book: String,
    rhyme_group: String,
    tags: Vec<String>,
    #[allow(dead_code)]
    note: String,
}

fn verdict() -> Verdict {
    serde_json::from_slice(VERDICT_JSON).expect("正式 index-mode verdict 应可解析")
}

fn contract() -> Contract {
    toml::from_str(QUERIES_TOML).expect("黄金查询契约应可解析")
}

fn fixtures() -> Fixtures {
    toml::from_str(POEMS_TOML).expect("黄金 fixture 应可解析")
}

fn golden_connection() -> (Connection, Fixtures) {
    let mut connection = Connection::open_in_memory().expect("open in-memory database");
    connection
        .execute_batch(
            "PRAGMA page_size=4096;
             PRAGMA foreign_keys=ON;
             CREATE TABLE poem (
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 title TEXT NOT NULL,
                 author TEXT NOT NULL,
                 dynasty TEXT NOT NULL,
                 ci_tune TEXT,
                 body TEXT NOT NULL,
                 first_line TEXT NOT NULL
             );
             CREATE TABLE poem_rhyme_group (
                 poem_id TEXT NOT NULL,
                 rhyme_book TEXT NOT NULL,
                 rhyme_group TEXT NOT NULL,
                 tone TEXT NOT NULL,
                 confidence TEXT NOT NULL
             );
             CREATE TABLE poem_tag (
                 poem_id TEXT NOT NULL,
                 tag TEXT NOT NULL,
                 PRIMARY KEY (poem_id, tag)
             ) WITHOUT ROWID;
             CREATE TABLE variant_map (
                 src_char TEXT PRIMARY KEY NOT NULL,
                 dst_char TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE corpus_meta (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
                 index_detail_mode TEXT NOT NULL
             );
             CREATE INDEX poem_author_idx ON poem(author);
             CREATE INDEX poem_title_idx ON poem(title);
             CREATE INDEX poem_ci_tune_idx ON poem(ci_tune);
             CREATE INDEX poem_first_line_idx ON poem(first_line);
             CREATE INDEX poem_rhyme_group_idx
                 ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id);
             CREATE INDEX poem_tag_idx ON poem_tag(tag, poem_id);",
        )
        .expect("create golden schema");

    let fixtures = fixtures();
    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, index_detail_mode) VALUES (1, ?1)",
            params![verdict().chosen_mode],
        )
        .expect("把裁决选定的形态写进 corpus_meta，首启从这里读");
    let transaction = connection.transaction().expect("begin fixture transaction");
    for variant in &fixtures.variants {
        transaction
            .execute(
                "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                params![variant.from, variant.to],
            )
            .expect("insert variant");
    }
    for poem in &fixtures.poems {
        transaction
            .execute(
                "INSERT INTO poem(stable_id, title, author, dynasty, ci_tune, body, first_line)
                 VALUES (?1, ?2, ?3, ?4, NULLIF(?5, ''), ?6, ?7)",
                params![
                    poem.stable_id,
                    poem.title,
                    poem.author,
                    poem.dynasty,
                    poem.ci_tune,
                    poem.body,
                    poem.first_line,
                ],
            )
            .expect("insert poem");
        if !poem.rhyme_group.is_empty() {
            let rhyme_book = match poem.rhyme_book.as_str() {
                "平水韵" => "pingshui",
                "词林正韵" => "cilin",
                other => panic!("fixture 含未知韵书：{other}"),
            };
            transaction
                .execute(
                    "INSERT INTO poem_rhyme_group(
                         poem_id, rhyme_book, rhyme_group, tone, confidence
                     ) VALUES (?1, ?2, ?3, 'level', 'unambiguous')",
                    params![poem.stable_id, rhyme_book, poem.rhyme_group],
                )
                .expect("insert rhyme group");
        }
        for tag in &poem.tags {
            transaction
                .execute(
                    "INSERT INTO poem_tag(poem_id, tag) VALUES (?1, ?2)",
                    params![poem.stable_id, tag],
                )
                .expect("insert tag");
        }
    }
    transaction.commit().expect("commit fixture transaction");

    // 随包工件到此为止：`poem` 与元数据齐备，三张检索结构一张都没有。
    // 下一步就是**首启**，在用户机器上跑——它从 `corpus_meta.index_detail_mode`
    // 读裁决选定的形态，与运行时走的是同一条路径。
    assert!(
        !yunjian_core::derived_indexes_present(&connection).expect("探测派生结构"),
        "随包形态不该含任何检索结构"
    );
    yunjian_core::build_derived_indexes(&mut connection).expect("首启构建派生结构");
    yunjian_core::verify_derived_indexes(&connection).expect("首启构建后派生结构必须可用");
    (connection, fixtures)
}

/// 只建随包形态、**不**跑首启构建的连接。用来证明派生结构是承重的。
fn shipped_only_connection() -> Connection {
    let (connection, _) = golden_connection();
    connection
        .execute_batch(
            "DROP TABLE poem_fts;
             DROP INDEX ngram_gram_idx;
             DROP TABLE ngram;
             DROP INDEX poem_last_char_idx;
             DROP TABLE poem_last_char;",
        )
        .expect("退回随包形态");
    connection
}

fn variants(connection: &Connection) -> BTreeMap<char, char> {
    let mut statement = connection
        .prepare("SELECT src_char, dst_char FROM variant_map ORDER BY src_char")
        .expect("prepare variant query");
    statement
        .query_map([], |row| {
            let source = row.get::<_, String>(0)?;
            let target = row.get::<_, String>(1)?;
            Ok((
                source.chars().next().expect("non-empty source"),
                target.chars().next().expect("non-empty target"),
            ))
        })
        .expect("query variants")
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .expect("collect variants")
}

fn normalize(query: &str, variants: &BTreeMap<char, char>) -> String {
    query
        .trim()
        .chars()
        .filter(|character| !is_punctuation(*character) || matches!(character, '%' | '_' | '·'))
        .map(|character| variants.get(&character).copied().unwrap_or(character))
        .collect()
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '《'
                | '》'
                | '〈'
                | '〉'
                | '【'
                | '】'
                | '〔'
                | '〕'
                | '—'
                | '…'
        )
}

fn collect_ids(connection: &Connection, sql: &str, bindings: &[String]) -> Vec<String> {
    let mut statement = connection.prepare(sql).expect("prepare contract query");
    statement
        .query_map(params_from_iter(bindings), |row| row.get::<_, String>(0))
        .expect("execute contract query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect contract results")
}

fn execute_contract(
    connection: &Connection,
    entry: &ContractEntry,
    normalized: &str,
) -> Vec<String> {
    let like = format!("%{normalized}%");
    match entry.expect_plan.as_str() {
        "Ngram" => collect_ids(
            connection,
            "SELECT p.stable_id
             FROM ngram AS n
             JOIN poem AS p ON p.stable_id = n.stable_id
             WHERE n.gram = ?1 AND p.body LIKE ?2",
            &[normalized.to_owned(), like],
        ),
        "Match" => collect_ids(
            connection,
            "SELECT p.stable_id
             FROM poem_fts
             JOIN poem AS p ON p.rowid = poem_fts.rowid
             WHERE poem_fts MATCH ?1",
            &[format!("\"{normalized}\"")],
        ),
        "Like" => collect_ids(
            connection,
            "SELECT p.stable_id
             FROM poem_fts
             JOIN poem AS p ON p.rowid = poem_fts.rowid
             WHERE poem_fts.body LIKE ?1",
            &[like],
        ),
        "Empty" => Vec::new(),
        "FullScan" => collect_ids(
            connection,
            "SELECT stable_id FROM poem WHERE body LIKE ?1",
            &[like],
        ),
        "Meta" => execute_meta_contract(connection, entry, normalized),
        other => panic!("契约 {} 含未知计划 {other}", entry.id),
    }
}

fn execute_meta_contract(
    connection: &Connection,
    entry: &ContractEntry,
    normalized: &str,
) -> Vec<String> {
    match entry.class.as_str() {
        "two_char_author" => collect_ids(
            connection,
            "SELECT stable_id FROM poem WHERE author = ?1",
            &[normalized.to_owned()],
        ),
        "title_lookup" | "ci_tune_title_lookup" => collect_ids(
            connection,
            "SELECT stable_id FROM poem WHERE title = ?1",
            &[normalized.to_owned()],
        ),
        "ci_tune_lookup" => collect_ids(
            connection,
            "SELECT stable_id FROM poem WHERE ci_tune = ?1",
            &[normalized.to_owned()],
        ),
        "first_line_prefix" => collect_ids(
            connection,
            "SELECT stable_id FROM poem WHERE first_line LIKE ?1",
            &[format!("{normalized}%")],
        ),
        "last_char_lookup" => collect_ids(
            connection,
            "SELECT DISTINCT poem_id FROM poem_last_char WHERE ch = ?1",
            &[normalized.to_owned()],
        ),
        "rhyme_group_query" => collect_ids(
            connection,
            "SELECT poem_id FROM poem_rhyme_group
             WHERE rhyme_group = ?1 AND confidence != 'unresolved'",
            &[normalized.to_owned()],
        ),
        "tag_query" => collect_ids(
            connection,
            "SELECT poem_id FROM poem_tag WHERE tag = ?1",
            &[normalized.to_owned()],
        ),
        other => panic!("契约 {} 含未知 Meta 类别 {other}", entry.id),
    }
}

fn explain(connection: &Connection, sql: &str, bindings: &[String]) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare query plan");
    statement
        .query_map(params_from_iter(bindings), |row| row.get::<_, String>(3))
        .expect("explain query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect query plan")
}

#[test]
fn fts_schema_matches_the_binding_verdict_and_has_no_shadow_content_table() {
    let (connection, _) = golden_connection();
    let verdict = verdict();
    assert_eq!(verdict.schema_version, 1);
    assert!(verdict.ngram_aux_enabled);
    assert!(!verdict.justification.is_empty());
    assert!(verdict.selection_rule.is_object());
    assert!(verdict.environment.is_object());
    assert!(verdict.corpus.is_object());
    assert!(verdict.contract.is_object());
    assert!(verdict.results.is_array());
    assert!(verdict.scale_projection.is_array());

    yunjian_core::verify_derived_indexes(&connection)
        .expect("首启派生出来的结构必须与 corpus_meta 记录的裁决形态一致");

    let ddl: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='poem_fts'",
            [],
            |row| row.get(0),
        )
        .expect("read poem_fts DDL");
    assert!(ddl.contains("body"));
    assert!(ddl.contains("content='poem'"));
    assert!(ddl.contains("content_rowid='rowid'"));
    assert!(ddl.contains("tokenize='trigram'"));
    assert!(ddl.contains(&format!("detail={}", verdict.chosen_mode)));
    assert!(!ddl.contains("remove_diacritics"));

    let shadow_content_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_list WHERE name='poem_fts_content'",
            [],
            |row| row.get(0),
        )
        .expect("count shadow content tables");
    assert_eq!(shadow_content_tables, 0);

    let fts_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema
             WHERE type='table' AND lower(sql) LIKE '%using fts5(%'",
            [],
            |row| row.get(0),
        )
        .expect("count FTS tables");
    assert_eq!(fts_tables, 1);

    connection
        .execute(
            "INSERT INTO poem_fts(poem_fts) VALUES('integrity-check')",
            [],
        )
        .expect("FTS integrity-check must succeed");
}

#[test]
fn the_first_launch_derivation_reproduces_the_fixture_declared_last_chars() {
    // `poem.last_chars` JSON 随包，`poem_last_char` 首启派生——两者是同一份数据的两种
    // 形态。若派生规则与当初写 JSON 的规则不一致，尾字检索与展示会错位，而这种偏差
    // 不会报错，只会让某些查询少召回几条。逐首逐句核对。
    let (connection, fixtures) = golden_connection();
    for poem in &fixtures.poems {
        let mut statement = connection
            .prepare("SELECT ch FROM poem_last_char WHERE poem_id=?1 ORDER BY line_index")
            .expect("prepare");
        let derived = statement
            .query_map(params![poem.stable_id], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect");
        assert_eq!(
            derived, poem.last_chars,
            "{} 的首启派生尾字与 fixture 声明的不一致",
            poem.stable_id
        );
    }
}

#[test]
fn fts_two_character_plan_uses_the_ngram_covering_index() {
    let (connection, _) = golden_connection();
    let sql = "SELECT p.stable_id
               FROM ngram AS n
               JOIN poem AS p ON p.stable_id = n.stable_id
               WHERE n.gram = ?1 AND p.body LIKE ?2";
    let plan = explain(&connection, sql, &["明月".to_owned(), "%明月%".to_owned()]);
    assert!(
        plan.iter()
            .any(|line| line.contains("USING COVERING INDEX ngram_gram_idx (gram=?)")),
        "query plan did not use ngram_gram_idx: {plan:?}"
    );
    assert!(
        plan.iter().all(|line| !line.contains("SCAN poem")),
        "query plan scanned poem: {plan:?}"
    );
}

#[test]
fn fts_all_thirty_seven_golden_contracts_return_the_expected_top_hit() {
    let (connection, fixtures) = golden_connection();
    let contract = contract();
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.fixture_file, "fixtures/poems.toml");
    assert_eq!(fixtures.schema_version, 1);
    assert_eq!(contract.queries.len(), 37);
    assert_eq!(
        contract
            .queries
            .iter()
            .map(|entry| entry.class.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        18
    );

    let variants = variants(&connection);
    let fixture_rank = fixtures
        .poems
        .iter()
        .enumerate()
        .map(|(rank, poem)| (poem.stable_id.as_str(), rank))
        .collect::<BTreeMap<_, _>>();
    for entry in &contract.queries {
        assert!(entry.note.chars().count() >= 10);
        let normalized = normalize(&entry.query, &variants);
        let mut hits = execute_contract(&connection, entry, &normalized);
        hits.sort_by_key(|id| fixture_rank.get(id.as_str()).copied().unwrap_or(usize::MAX));
        hits.dedup();
        assert!(
            hits.len() >= entry.expect_min_hits,
            "{} expected at least {} hits, got {}: {hits:?}",
            entry.id,
            entry.expect_min_hits,
            hits.len()
        );
        if entry.expect_min_hits > 0 {
            assert_eq!(
                hits.first().map(String::as_str),
                Some(entry.expect_top_id.as_str()),
                "{} returned the wrong top hit: {hits:?}",
                entry.id
            );
        }
    }
}

#[test]
fn without_the_first_launch_build_the_two_character_contracts_cannot_run_at_all() {
    // 这一条是上一条的可证伪对照：若把候选表从随包工件里拿掉却什么都不坏，那说明
    // 「首启构建」是装饰而不是必要步骤。走 Ngram 计划的条目全是两字查询，而 FTS5
    // trigram 在三字以下推不出任何约束，所以它们只能靠候选表。
    let connection = shipped_only_connection();
    let contract = contract();
    let ngram_entries = contract
        .queries
        .iter()
        .filter(|entry| entry.expect_plan == "Ngram")
        .collect::<Vec<_>>();
    assert!(
        !ngram_entries.is_empty(),
        "契约里必须有走候选表的条目，否则这条对照没有意义"
    );
    for entry in ngram_entries {
        assert_eq!(
            entry.query.chars().count(),
            2,
            "{} 走候选表却不是两字查询，前提变了",
            entry.id
        );
        let error = connection
            .prepare(
                "SELECT p.stable_id FROM ngram AS n
                 JOIN poem AS p ON p.stable_id = n.stable_id
                 WHERE n.gram = ?1 AND p.body LIKE ?2",
            )
            .expect_err("没有候选表时这条查询根本无法准备");
        assert!(
            error.to_string().contains("ngram"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn fts_verdict_drift_is_rejected_instead_of_being_silently_ignored() {
    // 裁决的牙齿在拆包后仍然在，只是位置变了：构建期把 `chosen_mode` 刻进
    // `corpus_meta.index_detail_mode`，首启照它建。篡改那一列即形态漂移，必须被拒。
    let (connection, _) = golden_connection();
    let verdict = verdict();
    let mismatched = match verdict.chosen_mode.as_str() {
        "full" => "column",
        _ => "full",
    };
    connection
        .execute(
            "UPDATE corpus_meta SET index_detail_mode=?1",
            params![mismatched],
        )
        .expect("篡改记录的形态");
    let error = yunjian_core::verify_derived_indexes(&connection)
        .expect_err("a verdict that disagrees with the built index must fail");
    let message = error.to_string();
    assert!(message.contains(mismatched), "unexpected error: {message}");
    assert!(
        message.contains(&verdict.chosen_mode),
        "unexpected error: {message}"
    );
}

struct Footprint {
    fts_bytes: i64,
    shadow_content_tables: i64,
}

fn footprint_path(label: &str) -> (PathBuf, bool) {
    if let Some(directory) = std::env::var_os("YUNJIAN_FTS_EVIDENCE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).expect("create evidence directory");
        return (directory.join(format!("{label}.db")), true);
    }
    (
        std::env::temp_dir().join(format!(
            "yunjian-fts-{label}-{}-{}.db",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        )),
        false,
    )
}

fn create_footprint_database(path: &Path, external_content: bool) -> Footprint {
    if path.exists() {
        fs::remove_file(path).expect("remove old footprint database");
    }
    let mut connection = Connection::open(path).expect("open footprint database");
    connection
        .execute_batch(
            "PRAGMA page_size=4096;
             PRAGMA journal_mode=DELETE;
             CREATE TABLE poem (
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE TABLE corpus_meta (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
                 index_detail_mode TEXT NOT NULL
             );
             INSERT INTO corpus_meta(singleton, index_detail_mode) VALUES (1, 'full');",
        )
        .expect("create footprint schema");
    {
        let transaction = connection.transaction().expect("begin footprint inserts");
        for ordinal in 0..2_048 {
            let stable_id = format!("fixture:{ordinal:08}");
            let body = format!(
                "第{ordinal:04}首床前明月光疑是地上霜举头望明月低头思故乡海上生明月天涯共此时"
            );
            transaction
                .execute(
                    "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
                    params![stable_id, body],
                )
                .expect("insert footprint poem");
        }
        transaction.commit().expect("commit footprint poems");
    }

    if external_content {
        yunjian_core::build_derived_indexes(&mut connection)
            .expect("build external-content footprint");
        connection
            .execute_batch("VACUUM")
            .expect("compact external-content footprint");
    } else {
        // 对照组自己建候选表：它要量的是「不用 external-content 时 FTS 有多大」，
        // 与派生结构从哪来无关。
        connection
            .execute_batch(yunjian_core::derive::DERIVED_SCHEMA_SQL)
            .expect("create footprint derived schema");
        populate_test_ngrams(&mut connection);
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE poem_fts USING fts5(
                     body,
                     tokenize='trigram',
                     detail=full
                 );
                 INSERT INTO poem_fts(rowid, body) SELECT rowid, body FROM poem;
                 INSERT INTO poem_fts(poem_fts) VALUES('optimize');
                 INSERT INTO poem_fts(poem_fts) VALUES('integrity-check');
                 VACUUM;",
            )
            .expect("build internal-content footprint");
    }

    let shadow_content_tables = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_list WHERE name='poem_fts_content'",
            [],
            |row| row.get(0),
        )
        .expect("count footprint shadow tables");
    let fts_bytes = connection
        .query_row(
            "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name GLOB 'poem_fts*'",
            [],
            |row| row.get(0),
        )
        .expect("measure FTS footprint");
    drop(connection);
    Footprint {
        fts_bytes,
        shadow_content_tables,
    }
}

fn populate_test_ngrams(connection: &mut Connection) {
    let rows = {
        let mut statement = connection
            .prepare("SELECT stable_id, body FROM poem ORDER BY stable_id")
            .expect("prepare footprint poem query");
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query footprint poems")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect footprint poems")
    };
    let transaction = connection.transaction().expect("begin footprint ngrams");
    {
        let mut insert = transaction
            .prepare("INSERT INTO ngram(gram, stable_id) VALUES (?1, ?2)")
            .expect("prepare footprint ngram insert");
        for (stable_id, body) in rows {
            let characters = body
                .chars()
                .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
                .collect::<Vec<_>>();
            let mut grams = BTreeSet::new();
            for (index, character) in characters.iter().enumerate() {
                grams.insert(character.to_string());
                if let Some(next) = characters.get(index + 1) {
                    grams.insert(format!("{character}{next}"));
                }
            }
            for gram in grams {
                insert
                    .execute(params![gram, stable_id])
                    .expect("insert footprint ngram");
            }
        }
    }
    transaction.commit().expect("commit footprint ngrams");
}

#[test]
fn fts_external_content_eliminates_the_shadow_copy_and_reduces_bytes() {
    let (external_path, preserve_external) = footprint_path("external-content");
    let (internal_path, preserve_internal) = footprint_path("internal-content");
    let external = create_footprint_database(&external_path, true);
    let internal = create_footprint_database(&internal_path, false);

    assert_eq!(external.shadow_content_tables, 0);
    assert_eq!(internal.shadow_content_tables, 1);
    assert!(
        internal.fts_bytes > external.fts_bytes,
        "internal={} external={}",
        internal.fts_bytes,
        external.fts_bytes
    );
    if !preserve_external {
        fs::remove_file(external_path).ok();
    }
    if !preserve_internal {
        fs::remove_file(internal_path).ok();
    }
}
