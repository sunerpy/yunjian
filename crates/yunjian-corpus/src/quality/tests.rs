//! `quality` 的测试。
//!
//! 两条断言是这批测试的骨架，其余都在为它们提供可证伪的场景：
//!
//! 1. **守恒只看处置台账。** 有一个刻意造的「一条记录带三个 finding」的 fixture，
//!    证明不变量不会在多 finding 记录上崩掉。
//! 2. **`work_group` 不含作者**，所以《赤壁》的杜牧/李商隐双重归属真的会触发
//!    `conflicting_attribution`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use super::*;
use crate::ingest::werneror::{Bucket, CLASSICAL_BUCKETS, CoveredWorks};
use crate::ingest::{Defect, ScriptDetector, chinese_poetry, werneror};
use crate::model::{
    Dynasty, Provenance, ProvenanceKind, RebuildOutput, RecordInput, RegistryState, Script,
    SourceLocator, rebuild_corpus,
};

const CORPUS_VERSION: &str = "0.1.0-test";

/// fixture 目录里在古典白名单上的分桶。`宋末金初.csv` 之外的 `当代.csv` 与
/// `未来.csv` 刻意不列——前者是已知近现代桶，后者根本不在白名单上，两者都必须
/// 由策略排除而不是由这份名单排除。
const FIXTURE_BUCKETS: [&str; 7] = [
    "先秦.csv",
    "秦.csv",
    "魏晋末南北朝初.csv",
    "隋末唐初.csv",
    "唐.csv",
    "宋末金初.csv",
    "辽.csv",
];

#[derive(Debug, Deserialize)]
struct UpstreamRecord {
    author: String,
    paragraphs: Vec<String>,
    title: String,
    id: String,
}

fn quality_fixture(name: &str) -> Vec<UpstreamRecord> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/quality")
        .join(name);
    let raw = std::fs::read_to_string(&path).expect("读取 quality fixture");
    serde_json::from_str(&raw).expect("解析 quality fixture")
}

fn provenance(license_class: LicenseClass) -> Provenance {
    Provenance {
        source_name: chinese_poetry::SOURCE_NAME.to_owned(),
        source_rev: chinese_poetry::SOURCE_REV.to_owned(),
        license: "MIT".to_owned(),
        license_class,
        kind: ProvenanceKind::Original,
    }
}

fn fixture_provenance() -> Provenance {
    Provenance {
        source_name: SUPPLEMENT_SOURCE.to_owned(),
        source_rev: SUPPLEMENT_REV.to_owned(),
        license: "MIT".to_owned(),
        license_class: LicenseClass::PublicDomain,
        kind: ProvenanceKind::Original,
    }
}

fn upstream_to_input(
    detector: &ScriptDetector,
    record: &UpstreamRecord,
    dynasty_raw: &str,
    provenance: Provenance,
) -> RecordInput {
    let body_original = record.paragraphs.join("\n");
    RecordInput {
        source_locator: SourceLocator::native(&provenance.source_name, &record.id),
        genre: Genre::Shi,
        title: record.title.clone(),
        title_raw: record.title.clone(),
        author: record.author.clone(),
        dynasty: Dynasty::Tang,
        dynasty_raw: dynasty_raw.to_owned(),
        body_lines: record.paragraphs.clone(),
        script: detector.detect(&body_original),
        body_original,
        provenance,
    }
}

fn chibi_inputs() -> Vec<RecordInput> {
    let detector = ScriptDetector::new().expect("初始化探测器");
    quality_fixture("chibi.json")
        .iter()
        .map(|record| {
            upstream_to_input(
                &detector,
                record,
                "唐",
                provenance(LicenseClass::PublicDomain),
            )
        })
        .collect()
}

fn multi_finding_inputs() -> Vec<RecordInput> {
    let detector = ScriptDetector::new().expect("初始化探测器");
    quality_fixture("multi_finding.json")
        .iter()
        .map(|record| upstream_to_input(&detector, record, "唐", fixture_provenance()))
        .collect()
}

fn build(inputs: Vec<RecordInput>) -> RebuildOutput {
    rebuild_corpus(&RegistryState::default(), &[], inputs, CORPUS_VERSION, &[]).expect("重建应成功")
}

fn analyze_records(output: &RebuildOutput) -> QualityReport {
    let normalizer = Normalizer::new().expect("初始化归一器");
    let input = QualityInput {
        shippable: &output.shippable_records,
        restricted: &output.restricted_records,
        blocked: Vec::new(),
        normalization: &[],
    };
    analyze(&input, &normalizer).expect("分析应成功")
}

fn synthetic(ordinal: usize, author: &str, body: &str) -> RecordInput {
    RecordInput {
        source_locator: SourceLocator::positional("fixture", "synthetic.csv", ordinal),
        genre: Genre::Shi,
        title: format!("合成 其{ordinal}"),
        title_raw: format!("合成 其{ordinal}"),
        author: author.to_owned(),
        dynasty: Dynasty::Tang,
        dynasty_raw: "唐".to_owned(),
        body_lines: vec![body.to_owned()],
        body_original: body.to_owned(),
        script: Script::Simplified,
        provenance: provenance(LicenseClass::PublicDomain),
    }
}

fn blocked(ordinal: usize, disposition: Disposition, reason: ReasonCode) -> BlockedUnit {
    BlockedUnit {
        source: "fixture".to_owned(),
        source_locator: format!("fixture:blocked.csv:{ordinal}"),
        disposition,
        reason,
        detail: "合成的被挡下单元".to_owned(),
        work_group: None,
    }
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "yunjian-quality-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("创建临时目录");
    path
}

fn fixture_buckets() -> Vec<Bucket> {
    FIXTURE_BUCKETS
        .iter()
        .map(|file| {
            *CLASSICAL_BUCKETS
                .iter()
                .find(|bucket| bucket.file == *file)
                .unwrap_or_else(|| panic!("白名单里没有 fixture 分桶 {file}"))
        })
        .collect()
}

fn chinese_poetry_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chinese_poetry")
}

fn werneror_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/werneror")
}

fn supplement_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/quality")
}

/// fixture 范围的一次完整跑。**与 `xtask corpus-quality` 走的是同一个
/// [`run_pipeline`]**，否则提交的基线与测试校验的会是两条不同的流水线。
fn end_to_end() -> PipelineOutcome {
    let supplement = load_supplement(&supplement_dir()).expect("读补充记录");
    run_pipeline(
        &chinese_poetry_fixture_root(),
        &werneror_fixture_root(),
        &fixture_buckets(),
        supplement,
        CORPUS_VERSION,
    )
    .expect("fixture 范围流水线应成功")
}

#[test]
fn chibi_dual_attribution_fires_conflicting_attribution() {
    let output = build(chibi_inputs());
    assert_eq!(output.shippable_records.len(), 2, "两条记录都应入库");

    let report = analyze_records(&output);
    let conflicts = report.findings_with(ReasonCode::ConflictingAttribution);
    assert_eq!(
        conflicts.len(),
        2,
        "同组两条记录各记一条归属冲突 finding：{conflicts:#?}"
    );
    for finding in &conflicts {
        assert!(
            finding.detail.contains("杜牧") && finding.detail.contains("李商隱"),
            "冲突详情必须点名两个作者：{}",
            finding.detail
        );
    }
    assert_eq!(
        report.finding_count(ReasonCode::DuplicateInGroup),
        2,
        "重出也要各记一条"
    );
}

#[test]
fn work_group_excludes_the_author_so_two_attributions_land_in_one_group() {
    let output = build(chibi_inputs());
    let [first, second] = &output.shippable_records[..] else {
        panic!("应有两条记录");
    };
    assert_eq!(
        first.work_group, second.work_group,
        "work_group 不含作者，同正文必须同组——这是归属冲突可被发现的前提"
    );
    assert_ne!(
        first.edition_group, second.edition_group,
        "edition_group 含作者，必须把两个归属分开"
    );
    assert_ne!(first.author, second.author);

    let normalizer = Normalizer::new().expect("初始化归一器");
    assert_eq!(
        detection_group(&normalizer, first),
        detection_group(&normalizer, second),
        "判重口径下也必须同组"
    );
}

#[test]
fn duplicates_are_grouped_and_never_deleted() {
    let output = build(chibi_inputs());
    let report = analyze_records(&output);
    assert_eq!(report.poem_count, 2, "重出记录一条都不许删——互见是真实现象");
    assert_eq!(report.counts.shipped, 2);
    assert_eq!(report.counts.excluded, 0);
    assert!(
        report
            .dispositions
            .iter()
            .all(|row| row.disposition == Disposition::Shipped)
    );
}

#[test]
fn one_record_can_carry_three_findings_without_breaking_conservation() {
    let output = build(multi_finding_inputs());
    let report = analyze_records(&output);

    assert_eq!(report.input_rows, 2);
    assert_eq!(report.dispositions.len(), 2, "两条输入恰好两行台账");
    report.check_conservation().expect("守恒必须成立");

    for record in &output.shippable_records {
        let findings = report.findings_for(&record.stable_id);
        let codes: BTreeSet<ReasonCode> =
            findings.iter().map(|finding| finding.reason_code).collect();
        assert_eq!(
            findings.len(),
            3,
            "这条记录刻意同时命中重出、归属冲突、长度可疑：{findings:#?}"
        );
        assert_eq!(
            codes,
            BTreeSet::from([
                ReasonCode::DuplicateInGroup,
                ReasonCode::ConflictingAttribution,
                ReasonCode::SuspectLength,
            ])
        );
        assert_eq!(
            report
                .dispositions
                .iter()
                .filter(|row| row.stable_id.as_deref() == Some(record.stable_id.as_str()))
                .count(),
            1,
            "带三个 finding 的记录在台账上仍然只占一行"
        );
    }

    assert_eq!(report.findings.len(), 6, "2 条记录 × 3 个 finding");
    assert_ne!(
        report.findings.len(),
        report.input_rows,
        "finding 数与输入记录数是两个数；把它们相加或相等地看待就是那个算术错误"
    );
}

#[test]
fn conservation_is_asserted_over_dispositions_never_over_findings() {
    let inputs = (0..500)
        .map(|ordinal| synthetic(ordinal, "李白", &format!("孤帆远影碧空尽其{ordinal}")))
        .collect::<Vec<_>>();
    let output = build(inputs);

    let mut blocked_units = Vec::new();
    for ordinal in 0..60 {
        blocked_units.push(blocked(
            ordinal,
            Disposition::Quarantined,
            ReasonCode::LossyChar,
        ));
    }
    for ordinal in 60..100 {
        blocked_units.push(blocked(
            ordinal,
            Disposition::Excluded,
            ReasonCode::ExcludedByPolicy,
        ));
    }

    let normalizer = Normalizer::new().expect("初始化归一器");
    let input = QualityInput {
        shippable: &output.shippable_records,
        restricted: &output.restricted_records,
        blocked: blocked_units,
        normalization: &[],
    };
    let report = analyze(&input, &normalizer).expect("分析应成功");

    assert_eq!(report.input_rows, 600);
    assert_eq!(
        report.dispositions.len(),
        600,
        "count(dispositions) == input_rows"
    );
    assert_eq!(report.counts.shipped, 500);
    assert_eq!(report.counts.quarantined, 60);
    assert_eq!(report.counts.excluded, 40);
    assert_eq!(
        report.counts.shipped + report.counts.quarantined + report.counts.excluded,
        report.input_rows
    );
    assert_eq!(report.poem_count, report.counts.shipped);
    report.check_conservation().expect("守恒必须成立");
}

#[test]
fn a_lost_record_without_a_disposition_row_fails_conservation() {
    let inputs = (0..120)
        .map(|ordinal| synthetic(ordinal, "杜甫", &format!("会当凌绝顶其{ordinal}")))
        .collect::<Vec<_>>();
    let output = build(inputs);
    let mut report = analyze_records(&output);
    report.check_conservation().expect("原始报告应守恒");

    report.dispositions.truncate(20);
    report.counts.shipped = 20;
    report.poem_count = 20;
    let error = report
        .check_conservation()
        .expect_err("台账少了 100 行必须失败");
    let message = error.to_string();
    assert!(message.contains("120"), "错误须给出输入数：{message}");
    assert!(message.contains("静默消失"), "错误须说明后果：{message}");
}

#[test]
fn poem_count_must_equal_shipped() {
    let output = build(vec![synthetic(0, "王维", "空山不见人，但闻人语响。")]);
    let mut report = analyze_records(&output);
    report.poem_count += 1;
    let error = report
        .check_conservation()
        .expect_err("poem_count 漂移必须失败");
    assert!(error.to_string().contains("poem_count"));
}

#[test]
fn every_stable_id_in_defects_json_appears_in_dispositions_json() {
    let root = temp_dir("join");
    let end = end_to_end();
    write_artifacts(&root, "fixtures", &end.report).expect("写出工件");

    let defects = DefectsFile::load(root.join(DEFECTS_JSON)).expect("读 defects.json");
    let dispositions =
        DispositionsFile::load(root.join(DISPOSITIONS_JSON)).expect("读 dispositions.json");

    let ledger: BTreeSet<String> = dispositions
        .rows
        .iter()
        .filter_map(|row| row.stable_id.clone())
        .collect();
    let ids: Vec<&String> = defects
        .findings
        .iter()
        .filter_map(|finding| finding.stable_id.as_ref())
        .collect();
    assert!(
        !ids.is_empty(),
        "join 必须非空，否则这条断言永远成立却什么都没验"
    );
    for id in ids {
        assert!(
            ledger.contains(id),
            "defects.json 的 stable_id {id} 不在 dispositions.json"
        );
    }
    assert!(
        !defects.summary.is_empty(),
        "defects.json 必须带 summary 供 jq 断言"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_orphan_finding_fails_the_cross_report_join() {
    let output = build(vec![synthetic(0, "孟浩然", "春眠不觉晓，处处闻啼鸟。")]);
    let mut report = analyze_records(&output);
    report
        .check_cross_report_integrity()
        .expect("原始报告应通过");

    report.findings.push(Finding {
        stable_id: Some("deadbeefdeadbeef".to_owned()),
        work_group: None,
        reason_code: ReasonCode::SuspectLength,
        detail: "指向不存在的记录".to_owned(),
        source: "fixture".to_owned(),
    });
    let error = report
        .check_cross_report_integrity()
        .expect_err("孤儿 finding 必须失败");
    assert!(error.to_string().contains("deadbeefdeadbeef"));
}

#[test]
fn a_self_contradictory_report_is_never_written_to_disk() {
    let root = temp_dir("guard");
    let output = build(vec![synthetic(0, "柳宗元", "千山鸟飞绝，万径人踪灭。")]);
    let mut report = analyze_records(&output);
    report.dispositions.clear();
    write_artifacts(&root, "fixtures", &report).expect_err("守恒不成立时不许落盘");
    assert!(
        !root.join(DEFECTS_JSON).exists(),
        "校验先于写盘，工件不该出现"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn baseline_drift_fails_and_names_the_reason_code() {
    let end = end_to_end();
    let baseline = Baseline::from_report("fixtures", "实测生成", &end.report);
    baseline.check(&end.report).expect("刚生成的基线必须自洽");

    let mut drifted = end.report.clone();
    let victim = drifted
        .dispositions
        .iter()
        .find_map(|row| row.stable_id.clone())
        .expect("需要一个已铸造身份的记录");
    for index in 0..20 {
        drifted.findings.push(Finding {
            stable_id: Some(victim.clone()),
            work_group: None,
            reason_code: ReasonCode::LossyChar,
            detail: format!("注入的第 {index} 条缺字 finding"),
            source: "fixture".to_owned(),
        });
    }
    *drifted
        .summary
        .entry(ReasonCode::LossyChar.as_str().to_owned())
        .or_default() += 20;

    let error = baseline.check(&drifted).expect_err("超容差必须失败");
    let message = error.to_string();
    assert!(message.contains("lossy_char"), "必须点名原因码：{message}");
    assert!(
        message.contains("基线漂移"),
        "必须说明是基线漂移：{message}"
    );
    assert_eq!(
        drifted.input_rows, end.report.input_rows,
        "只加 finding 不改输入数，证明门禁是按逐码计数触发的"
    );
}

#[test]
fn zero_tolerance_codes_admit_no_drift_at_all() {
    assert_eq!(ReasonCode::RestrictedLicense.default_tolerance_pct(), 0);
    assert_eq!(ReasonCode::ExcludedByPolicy.default_tolerance_pct(), 0);
    for reason in ReasonCode::ALL {
        if matches!(
            reason,
            ReasonCode::RestrictedLicense | ReasonCode::ExcludedByPolicy
        ) {
            continue;
        }
        assert_eq!(reason.default_tolerance_pct(), 10, "{}", reason.as_str());
    }

    let entry = BaselineEntry {
        reason_code: ReasonCode::RestrictedLicense,
        expected: 0,
        tolerance_pct: 0,
    };
    assert_eq!(entry.allowed(), (0, 0), "受限许可出现一条就必须失败");
}

#[test]
fn tolerance_is_floored_so_small_counts_are_exact() {
    let small = BaselineEntry {
        reason_code: ReasonCode::LossyChar,
        expected: 3,
        tolerance_pct: 10,
    };
    assert_eq!(
        small.allowed(),
        (3, 3),
        "3 的 10% 下取整是 0，所以 fixture 规模的基线是精确的"
    );
    let large = BaselineEntry {
        reason_code: ReasonCode::LossyChar,
        expected: 6_113,
        tolerance_pct: 10,
    };
    assert_eq!(large.allowed(), (5_502, 6_724));
}

#[test]
fn a_baseline_missing_a_reason_code_fails_rather_than_passing_it() {
    let end = end_to_end();
    let mut baseline = Baseline::from_report("fixtures", "实测生成", &end.report);
    baseline
        .codes
        .retain(|entry| entry.reason_code != ReasonCode::ConflictingAttribution);
    let error = baseline.check(&end.report).expect_err("缺原因码必须失败");
    assert!(error.to_string().contains("conflicting_attribution"));
}

#[test]
fn input_row_drift_fails_because_per_code_counts_stop_being_comparable() {
    let end = end_to_end();
    let mut baseline = Baseline::from_report("fixtures", "实测生成", &end.report);
    baseline.input_rows += 100;
    let error = baseline.check(&end.report).expect_err("输入数漂移必须失败");
    assert!(error.to_string().contains("输入行数漂移"));
}

#[test]
fn the_committed_baseline_covers_all_nine_reason_codes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BASELINE_JSON);
    let baseline = Baseline::load(&path).expect("读取提交的基线");
    assert_eq!(baseline.schema_version, SCHEMA_VERSION);
    assert_eq!(baseline.scope, "fixtures");
    assert_eq!(baseline.codes.len(), ReasonCode::ALL.len());
    for reason in ReasonCode::ALL {
        let entry = baseline
            .codes
            .iter()
            .find(|entry| entry.reason_code == reason)
            .unwrap_or_else(|| panic!("基线缺少 {}", reason.as_str()));
        assert_eq!(
            entry.tolerance_pct,
            reason.default_tolerance_pct(),
            "{} 的容差必须与默认一致，改它要有理由",
            reason.as_str()
        );
    }
}

#[test]
fn the_committed_baseline_matches_a_fresh_fixture_run() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BASELINE_JSON);
    let baseline = Baseline::load(&path).expect("读取提交的基线");
    let end = end_to_end();
    baseline
        .check(&end.report)
        .expect("提交的基线必须与当前 fixture 实测一致");
}

#[test]
fn lossy_char_records_are_quarantined_and_kept_out_of_the_main_table() {
    let end = end_to_end();
    let lossy = end.report.findings_with(ReasonCode::LossyChar);
    assert!(!lossy.is_empty(), "fixture 里有含半角 `?` 的记录");
    for finding in &lossy {
        assert!(
            finding.stable_id.is_none(),
            "缺字记录在铸造身份之前就被挡下"
        );
        assert!(finding.detail.contains("不可恢复"));
        assert!(
            !finding.detail.contains("已修正"),
            "本阶段不许补字：{}",
            finding.detail
        );
    }
    let quarantined = end
        .report
        .dispositions
        .iter()
        .filter(|row| row.disposition == Disposition::Quarantined)
        .count();
    assert!(quarantined >= lossy.len());
}

#[test]
fn excluded_by_policy_covers_the_modern_and_unlisted_buckets() {
    let end = end_to_end();
    let excluded = end.report.findings_with(ReasonCode::ExcludedByPolicy);
    let detail = excluded
        .iter()
        .map(|finding| finding.detail.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        detail.contains("当代.csv"),
        "已知近现代分桶必须被排除：{detail}"
    );
    assert!(
        detail.contains("未来.csv"),
        "白名单外的新分桶默认排除：{detail}"
    );
}

#[test]
fn conversion_unstable_joins_to_the_record_by_stable_id() {
    let output = build(vec![synthetic(0, "韩愈", "昵昵儿女语，恩怨相尔汝。")]);
    let normalizer = Normalizer::new().expect("初始化归一器");
    let normalized = normalizer
        .normalize(&output.shippable_records)
        .expect("归一");
    assert_eq!(
        normalized.findings.len(),
        1,
        "「昵」是 t2s 单字不动、往返推导被过滤掉的那批字形之一"
    );

    let input = QualityInput {
        shippable: &output.shippable_records,
        restricted: &output.restricted_records,
        blocked: Vec::new(),
        normalization: &normalized.findings,
    };
    let report = analyze(&input, &normalizer).expect("分析应成功");
    let findings = report.findings_with(ReasonCode::ConversionUnstable);
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].stable_id.as_deref(),
        Some(output.shippable_records[0].stable_id.as_str())
    );
    assert!(findings[0].work_group.is_some(), "join 上了就该带分组键");
    assert_eq!(
        report.counts.shipped, 1,
        "conversion_unstable 是待复核告警，不改处置"
    );
}

#[test]
fn a_normalization_finding_for_an_unknown_record_fails_loudly() {
    let output = build(vec![synthetic(
        0,
        "李商隐",
        "沧海月明珠有泪，蓝田日暖玉生烟。",
    )]);
    let normalizer = Normalizer::new().expect("初始化归一器");
    let stray = [NormalizationFinding {
        stable_id: "0000000000000000".to_owned(),
        reason: crate::normalize::NormalizationReason::ConversionUnstable,
        detail: "指向不存在的记录".to_owned(),
    }];
    let input = QualityInput {
        shippable: &output.shippable_records,
        restricted: &output.restricted_records,
        blocked: Vec::new(),
        normalization: &stray,
    };
    let error = analyze(&input, &normalizer).expect_err("join 不上必须失败");
    assert!(error.to_string().contains("0000000000000000"));
}

#[test]
fn suspect_length_fires_on_a_short_body_and_on_an_unsplit_verse_line() {
    let short = build(vec![synthetic(0, "无名氏", "山中月")]);
    let report = analyze_records(&short);
    let findings = report.findings_with(ReasonCode::SuspectLength);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].detail.contains("3 个汉字"));

    let long_line = "床前明月光疑是地上霜举头望明月低头思故乡".repeat(4);
    let unsplit = build(vec![synthetic(1, "李白", &long_line)]);
    let report = analyze_records(&unsplit);
    let findings = report.findings_with(ReasonCode::SuspectLength);
    assert_eq!(findings.len(), 1, "整段挤成一行必须被报出来");
    assert!(findings[0].detail.contains("句读"));
}

#[test]
fn prose_genres_are_exempt_from_the_verse_line_limit() {
    let mut input = synthetic(0, "韩愈", &"古之学者必有师师者所以传道受业解惑也".repeat(6));
    input.genre = Genre::Wen;
    let output = build(vec![input]);
    let report = analyze_records(&output);
    assert_eq!(
        report.finding_count(ReasonCode::SuspectLength),
        0,
        "散文本来就是长句，行长上限只对诗词曲成立"
    );
}

#[test]
fn unknown_dynasty_can_only_come_from_the_ingest_stage_and_is_an_exclusion() {
    let mut input = synthetic(0, "无名氏", "枯藤老树昏鸦，小桥流水人家。");
    input.dynasty_raw = String::new();
    let error = rebuild_corpus(
        &RegistryState::default(),
        &[],
        vec![input],
        CORPUS_VERSION,
        &[],
    )
    .expect_err("身份铸造之前就该拦住不可归一的朝代");
    assert!(
        error.to_string().contains("无法归一朝代"),
        "拦在 model::prepare 上，所以 quality 层不需要再兜一次：{error}"
    );

    assert_eq!(
        map_defect(DefectReason::UnknownDynasty),
        Some((ReasonCode::UnknownDynasty, Disposition::Excluded))
    );

    let normalizer = Normalizer::new().expect("初始化归一器");
    let quality_input = QualityInput {
        shippable: &[],
        restricted: &[],
        blocked: vec![blocked(
            0,
            Disposition::Excluded,
            ReasonCode::UnknownDynasty,
        )],
        normalization: &[],
    };
    let report = analyze(&quality_input, &normalizer).expect("分析应成功");
    assert_eq!(report.finding_count(ReasonCode::UnknownDynasty), 1);
    assert_eq!(report.counts.excluded, 1);
    assert_eq!(report.poem_count, 0);
}

#[test]
fn restricted_records_are_quarantined_and_never_shipped() {
    let mut input = synthetic(0, "某今人", "受限内容，不得分发。");
    input.provenance = provenance(LicenseClass::Restricted);
    let output = build(vec![input]);
    assert!(output.shippable_records.is_empty());
    assert_eq!(output.restricted_records.len(), 1);

    let report = analyze_records(&output);
    assert_eq!(report.poem_count, 0);
    assert_eq!(report.counts.quarantined, 1);
    assert_eq!(report.finding_count(ReasonCode::RestrictedLicense), 1);
    report.check_conservation().expect("守恒必须成立");
}

#[test]
fn a_restricted_record_in_the_shippable_slice_is_rejected() {
    let mut input = synthetic(0, "某今人", "受限内容，不得分发。");
    input.provenance = provenance(LicenseClass::Restricted);
    let smuggled = build(vec![input]).restricted_records;
    let normalizer = Normalizer::new().expect("初始化归一器");
    let quality_input = QualityInput {
        shippable: &smuggled,
        restricted: &[],
        blocked: Vec::new(),
        normalization: &[],
    };
    let error = analyze(&quality_input, &normalizer).expect_err("受限记录不许出现在可分发集合");
    assert!(error.to_string().contains("许可受限"));
}

#[test]
fn empty_body_is_reported_and_excluded_by_the_ingest_stage() {
    let end = end_to_end();
    let empty = end.report.findings_with(ReasonCode::EmptyBody);
    assert!(!empty.is_empty(), "fixture 里有正文为空的上游条目");
    for finding in &empty {
        assert!(finding.stable_id.is_none(), "空正文在铸造身份之前就被挡下");
    }
    assert!(
        end.shippable
            .iter()
            .all(|record| !record.body_lines.join("\n").trim().is_empty()),
        "可分发集合里不许有空正文"
    );
}

#[test]
fn a_blocked_unit_claiming_to_be_shipped_is_rejected() {
    let normalizer = Normalizer::new().expect("初始化归一器");
    let input = QualityInput {
        shippable: &[],
        restricted: &[],
        blocked: vec![blocked(0, Disposition::Shipped, ReasonCode::LossyChar)],
        normalization: &[],
    };
    let error = analyze(&input, &normalizer).expect_err("挡下的记录不能是 shipped");
    assert!(error.to_string().contains("shipped"));
}

#[test]
fn a_duplicated_locator_in_the_ledger_is_rejected() {
    let normalizer = Normalizer::new().expect("初始化归一器");
    let input = QualityInput {
        shippable: &[],
        restricted: &[],
        blocked: vec![
            blocked(7, Disposition::Excluded, ReasonCode::ExcludedByPolicy),
            blocked(7, Disposition::Quarantined, ReasonCode::LossyChar),
        ],
        normalization: &[],
    };
    let error = analyze(&input, &normalizer).expect_err("locator 重复必须失败");
    let message = error.to_string();
    assert!(message.contains("fixture:blocked.csv:7"));
    assert!(message.contains("守恒式漏掉"), "错误要说明为什么这条也要查");
}

#[test]
fn the_end_to_end_run_over_committed_fixtures_is_conserved_and_joinable() {
    let end = end_to_end();
    end.report.check_conservation().expect("守恒");
    end.report
        .check_cross_report_integrity()
        .expect("跨报告完整性");
    assert!(
        end.report.input_rows > end.report.poem_count,
        "有被挡下的输入"
    );
    assert_eq!(end.report.poem_count, end.shippable.len());
    assert_eq!(
        end.report.summary.len(),
        ReasonCode::ALL.len(),
        "summary 恒列全部原因码，命中 0 也要有键，否则基线无法发现「这个码不再命中了」"
    );
}

#[test]
fn artifacts_are_byte_reproducible_across_two_runs() {
    let first = temp_dir("repro-a");
    let second = temp_dir("repro-b");
    let end = end_to_end();
    write_artifacts(&first, "fixtures", &end.report).expect("第一次写出");
    write_artifacts(&second, "fixtures", &end.report).expect("第二次写出");
    for name in [DEFECTS_JSON, DEFECTS_MD, DISPOSITIONS_JSON] {
        let left = std::fs::read(first.join(name)).expect("读第一次");
        let right = std::fs::read(second.join(name)).expect("读第二次");
        assert_eq!(left, right, "{name} 必须逐字节可复现");
    }
    std::fs::remove_dir_all(&first).ok();
    std::fs::remove_dir_all(&second).ok();
}

#[test]
fn the_detection_key_folds_the_yu_variant_that_work_group_misses() {
    let traditional = synthetic(0, "无名氏", "綺殿千尋起，離宮百雉餘。");
    let variant = synthetic(1, "无名氏", "绮殿千寻起，离宫百雉馀。");
    let output = build(vec![traditional, variant]);
    let [first, second] = &output.shippable_records[..] else {
        panic!("应有两条记录");
    };
    assert_ne!(
        first.work_group, second.work_group,
        "身份上的 work_group 按原字形算，异体与繁简都判不出重——这是已知限制"
    );

    let normalizer = Normalizer::new().expect("初始化归一器");
    assert_eq!(
        detection_group(&normalizer, first),
        detection_group(&normalizer, second),
        "判重口径先过 canonicalize，所以「餘/馀」与繁简差异都归到一组"
    );

    let report = analyze_records(&output);
    assert_eq!(
        report.finding_count(ReasonCode::DuplicateInGroup),
        2,
        "两条异体写法必须被报成重出"
    );
}

#[test]
fn reason_code_strings_match_the_serialized_form() {
    for reason in ReasonCode::ALL {
        let json = serde_json::to_string(&reason).expect("序列化原因码");
        assert_eq!(json, format!("\"{}\"", reason.as_str()));
    }
    for disposition in [
        Disposition::Shipped,
        Disposition::Quarantined,
        Disposition::Excluded,
    ] {
        let json = serde_json::to_string(&disposition).expect("序列化处置");
        assert_eq!(json, format!("\"{}\"", disposition.as_str()));
    }
}

#[test]
fn strain_defects_do_not_become_ledger_rows() {
    for reason in [
        DefectReason::StrainsMisaligned,
        DefectReason::StrainsUnavailable,
        DefectReason::StrainsLineMismatch,
    ] {
        assert!(
            map_defect(reason).is_none(),
            "平仄类缺陷挂在已入库的记录上，折成台账行会把同一条记录数两遍"
        );
    }
    let outcome = IngestOutcome {
        defects: vec![Defect {
            relative_path: "全唐诗/poet.tang.0.json".to_owned(),
            ordinal: 3,
            reason: DefectReason::StrainsUnavailable,
            detail: "上游没算平仄".to_owned(),
        }],
        ..IngestOutcome::default()
    };
    assert!(blocked_from_chinese_poetry(&outcome).is_empty());
}

#[test]
fn werneror_lossy_rows_are_counted_once_not_twice() {
    let outcome = werneror::ingest_buckets(
        werneror_fixture_root(),
        &fixture_buckets(),
        &CoveredWorks::empty(),
    )
    .expect("werneror 入库");

    let units = blocked_from_werneror(&outcome);
    let lossy = units
        .iter()
        .filter(|unit| unit.reason == ReasonCode::LossyChar)
        .count();
    assert_eq!(
        lossy,
        outcome.quarantined.len(),
        "缺字行既进 quarantined 又有一条 LossyCharacter 缺陷，只能算一次"
    );

    let mut locators: Vec<&str> = units
        .iter()
        .map(|unit| unit.source_locator.as_str())
        .collect();
    let before = locators.len();
    locators.sort_unstable();
    locators.dedup();
    assert_eq!(before, locators.len(), "被挡下的单元 locator 必须互不重复");
}
