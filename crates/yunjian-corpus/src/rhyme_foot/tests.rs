use super::*;
use crate::model::Genre;
use crate::quality::ReasonCode;
use crate::quality::{Disposition, DispositionCounts, DispositionRow, QualityReport};
use crate::rhyme::{RhymeImport, import};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yunjian_core::rhyme::{RhymeBook, RhymeTone};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rhyme_book")
}

fn import_fixture() -> RhymeImport {
    import(fixture_root()).expect("韵书夹具应可导入")
}

fn poem_rows(poem_id: &str, genre: Genre, characters: &[&str]) -> Vec<PoemLastCharInput> {
    characters
        .iter()
        .enumerate()
        .map(|(line_index, character)| PoemLastCharInput {
            poem_id: poem_id.to_owned(),
            work_group: format!("work-{poem_id}"),
            genre,
            line_index,
            character: (*character).to_owned(),
        })
        .collect()
}

#[test]
fn regulated_verse_polyphone_is_resolved_by_the_other_rhyme_feet() {
    let import = import_fixture();
    let input = poem_rows(
        "seven-regulated",
        Genre::Shi,
        &["江", "东", "侵", "同", "董", "铜", "送", "空"],
    );

    let output = derive(&input, &import).expect("七律韵脚应可推导");

    assert_eq!(output.rows.len(), 1);
    let row = &output.rows[0];
    assert_eq!(row.poem_id, "seven-regulated");
    assert_eq!(row.rhyme_book, RhymeBook::Pingshui);
    assert_eq!(row.rhyme_group, "一东");
    assert_eq!(row.tone, RhymeTone::Level);
    assert_eq!(row.confidence, RhymeConfidence::ResolvedByVote);
    assert!(output.findings.is_empty());
}

#[test]
fn ambiguous_two_line_poem_without_corroboration_stays_unresolved() {
    let import = import_fixture();
    let input = poem_rows("two-lines", Genre::Shi, &["〇", "临"]);

    let output = derive(&input, &import).expect("无旁证不应成为硬错误");

    assert_eq!(output.rows.len(), 2, "应保留临的两个真实候选，不能猜一个");
    assert!(
        output
            .rows
            .iter()
            .all(|row| row.confidence == RhymeConfidence::Unresolved)
    );
    assert_eq!(output.findings.len(), 1);
    assert_eq!(output.findings[0].reason_code, ReasonCode::RhymeUnresolved);
}

#[test]
fn ci_uses_cilin_instead_of_pingshui() {
    let import = import_fixture();
    let input = poem_rows("ci-fixture", Genre::Ci, &["东", "同"]);

    let output = derive(&input, &import).expect("词韵脚应可推导");

    assert_eq!(output.rows.len(), 1);
    let row = &output.rows[0];
    assert_eq!(row.rhyme_book, RhymeBook::Cilin);
    assert_eq!(row.rhyme_group, "第一部");
    assert_eq!(row.tone, RhymeTone::Level);
    assert_eq!(row.confidence, RhymeConfidence::Unambiguous);
    assert!(
        output
            .rows
            .iter()
            .all(|row| row.rhyme_book != RhymeBook::Pingshui)
    );
}

#[test]
fn contradictory_rhyme_feet_are_reported_instead_of_breaking_a_tie() {
    let import = import_fixture();
    let input = poem_rows("contradictory", Genre::Shi, &["东", "同", "江", "杠"]);

    let output = derive(&input, &import).expect("矛盾证据应进入质量报告");

    assert_eq!(output.rows.len(), 2);
    assert!(
        output
            .rows
            .iter()
            .all(|row| row.confidence == RhymeConfidence::Unresolved)
    );
    assert_eq!(output.findings.len(), 1);
    assert_eq!(output.findings[0].reason_code, ReasonCode::RhymeUnresolved);
    assert!(output.findings[0].detail.contains("一东"));
    assert!(output.findings[0].detail.contains("三江"));
}

#[test]
fn confidence_distribution_and_unresolved_ratio_are_measurable() {
    let import = import_fixture();
    let mut input = poem_rows(
        "resolved",
        Genre::Shi,
        &["江", "东", "侵", "同", "董", "铜", "送", "空"],
    );
    input.extend(poem_rows("unambiguous", Genre::Ci, &["东", "同"]));
    input.extend(poem_rows("unresolved", Genre::Shi, &["〇", "临"]));

    let output = derive(&input, &import).expect("统计夹具应可推导");
    let stats = output.stats();

    assert_eq!(
        stats.rows_by_confidence,
        BTreeMap::from([
            (RhymeConfidence::ResolvedByVote, 1),
            (RhymeConfidence::Unambiguous, 1),
            (RhymeConfidence::Unresolved, 2),
        ])
    );
    assert_eq!(
        stats.poems_by_confidence,
        BTreeMap::from([
            (RhymeConfidence::ResolvedByVote, 1),
            (RhymeConfidence::Unambiguous, 1),
            (RhymeConfidence::Unresolved, 1),
        ])
    );
    assert_eq!(stats.analyzed_poems, 3);
    assert_eq!(stats.unresolved_poems, 1);
    assert!((stats.unresolved_ratio() - 1.0 / 3.0).abs() < f64::EPSILON);
}

#[test]
fn unresolved_findings_are_counted_in_the_quality_report() {
    let import = import_fixture();
    let input = poem_rows("unresolved", Genre::Shi, &["〇", "临"]);
    let output = derive(&input, &import).expect("无旁证应产出 finding");
    let mut quality = QualityReport {
        schema_version: 1,
        input_rows: 1,
        poem_count: 1,
        counts: DispositionCounts {
            shipped: 1,
            quarantined: 0,
            excluded: 0,
        },
        summary: ReasonCode::ALL
            .iter()
            .map(|reason| (reason.as_str().to_owned(), 0))
            .collect(),
        findings: Vec::new(),
        dispositions: vec![DispositionRow {
            source_locator: "fixture:unresolved".to_owned(),
            stable_id: Some("unresolved".to_owned()),
            disposition: Disposition::Shipped,
        }],
    };

    quality
        .extend_findings(output.findings)
        .expect("韵脚 finding 应并入质量报告");

    assert_eq!(quality.finding_count(ReasonCode::RhymeUnresolved), 1);
    assert_eq!(quality.findings.len(), 1);
    quality
        .check_conservation()
        .expect("finding 不改变处置守恒");
}
