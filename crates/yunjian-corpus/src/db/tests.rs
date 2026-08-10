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
            confidence: "unambiguous".to_owned(),
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

#[test]
fn schema_is_the_checked_in_single_source_of_truth() {
    let path = temp_db("schema");
    build(&path);
    let conn = Connection::open(&path).expect("open built db");
    let mut statement = conn
        .prepare(
            "SELECT sql FROM sqlite_schema \
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
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
    fs::remove_file(path).ok();
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
    ) = conn
        .query_row(
            "SELECT schema_version, corpus_version, built_at, source_manifest_sha256, \
                    poem_count, finding_count, input_row_count, index_detail_mode, \
                    builder_sqlite_version, integrity_check FROM corpus_meta",
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
                ))
            },
        )
        .expect("read meta");
    assert_eq!(meta.0, 1);
    assert_eq!(meta.1, "1.2.3");
    assert_eq!(meta.2, "2026-08-10T00:00:00Z");
    assert_eq!(meta.3.len(), 64);
    assert_eq!((meta.4, meta.5, meta.6), (2, 1, 3));
    assert_eq!(meta.7, "full");
    assert!(!meta.8.is_empty());
    assert_eq!(meta.9, "ok");

    let disposition_count: i64 = conn
        .query_row("SELECT count(*) FROM disposition", [], |row| row.get(0))
        .expect("count dispositions");
    let classified_count: i64 = conn
        .query_row(
            "SELECT sum(CASE WHEN disposition IN ('shipped','quarantined','excluded') \
                             THEN 1 ELSE 0 END) FROM disposition",
            [],
            |row| row.get(0),
        )
        .expect("count classified dispositions");
    let shipped_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM disposition WHERE disposition='shipped'",
            [],
            |row| row.get(0),
        )
        .expect("count shipped dispositions");
    assert_eq!(disposition_count, meta.6);
    assert_eq!(classified_count, disposition_count);
    assert_eq!(shipped_count, meta.4);
    fs::remove_file(path).ok();
}

#[test]
fn identical_inputs_produce_identical_database_bytes() {
    let first = temp_db("repro-a");
    let second = temp_db("repro-b");
    build(&first);
    build(&second);
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    fs::remove_file(first).ok();
    fs::remove_file(second).ok();
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
}

#[test]
fn incompatible_schema_is_a_typed_actionable_error() {
    let path = temp_db("compat");
    build(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE corpus_meta SET schema_version=2", [])
        .unwrap();
    drop(conn);

    let error = open_corpus(&path).expect_err("schema 2 must be rejected");
    assert!(matches!(
        error,
        OpenCorpusError::IncompatibleSchema {
            corpus_schema_version: 2,
            app_version: env!("CARGO_PKG_VERSION"),
            ..
        }
    ));
    let message = error.to_string();
    assert!(message.contains("2"));
    assert!(message.contains(env!("CARGO_PKG_VERSION")));
    assert!(message.contains("1..=1"));
    assert!(message.contains("yunjian corpus fetch"));
    fs::remove_file(path).ok();
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
    fs::remove_file(path).ok();
}
