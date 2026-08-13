use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;
use serde::Deserialize;
use yunjian_core::{CorpusConfig, CorpusHandle, GradingConfig, SCHEMA_VERSION};
use yunjian_recite::{
    AlignOp, FsrsGrade, PhoneticReview, Poem, RelativeRhythm, SubstitutionClass, TypedAttempt,
    VoicePracticeFeedback, align, grade_typed, review_typed,
};

const REQUIRED_EXECUTABLE_CATEGORIES: [&str; 8] = [
    "perfect",
    "missing_line",
    "extra_character",
    "rerecitation",
    "homophone_substitution",
    "distinct_substitution",
    "wrong_poem",
    "half_finished",
];
const ACCENT_CATEGORY: &str = "heavily_accented_reading";
const MAX_BAND_WIDTH: f32 = 0.2;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenSet {
    schema_version: u32,
    #[serde(rename = "case")]
    cases: Vec<GoldenCase>,
    unmeasured: Vec<UnmeasuredCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenCase {
    id: String,
    category: String,
    reference: String,
    attempt: String,
    expected_ops: Vec<String>,
    scores: ScoreBands,
    is_rejected: bool,
    expected_grade: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnmeasuredCase {
    id: String,
    category: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreBands {
    completeness: [f32; 2],
    accuracy_strict: [f32; 2],
    accuracy_lenient: [f32; 2],
    fluency: [f32; 2],
}

#[derive(Debug, Deserialize)]
struct CerReport {
    measured: bool,
    scoring_mode: String,
    cer_threshold: f64,
    overall: OverallCer,
    fixture: CerFixture,
}

#[derive(Debug, Deserialize)]
struct OverallCer {
    cer: f64,
}

#[derive(Debug, Deserialize)]
struct CerFixture {
    human_recordings_used: bool,
}

struct Fixture {
    dir: PathBuf,
    handle: CorpusHandle,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn golden_set() -> GoldenSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cases.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取黄金契约失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析黄金契约失败 {}：{error}", path.display()))
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-recite-golden-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建黄金测试目录");
    let path = dir.join("corpus.db");
    write_fixture(&path);
    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(path),
        data_dir: dir.clone(),
        archive: None,
    })
    .expect("打开黄金测试语料库");
    Fixture { dir, handle }
}

fn write_fixture(path: &Path) {
    let connection = Connection::open(path).expect("创建黄金测试语料库");
    connection
        .execute_batch(
            "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);
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
             );",
        )
        .expect("创建黄金测试 schema");
    connection
        .execute(
            "INSERT INTO corpus_meta VALUES
             (1, ?1, 'golden-v1', '2026-08-13T00:00:00Z', 0, 'full',
              'first_launch', '10k', 'ok')",
            [SCHEMA_VERSION],
        )
        .expect("写入黄金测试元数据");
    connection.close().expect("关闭黄金测试语料库");
}

#[test]
fn all_hand_labelled_attempts_match_the_immutable_contract() {
    let set = golden_set();
    let fixture = fixture();
    let grading = GradingConfig::default();
    let mut executed = 0_usize;
    let mut failures = Vec::new();

    for case in &set.cases {
        let poem = Poem::new(&fixture.handle, &case.reference)
            .unwrap_or_else(|error| panic!("{}: 构造参考诗失败：{error}", case.id));
        let attempt = TypedAttempt::new(&fixture.handle, &case.attempt)
            .unwrap_or_else(|error| panic!("{}: 构造尝试失败：{error}", case.id));
        let alignment = align(&fixture.handle, &case.reference, &case.attempt)
            .unwrap_or_else(|error| panic!("{}: 对齐失败：{error}", case.id));
        let review = review_typed(&poem, &attempt);
        let actual_ops = operation_contract(&alignment.ops, &review);

        if actual_ops != case.expected_ops {
            failures.push(format!(
                "{}: 操作序列漂移\n  实际：{actual_ops:?}\n  期望：{:?}",
                case.id, case.expected_ops
            ));
        }
        collect_band_failure(
            &mut failures,
            case,
            "completeness",
            review.score.completeness,
            case.scores.completeness,
        );
        collect_band_failure(
            &mut failures,
            case,
            "accuracy_strict",
            review.score.accuracy_strict,
            case.scores.accuracy_strict,
        );
        collect_band_failure(
            &mut failures,
            case,
            "accuracy_lenient",
            review.score.accuracy_lenient,
            case.scores.accuracy_lenient,
        );
        collect_band_failure(
            &mut failures,
            case,
            "fluency",
            review.score.fluency,
            case.scores.fluency,
        );
        if review.score.is_rejected != case.is_rejected {
            failures.push(format!(
                "{}: 拒绝判定漂移，实际 {}，期望 {}",
                case.id, review.score.is_rejected, case.is_rejected
            ));
        }
        let actual_grade = grade_typed(&review.score, true, &grading);
        let expected_grade = parse_grade(&case.expected_grade);
        if actual_grade != expected_grade {
            failures.push(format!(
                "{}: 默认评级漂移，实际 {actual_grade:?}，期望 {expected_grade:?}",
                case.id
            ));
        }
        executed += 1;
    }

    assert_eq!(executed, set.cases.len(), "必须实际执行契约中的每一个 case");
    assert!(
        executed >= 30,
        "只执行了 {executed} 个 case，方案要求至少 30 个"
    );
    assert!(
        failures.is_empty(),
        "{} 个黄金断言失败：\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn operation_contract(ops: &[AlignOp], review: &PhoneticReview) -> Vec<String> {
    let substitutions = review
        .substitutions
        .iter()
        .map(|item| ((item.reference_index, item.attempt_index), item.class))
        .collect::<BTreeMap<_, _>>();
    let mut runs = Vec::<(String, String)>::new();
    for op in ops {
        let (kind, text, merge) = match op {
            AlignOp::Normal { character, .. } => ("normal", character.to_string(), true),
            AlignOp::Deletion { reference, .. } => ("deletion", reference.to_string(), true),
            AlignOp::Insertion { attempt, .. } => ("insertion", attempt.to_string(), true),
            AlignOp::ReRecitation { text, .. } => ("rerecitation", text.clone(), false),
            AlignOp::Substitution {
                reference_index,
                attempt_index,
                reference,
                attempt,
            } => {
                let class = substitutions
                    .get(&(*reference_index, *attempt_index))
                    .unwrap_or_else(|| panic!("替换 {reference}→{attempt} 缺少拼音复核"));
                let kind = match class {
                    SubstitutionClass::NearHomophone => "near_homophone",
                    SubstitutionClass::Distinct => "substitution",
                };
                (kind, format!("{reference}→{attempt}"), true)
            }
        };
        if merge
            && let Some((last_kind, last_text)) = runs.last_mut()
            && last_kind == kind
        {
            if kind == "substitution" || kind == "near_homophone" {
                last_text.push(',');
            }
            last_text.push_str(&text);
            continue;
        }
        runs.push((kind.to_owned(), text));
    }
    runs.into_iter()
        .map(|(kind, text)| format!("{kind}:{text}"))
        .collect()
}

fn collect_band_failure(
    failures: &mut Vec<String>,
    case: &GoldenCase,
    name: &str,
    actual: f32,
    band: [f32; 2],
) {
    if !(band[0]..=band[1]).contains(&actual) {
        failures.push(format!(
            "{}: {name}={actual} 不在人工区间 [{}, {}]",
            case.id, band[0], band[1]
        ));
    }
}

fn parse_grade(value: &str) -> FsrsGrade {
    match value {
        "Again" => FsrsGrade::Again,
        "Hard" => FsrsGrade::Hard,
        "Good" => FsrsGrade::Good,
        "Easy" => FsrsGrade::Easy,
        other => panic!("黄金契约含非法等级：{other}"),
    }
}

#[test]
fn golden_manifest_cannot_be_weakened_into_an_empty_or_vague_suite() {
    let set = golden_set();
    assert_eq!(set.schema_version, 1);
    assert!(set.cases.len() >= 30, "黄金集至少需要 30 个可执行 case");

    let mut ids = BTreeSet::new();
    let mut category_counts = BTreeMap::<&str, usize>::new();
    for case in &set.cases {
        assert!(ids.insert(case.id.as_str()), "case id 重复：{}", case.id);
        assert!(
            !case.reference.trim().is_empty(),
            "{}: reference 为空",
            case.id
        );
        assert!(!case.attempt.trim().is_empty(), "{}: attempt 为空", case.id);
        assert!(!case.expected_ops.is_empty(), "{}: op 标注为空", case.id);
        assert!(
            case.expected_ops.iter().all(|op| op.contains(':')),
            "{}: op 标注没有类型与内容：{:?}",
            case.id,
            case.expected_ops
        );
        *category_counts.entry(&case.category).or_default() += 1;
        for (name, band) in [
            ("completeness", case.scores.completeness),
            ("accuracy_strict", case.scores.accuracy_strict),
            ("accuracy_lenient", case.scores.accuracy_lenient),
            ("fluency", case.scores.fluency),
        ] {
            assert!(band[0] <= band[1], "{}: {name} 区间反向", case.id);
            assert!((0.0..=1.0).contains(&band[0]) && (0.0..=1.0).contains(&band[1]));
            assert!(
                band[1] - band[0] <= MAX_BAND_WIDTH,
                "{}: {name} 区间跨度超过 {MAX_BAND_WIDTH}",
                case.id
            );
        }
    }

    for category in REQUIRED_EXECUTABLE_CATEGORIES {
        let count = category_counts.get(category).copied().unwrap_or_default();
        assert_eq!(
            count, 4,
            "类别 {category} 必须恰有 4 个手工 case，实际 {count}"
        );
    }
    assert_eq!(
        category_counts.len(),
        REQUIRED_EXECUTABLE_CATEGORIES.len(),
        "出现未登记的可执行类别：{category_counts:?}"
    );
}

#[test]
fn accented_reading_is_explicitly_unmeasured_not_synthetic_ground_truth() {
    let set = golden_set();
    assert_eq!(set.unmeasured.len(), 4, "重口音缺口必须具名记录 4 种场景");
    for case in &set.unmeasured {
        assert!(ids_are_unique_across_sets(&set, &case.id));
        assert_eq!(case.category, ACCENT_CATEGORY);
        assert!(
            case.reason.contains("真人"),
            "{}: 原因必须点明缺少真人真值",
            case.id
        );
        assert!(
            case.reason.chars().count() >= 30,
            "{}: 未测原因过短",
            case.id
        );
    }
}

fn ids_are_unique_across_sets(set: &GoldenSet, id: &str) -> bool {
    set.cases.iter().filter(|case| case.id == id).count()
        + set.unmeasured.iter().filter(|case| case.id == id).count()
        == 1
}

#[test]
fn high_cer_forces_guided_practice_and_voice_feedback_has_no_machine_score() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("定位 workspace root");
    let report_path = root.join("docs/reports/asr-cer.json");
    let text = std::fs::read_to_string(&report_path)
        .unwrap_or_else(|error| panic!("读取 CER 报告失败 {}：{error}", report_path.display()));
    let report: CerReport = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 CER 报告失败 {}：{error}", report_path.display()));

    assert!(
        report.measured,
        "诚实性门禁必须读取实测报告，而不是 NOT MEASURED 占位"
    );
    assert!(
        report.overall.cer > report.cer_threshold,
        "当前实测 CER 应超过冻结阈值"
    );
    assert_eq!(report.scoring_mode, "guided_practice");
    assert!(
        matches!(
            report.scoring_mode.as_str(),
            "guided_practice" | "coverage_advisory"
        ),
        "旧 verdict 或未知模式不得重新进入产品：{}",
        report.scoring_mode
    );
    assert!(!report.fixture.human_recordings_used);

    let feedback = VoicePracticeFeedback {
        spoke: true,
        pause_count: 2,
        relative_rhythm: RelativeRhythm::Similar,
    };
    assert!(feedback.spoke);
    assert_eq!(feedback.pause_count, 2);
    assert_eq!(feedback.relative_rhythm, RelativeRhythm::Similar);
}

#[test]
fn wholly_wrong_poems_are_present_and_rejected_by_contract() {
    let cases = golden_set()
        .cases
        .into_iter()
        .filter(|case| case.category == "wrong_poem")
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 4);
    assert!(cases.iter().all(|case| case.is_rejected));
}
