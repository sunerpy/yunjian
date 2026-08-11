use super::*;
use crate::commentary::{AcceptedCitation, CommentaryRecord, PoemRef};
use crate::model::{
    CanonicalRecord, Dynasty, Genre, LicenseClass, Provenance, ProvenanceKind, Script,
    SourceLocatorKind,
};
use crate::normalize::{NormalizedRecord, VariantRow};
use crate::quality::{
    Disposition, DispositionCounts, DispositionRow, Finding, QualityReport, ReasonCode,
};
use crate::rhyme::RhymeEntry;
use crate::rhyme_foot::RhymeConfidence;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use yunjian_core::rhyme::{RhymeBook, RhymeTone};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn temp_db(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yunjian-db-{label}-{}-{}.db",
        std::process::id(),
        NEXT_PATH.fetch_add(1, Ordering::Relaxed)
    ))
}

fn record(id: &str, title: &str, author: &str, lines: &[&str]) -> CanonicalRecord {
    let body = lines.join("\n");
    CanonicalRecord {
        stable_id: id.to_owned(),
        content_hash: format!("{id:0<16}"),
        work_group: format!("{id:0<12}"),
        edition_group: format!("e{id:0<11}"),
        source_locator: format!("fixture:{id}"),
        source_locator_kind: SourceLocatorKind::Native,
        genre: Genre::Shi,
        title: title.to_owned(),
        title_raw: title.to_owned(),
        ci_tune: None,
        author: author.to_owned(),
        dynasty: Dynasty::Tang,
        dynasty_raw: "唐".to_owned(),
        body_lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        body_original: body,
        script: Script::Simplified,
        provenance: Provenance {
            source_name: "fixture".to_owned(),
            source_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            license: "public-domain".to_owned(),
            license_class: LicenseClass::PublicDomain,
            kind: ProvenanceKind::Original,
        },
    }
}

fn normalized(record: &CanonicalRecord) -> NormalizedRecord {
    NormalizedRecord {
        stable_id: record.stable_id.clone(),
        body: record.body_lines.join("\n"),
        body_lines: record.body_lines.clone(),
        body_original: record.body_original.clone(),
        script: record.script,
    }
}

fn fixture() -> CorpusDbInput {
    let first = record(
        "1111111111111111",
        "静夜思",
        "李白",
        &["床前明月光", "疑是地上霜"],
    );
    let second = record(
        "2222222222222222",
        "春望",
        "杜甫",
        &["国破山河在", "城春草木深"],
    );
    let records = vec![second.clone(), first.clone()];
    let normalized_records = records.iter().map(normalized).collect();
    let findings = vec![Finding {
        stable_id: Some(second.stable_id.clone()),
        work_group: Some(second.work_group.clone()),
        reason_code: ReasonCode::ConversionUnstable,
        detail: "fixture finding".to_owned(),
        source: "fixture".to_owned(),
    }];
    let dispositions = vec![
        DispositionRow {
            source_locator: first.source_locator.clone(),
            stable_id: Some(first.stable_id.clone()),
            disposition: Disposition::Shipped,
        },
        DispositionRow {
            source_locator: second.source_locator.clone(),
            stable_id: Some(second.stable_id.clone()),
            disposition: Disposition::Shipped,
        },
        DispositionRow {
            source_locator: "fixture:excluded".to_owned(),
            stable_id: None,
            disposition: Disposition::Excluded,
        },
    ];
    let mut summary = BTreeMap::new();
    summary.insert("conversion_unstable".to_owned(), 1);

    CorpusDbInput {
        corpus_version: "1.2.3".to_owned(),
        shipped_scope: ShippedScope::Sample10k,
        source_manifest: br#"
[[source]]
name = "older"
retrieved_at = "2026-07-01"
[[source]]
name = "newer"
retrieved_at = "2026-08-10"
"#
        .to_vec(),
        index_verdict: br#"{
  "schema_version": 1,
  "chosen_mode": "full",
  "ngram_aux_enabled": true,
  "environment": { "sqlite_version": "3.53.2", "page_size": 4096 }
}"#
        .to_vec(),
        records,
        normalized_records,
        commentaries: vec![CommentaryRecord {
            id: "jing-ye-si-one".to_owned(),
            poem_id: first.stable_id.clone(),
            poem: PoemRef {
                author: "李白".to_owned(),
                title: "静夜思".to_owned(),
                first_line: "床前明月光".to_owned(),
            },
            text: "此诗写羁旅夜思，语近而情深。".to_owned(),
            citation: AcceptedCitation {
                work: "鹤林玉露".to_owned(),
                author: "罗大经".to_owned(),
                dynasty: Dynasty::Song,
                dynasty_raw: "宋".to_owned(),
                work_completed_by: 1252,
                source_note: "卷一，据四库全书本".to_owned(),
            },
        }],
        rhymes: vec![RhymeEntry {
            book: RhymeBook::Pingshui,
            rhyme_group: "下平七阳".to_owned(),
            tone: RhymeTone::Level,
            tone_raw: "下平声部".to_owned(),
            character: "光".to_owned(),
        }],
        poem_rhyme_groups: vec![PoemRhymeGroupRow {
            poem_id: first.stable_id.clone(),
            rhyme_book: RhymeBook::Pingshui,
            rhyme_group: "下平七阳".to_owned(),
            tone: RhymeTone::Level,
            confidence: RhymeConfidence::Unambiguous,
        }],
        variants: vec![VariantRow {
            src_char: '國',
            dst_char: '国',
        }],
        tags: vec![PoemTagRow {
            poem_id: first.stable_id.clone(),
            tag: "思乡".to_owned(),
        }],
        quality: QualityReport {
            schema_version: 1,
            input_rows: 3,
            poem_count: 2,
            counts: DispositionCounts {
                shipped: 2,
                quarantined: 0,
                excluded: 1,
            },
            summary,
            findings,
            dispositions,
        },
    }
}

fn build(path: &Path) {
    build_database(path, &fixture()).expect("fixture database should build");
}

/// 一次构建产出两个文件，清理也必须清两个——留下的审计库会被下一个同名用例
/// 当成「这一对」而掩盖掉真正的配对错误。
fn cleanup(path: &Path) {
    fs::remove_file(audit_path(path)).ok();
    fs::remove_file(path).ok();
}

#[test]
fn schema_is_the_checked_in_single_source_of_truth() {
    let path = temp_db("schema");
    build(&path);
    let conn = Connection::open(&path).expect("open built db");
    let mut statement = conn
        .prepare(
            "SELECT sql FROM sqlite_schema \
              WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
                AND name NOT LIKE 'poem_fts%' \
              ORDER BY type, name",
        )
        .expect("prepare schema query");
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query schema")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect schema");
    let expected = schema_statements(SCHEMA_SQL).expect("checked-in schema should parse");
    assert_eq!(actual, expected);
    cleanup(&path);
}

#[test]
fn the_first_launch_build_turns_a_shipped_artifact_into_a_searchable_corpus() {
    let path = temp_db("fts-integration");
    let input = fixture();
    build_database(&path, &input).expect("fixture database should build with FTS");
    let mut connection = Connection::open(&path).expect("open built db");

    // 随包工件不含任何检索结构；三张都由首启在本机派生。
    assert!(!yunjian_core::derived_indexes_present(&connection).expect("probe derived"));
    let stats = yunjian_core::build_derived_indexes(&mut connection).expect("first-launch build");
    assert!(stats.grams > 0);
    assert!(stats.last_chars > 0);
    yunjian_core::verify_derived_indexes(&connection).expect("post-first-launch verification");
    cleanup(&path);
}

#[test]
fn the_shipped_artifact_carries_no_diagnostic_or_derived_tables() {
    let path = temp_db("no-diagnostics");
    build(&path);
    let connection = Connection::open(&path).expect("open built db");
    assert_no_diagnostic_tables(&connection).expect("随包库必须无 defect/disposition/ngram");
    for table in NON_SHIPPED_TABLES {
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                params![table],
                |row| row.get(0),
            )
            .expect("probe table");
        assert_eq!(count, 0, "随包库不该有 {table} 表");
    }

    // 反过来，审计库必须真的持有那两张台账。
    let audit = Connection::open(audit_path(&path)).expect("open audit db");
    for table in ["defect", "disposition", "audit_meta"] {
        let count: i64 = audit
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                params![table],
                |row| row.get(0),
            )
            .expect("probe audit table");
        assert_eq!(count, 1, "审计库应有 {table} 表");
    }
    cleanup(&path);
}

#[test]
fn audit_path_is_derived_from_the_corpus_path_not_supplied_separately() {
    assert_eq!(
        audit_path(Path::new("/tmp/corpus.db")),
        PathBuf::from("/tmp/corpus-audit.db")
    );
    assert_eq!(
        audit_path(Path::new("corpus-tang-song.db")),
        PathBuf::from("corpus-tang-song-audit.db")
    );
}

#[test]
fn an_audit_database_from_another_build_is_rejected_even_when_the_counts_line_up() {
    let first = temp_db("pair-a");
    let second = temp_db("pair-b");
    build(&first);
    let mut other = fixture();
    other.corpus_version = "9.9.9".to_owned();
    build_database(&second, &other).expect("second build");

    // 两次构建的三个计数逐一相同，只有 corpus_version 不同——身份三元组正是为
    // 这种「计数凑巧成立」的错配存在的。
    let error = verify_conservation_across_files(&first, audit_path(&second))
        .expect_err("错配的一对必须被拒");
    assert!(
        error.to_string().contains("同一次构建"),
        "unexpected: {error}"
    );
    cleanup(&first);
    cleanup(&second);
}

#[test]
fn cross_file_conservation_catches_dispositions_deleted_from_the_audit_database() {
    let path = temp_db("cross-file");
    build(&path);
    verify_conservation_across_files(&path, audit_path(&path)).expect("刚构建的一对应当守恒");

    let audit = Connection::open(audit_path(&path)).expect("open audit db");
    audit
        .execute("DELETE FROM disposition WHERE disposition='excluded'", [])
        .expect("删掉一条处置");
    drop(audit);

    let error = verify_conservation_across_files(&path, audit_path(&path))
        .expect_err("审计库少一条处置必须被跨文件校验抓到");
    assert!(error.to_string().contains("守恒"), "unexpected: {error}");
    cleanup(&path);
}

#[test]
fn build_populates_one_complete_meta_row_and_conserves_dispositions() {
    let path = temp_db("meta");
    build(&path);
    let conn = Connection::open(&path).expect("open built db");
    let meta_rows: i64 = conn
        .query_row("SELECT count(*) FROM corpus_meta", [], |row| row.get(0))
        .expect("count meta");
    assert_eq!(meta_rows, 1);
    let meta: (
        u32,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT schema_version, corpus_version, built_at, source_manifest_sha256, \
                    poem_count, finding_count, input_row_count, index_detail_mode, \
                    derived_indexes, shipped_scope, builder_sqlite_version, integrity_check \
             FROM corpus_meta",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                ))
            },
        )
        .expect("read meta");
    assert_eq!(meta.0, SCHEMA_VERSION);
    assert_eq!(meta.1, "1.2.3");
    assert_eq!(meta.2, "2026-08-10T00:00:00Z");
    assert_eq!(meta.3.len(), 64);
    assert_eq!((meta.4, meta.5, meta.6), (2, 1, 3));
    assert_eq!(meta.7, "full");
    assert_eq!(meta.8, "first_launch");
    assert_eq!(meta.9, "10k");
    assert!(!meta.10.is_empty());
    assert_eq!(meta.11, "ok");

    // 三条处置等式与拆库前逐字相同，只是左端搬进了审计库。
    let audit = Connection::open(audit_path(&path)).expect("open audit db");
    let disposition_count: i64 = audit
        .query_row("SELECT count(*) FROM disposition", [], |row| row.get(0))
        .expect("count dispositions");
    let classified_count: i64 = audit
        .query_row(
            "SELECT sum(CASE WHEN disposition IN ('shipped','quarantined','excluded') \
                             THEN 1 ELSE 0 END) FROM disposition",
            [],
            |row| row.get(0),
        )
        .expect("count classified dispositions");
    let shipped_count: i64 = audit
        .query_row(
            "SELECT count(*) FROM disposition WHERE disposition='shipped'",
            [],
            |row| row.get(0),
        )
        .expect("count shipped dispositions");
    let defect_count: i64 = audit
        .query_row("SELECT count(*) FROM defect", [], |row| row.get(0))
        .expect("count defects");
    assert_eq!(disposition_count, meta.6);
    assert_eq!(classified_count, disposition_count);
    assert_eq!(shipped_count, meta.4);
    assert_eq!(defect_count, meta.5);
    verify_conservation_across_files(&path, audit_path(&path)).expect("跨文件守恒");
    cleanup(&path);
}

#[test]
fn identical_inputs_produce_identical_database_bytes() {
    let first = temp_db("repro-a");
    let second = temp_db("repro-b");
    build(&first);
    build(&second);
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    // 审计库同样要逐字节可复现，否则「构建可复现」只覆盖了两个产物中的一个。
    assert_eq!(
        fs::read(audit_path(&first)).unwrap(),
        fs::read(audit_path(&second)).unwrap()
    );
    cleanup(&first);
    cleanup(&second);
}

#[test]
fn missing_one_hundred_dispositions_is_a_hard_conservation_failure() {
    let mut input = fixture();
    for ordinal in 0..100 {
        input.quality.input_rows += 1;
        input.quality.counts.excluded += 1;
        input.quality.dispositions.push(DispositionRow {
            source_locator: format!("fixture:dropped:{ordinal}"),
            stable_id: None,
            disposition: Disposition::Excluded,
        });
    }
    input.quality.dispositions.truncate(3);
    let path = temp_db("conservation");
    let error = build_database(&path, &input).expect_err("silent deletion must fail");
    assert!(
        error.to_string().contains("守恒"),
        "unexpected error: {error}"
    );
    assert!(!path.exists(), "failed build must not leave a database");
    assert!(
        !audit_path(&path).exists(),
        "失败的构建不得留下审计库；留下的话下一次构建会拿它去配一个新语料库"
    );
}

#[test]
fn incompatible_schema_is_a_typed_actionable_error() {
    let path = temp_db("compat");
    build(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE corpus_meta SET schema_version=3", [])
        .unwrap();
    drop(conn);

    let error = open_corpus(&path).expect_err("schema 3 must be rejected");
    assert!(matches!(
        error,
        OpenCorpusError::IncompatibleSchema {
            corpus_schema_version: 3,
            app_version: env!("CARGO_PKG_VERSION"),
            ..
        }
    ));
    let message = error.to_string();
    assert!(message.contains("3"));
    assert!(message.contains(env!("CARGO_PKG_VERSION")));
    assert!(message.contains("2..=2"));
    assert!(message.contains("yunjian corpus fetch"));
    cleanup(&path);
}

#[test]
fn opened_corpus_is_query_only_and_blocks_insert() {
    let path = temp_db("readonly");
    build(&path);
    let conn = open_corpus(&path).expect("open compatible corpus");
    let query_only: i64 = conn
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .expect("read query_only");
    assert_eq!(query_only, 1);
    let error = conn
        .execute("INSERT INTO tag(name) VALUES (?1)", params!["不应写入"])
        .expect_err("query_only must block INSERT");
    assert!(error.to_string().contains("readonly"));
    cleanup(&path);
}
