use super::*;
use crate::ingest::Tone;
use crate::model::{RegistryState, Script, SourceLocatorKind, rebuild_corpus};
use std::sync::atomic::{AtomicU64, Ordering};

/// 现代评注字段在 fixture 里被替换成这个哨兵串。它一旦出现在入库产物里，
/// 就证明版权墙漏了。
const SENTINEL: &str = "SENTINEL_MODERN_COMMENTARY_MUST_NEVER_SHIP";

const FIXTURE_TOTAL: usize = 37;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chinese_poetry")
}

fn ingest_fixture() -> IngestOutcome {
    ingest(fixture_root()).expect("fixture 入库应成功")
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "yunjian-ingest-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("创建临时目录");
    path
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("枚举源目录") {
        let entry = entry.expect("读取目录项");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("文件类型").is_dir() {
            std::fs::create_dir_all(&target).expect("创建目录");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("复制文件");
        }
    }
}

fn fixture_copy(label: &str) -> PathBuf {
    let root = temp_dir(label);
    copy_tree(&fixture_root(), &root);
    root
}

#[test]
fn every_declared_file_yields_its_expected_record_count() {
    let outcome = ingest_fixture();
    let expected = [
        ("全唐诗/poet.tang.0.json", 3),
        ("全唐诗/poet.tang.1000.json", 3),
        ("全唐诗/poet.song.0.json", 2),
        ("全唐诗/authors.tang.json", 1),
        ("全唐诗/authors.song.json", 1),
        ("宋词/ci.song.0.json", 2),
        ("诗经/shijing.json", 2),
        ("楚辞/chuci.json", 1),
        ("元曲/yuanqu.json", 2),
        ("五代诗词/huajianji/huajianji-1-juan.json", 2),
        ("五代诗词/nantang/poetrys.json", 2),
        ("五代诗词/nantang/intro.json", 0),
        ("五代诗词/nantang/authors.json", 0),
        ("水墨唐诗/shuimotangshi.json", 2),
        ("幽梦影/youmengying.json", 6),
        ("蒙学/guwenguanzhi.json", 2),
        ("蒙学/shenglvqimeng.json", 2),
        ("蒙学/wenzimengqiu.json", 1),
        ("蒙学/youxueqionglin.json", 2),
        ("蒙学/zengguangxianwen.json", 1),
    ];
    for (path, count) in expected {
        let tally = outcome
            .tally(path)
            .unwrap_or_else(|| panic!("缺少文件账目：{path}"));
        assert_eq!(tally.emitted_records, count, "{path} 入库条数不符");
    }
    assert_eq!(outcome.emitted(), FIXTURE_TOTAL);
    assert_eq!(
        outcome
            .tallies
            .iter()
            .map(|t| t.emitted_records)
            .sum::<usize>(),
        FIXTURE_TOTAL,
        "逐文件账目之和必须等于总入库数"
    );
}

/// 计划的 1% 容差断言。fixture 是每个 genre 的极小切片，因此这里比对的是
/// **fixture 清单声明值**；对锁定 revision 全量的同一断言见
/// [`full_checkout_totals_match_the_manifest_within_one_percent`]。
#[test]
fn fixture_total_is_within_one_percent_of_the_declared_expectation() {
    let outcome = ingest_fixture();
    let declared = FIXTURE_TOTAL as f64;
    let actual = outcome.emitted() as f64;
    assert!(
        ((actual - declared) / declared).abs() <= 0.01,
        "入库总数 {actual} 与声明值 {declared} 相差超过 1%"
    );
}

#[test]
fn script_is_detected_per_record_across_the_same_repository() {
    let outcome = ingest_fixture();
    let traditional = outcome
        .records
        .iter()
        .find(|record| record.script == Script::Traditional)
        .expect("必须至少探测到一条繁体记录");
    assert!(
        traditional.source_locator.as_str().starts_with(SOURCE_NAME),
        "locator 前缀不对：{}",
        traditional.source_locator.as_str()
    );
    assert!(
        outcome
            .records
            .iter()
            .any(|record| record.script == Script::Simplified),
        "必须至少探测到一条简体记录"
    );

    let tang = outcome
        .records
        .iter()
        .find(|record| record.body_original.starts_with("秦川雄帝宅"))
        .expect("全唐诗首条应在库");
    assert_eq!(tang.script, Script::Traditional);
    let ci = outcome
        .records
        .iter()
        .find(|record| record.body_original.starts_with("气和玉烛"))
        .expect("宋词首条应在库");
    assert_eq!(ci.script, Script::Simplified);
}

#[test]
fn strains_attach_to_the_positionally_matching_poem_and_mark_unknown_tones() {
    let outcome = ingest_fixture();
    let tang = outcome
        .records
        .iter()
        .find(|record| record.body_original.starts_with("秦川雄帝宅"))
        .expect("全唐诗首条应在库");
    let strains = outcome
        .strains
        .iter()
        .find(|entry| entry.source_locator == tang.source_locator.as_str())
        .expect("首条应挂上平仄");
    assert_eq!(strains.alignment, StrainAlignment::Positional);
    assert_eq!(strains.lines.len(), tang.body_lines.len());
    assert_eq!(strains.lines[0].raw, "平平平仄仄，平仄仄平平。");
    assert_eq!(strains.lines[0].tones.len(), 10);
    assert_eq!(strains.lines[0].tones[0], Tone::Level);
    assert_eq!(strains.lines[0].tones[3], Tone::Oblique);

    let unknown = outcome
        .strains
        .iter()
        .find(|entry| entry.unknown_count() > 0)
        .expect("fixture 必须含一条带未定音的平仄");
    assert!(
        unknown
            .lines
            .iter()
            .flat_map(|line| &line.tones)
            .any(|tone| *tone == Tone::Unknown),
        "？/○ 必须落到 Unknown"
    );
    assert!(
        unknown
            .lines
            .iter()
            .any(|line| line.raw.contains('○') || line.raw.contains('？')),
        "未定音记号必须原样保留在 raw 里以便复核"
    );
}

/// 上游声称 `strains` 与诗文件逐下标对应，实测三个分片并非如此。错位必须被
/// 发现并按原生 `id` 改挂，而不是把平仄挂到隔壁那首诗上。
#[test]
fn misaligned_strains_are_recovered_by_native_id_and_reported() {
    let outcome = ingest_fixture();
    let recovered = outcome
        .strains
        .iter()
        .filter(|entry| entry.alignment == StrainAlignment::RecoveredByNativeId)
        .count();
    assert!(recovered >= 2, "刻意错位的 fixture 应触发按 id 改挂");
    let reported = outcome
        .defects
        .iter()
        .filter(|defect| defect.reason == DefectReason::StrainsMisaligned)
        .collect::<Vec<_>>();
    assert!(!reported.is_empty(), "错位必须进缺陷报告");
    assert!(
        reported
            .iter()
            .all(|defect| defect.relative_path == "全唐诗/poet.tang.1000.json"),
        "错位记录必须带出文件名"
    );

    for entry in &outcome.strains {
        let record = outcome
            .records
            .iter()
            .find(|record| record.source_locator.as_str() == entry.source_locator)
            .expect("平仄必须挂在真实记录上");
        assert_eq!(
            entry.lines.len(),
            record.body_lines.len(),
            "平仄行数必须与正文行数一致：{}",
            entry.source_locator
        );
    }
}

#[test]
fn native_ids_are_used_as_locators_but_never_as_user_facing_keys() {
    let outcome = ingest_fixture();
    let tang = outcome
        .records
        .iter()
        .find(|record| record.body_original.starts_with("秦川雄帝宅"))
        .expect("全唐诗首条应在库");
    assert_eq!(tang.source_locator.kind(), SourceLocatorKind::Native);
    assert_eq!(
        tang.source_locator.as_str(),
        "chinese-poetry:3ad6d468-7ff1-4a7b-8b24-a27d70d00ed4"
    );

    let ci = outcome
        .records
        .iter()
        .find(|record| record.body_original.starts_with("气和玉烛"))
        .expect("宋词首条应在库");
    assert_eq!(
        ci.source_locator.kind(),
        SourceLocatorKind::Positional,
        "宋词没有原生 id，必须走位置型 locator"
    );

    let output = rebuild_corpus(
        &RegistryState::default(),
        &[],
        outcome.records.clone(),
        "corpus-fixture",
        &[],
    )
    .expect("身份铸造应成功");
    assert_eq!(output.shippable_records.len(), FIXTURE_TOTAL);
    assert!(output.restricted_records.is_empty());
    for record in &output.shippable_records {
        assert_eq!(record.stable_id.len(), 16, "stable_id 必须是 16 位十六进制");
        assert!(
            !record.stable_id.contains('-'),
            "上游 UUID 不得成为面向用户的键：{}",
            record.stable_id
        );
        assert!(
            record.stable_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "stable_id 含非十六进制字符：{}",
            record.stable_id
        );
    }
}

/// 十个 `shippable = false` 文件的现代评注字段一个字都不得进入可分发集合。
#[test]
fn modern_commentary_fields_never_reach_the_shippable_set() {
    let outcome = ingest_fixture();
    let serialized = serde_json::to_string(&outcome.records).expect("序列化候选记录");
    assert!(!serialized.contains(SENTINEL), "现代评注泄漏进了候选记录");

    let output = rebuild_corpus(
        &RegistryState::default(),
        &[],
        outcome.records.clone(),
        "corpus-fixture",
        &[],
    )
    .expect("身份铸造应成功");
    let shippable = serde_json::to_string(&output.shippable_records).expect("序列化可分发记录");
    assert!(!shippable.contains(SENTINEL), "现代评注泄漏进了可分发集合");

    for record in &outcome.records {
        assert!(!record.body_original.contains(SENTINEL));
        assert!(!record.title.contains(SENTINEL));
        assert!(!record.author.contains(SENTINEL));
    }

    assert_eq!(
        dropped_modern_fields(),
        vec!["abstract", "desc", "notes", "preface", "prologue"],
        "现代字段声明表变动必须是有意为之"
    );
}

/// 通篇现代编者文字的文件必须显式排除并留下缺陷记录，而不是被当成空文件放过。
#[test]
fn wholly_modern_files_are_excluded_with_a_recorded_reason() {
    let outcome = ingest_fixture();
    for path in [
        "五代诗词/nantang/intro.json",
        "五代诗词/nantang/authors.json",
    ] {
        let defect = outcome
            .defects
            .iter()
            .find(|defect| {
                defect.relative_path == path
                    && defect.reason == DefectReason::ModernCommentaryInseparable
            })
            .unwrap_or_else(|| panic!("{path} 必须有排除记录"));
        assert!(defect.detail.contains("shippable=false"));
        assert_eq!(outcome.tally(path).expect("应有账目").emitted_records, 0);
    }
}

/// 判据是文本是否前现代，不是字段名危不危险：清人评语与原书文言小传照发。
#[test]
fn pre_modern_commentary_and_biographies_are_shipped() {
    let outcome = ingest_fixture();
    let commentary = outcome
        .records
        .iter()
        .filter(|record| record.provenance.kind == ProvenanceKind::PublicDomainCommentary)
        .collect::<Vec<_>>();
    assert!(!commentary.is_empty(), "幽梦影的清人评语必须入库");
    assert!(
        commentary.iter().any(|record| record.author == "曹秋岳"),
        "评者应从「曹秋岳曰」中拆出"
    );
    assert!(
        commentary
            .iter()
            .all(|record| !record.body_original.contains('曰')
                || !record.body_original.starts_with("曹秋岳")),
        "评语正文不应重复带上评者前缀"
    );

    let biography = outcome
        .records
        .iter()
        .find(|record| record.title == "太宗皇帝 小传")
        .expect("全唐诗原书文言小传必须入库");
    assert_eq!(biography.provenance.kind, ProvenanceKind::Original);
    assert!(biography.body_original.starts_with("帝姓李氏"));
    assert_eq!(biography.dynasty, Dynasty::Tang);
}

#[test]
fn four_body_granularities_are_each_handled() {
    let outcome = ingest_fixture();
    let by_title = |needle: &str| {
        outcome
            .records
            .iter()
            .find(|record| record.title.contains(needle))
            .unwrap_or_else(|| panic!("缺少记录：{needle}"))
    };

    let lines = by_title("帝京篇");
    assert_eq!(lines.body_lines.len(), 4, "paragraphs[] 应逐行入库");

    let chapters = by_title("关雎");
    assert_eq!(chapters.title, "国风/周南/关雎");
    assert_eq!(chapters.body_lines.len(), 5, "content[] 应逐章入库");

    let single = by_title("太宗皇帝 小传");
    assert_eq!(single.body_lines.len(), 1, "单串正文应是一行");

    let nested = outcome
        .records
        .iter()
        .find(|record| record.title.starts_with("古文觀止/"))
        .expect("蒙学嵌套卷—篇结构应入库");
    assert!(
        nested.title.matches('/').count() >= 2,
        "嵌套标题应保留卷与篇：{}",
        nested.title
    );
    assert!(!nested.body_lines.is_empty());
}

#[test]
fn dynasty_is_canonicalized_while_the_original_string_survives() {
    let outcome = ingest_fixture();
    let qu = outcome
        .records
        .iter()
        .find(|record| record.genre == Genre::Qu)
        .expect("元曲应入库");
    assert_eq!(qu.dynasty, Dynasty::Yuan);
    assert_eq!(qu.dynasty_raw, "yuan", "字面串 yuan 必须原样保留");

    let guwen = outcome
        .records
        .iter()
        .find(|record| record.title.starts_with("古文觀止/"))
        .expect("古文观止应入库");
    assert_eq!(guwen.dynasty, Dynasty::PreQin, "逐篇朝代应取自 author 前缀");
    assert_eq!(
        guwen.author, "左丘明",
        "author 前缀里的朝代不应留在作者名里"
    );
}

/// `蒙学/wenzimengqiu.json` 的 `author` 字段装的是现代传记，只能取姓名。
#[test]
fn modern_biography_in_an_author_field_is_truncated_to_the_name() {
    let outcome = ingest_fixture();
    let record = outcome
        .records
        .iter()
        .find(|record| record.title.starts_with("文字蒙求/"))
        .expect("文字蒙求应入库");
    assert_eq!(record.author, "王筠");
    assert!(!record.author.contains("1784"));
    assert!(!record.author.contains("道光"));
}

/// 失败场景：把某个分片截成空 JSON 数组，入库必须带文件名大声失败。
#[test]
fn an_emptied_shard_fails_loudly_with_its_file_name() {
    let root = fixture_copy("empty-shard");
    let target = root.join("宋词/ci.song.0.json");
    std::fs::write(&target, "[]\n").expect("写入空数组");

    let error = ingest(&root).expect_err("空文件必须让入库失败");
    let message = error.to_string();
    assert!(
        message.contains("宋词/ci.song.0.json"),
        "错误信息必须带出文件名：{message}"
    );
    assert!(message.contains("0 条记录"), "{message}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_renamed_shard_family_fails_rather_than_silently_shrinking() {
    let root = fixture_copy("renamed-family");
    std::fs::remove_file(root.join("宋词/ci.song.0.json")).expect("删除唯一分片");

    let error = ingest(&root).expect_err("整个资产家族消失必须失败");
    assert!(error.to_string().contains("宋词"), "{}", error.to_string());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn commentator_split_tolerates_prose_without_a_speaker() {
    assert_eq!(
        split_commentator("曹秋岳曰：可想见其南面百城时。"),
        ("曹秋岳".to_owned(), "可想见其南面百城时。".to_owned())
    );
    assert_eq!(
        split_commentator("读《幽梦影》则春、夏、秋、冬，无时不宜。"),
        (
            "佚名".to_owned(),
            "读《幽梦影》则春、夏、秋、冬，无时不宜。".to_owned()
        )
    );
}

#[test]
fn name_before_paren_handles_both_bracket_widths() {
    assert_eq!(name_before_paren("王筠（1784-1854），字貫山"), "王筠");
    assert_eq!(name_before_paren("程登吉 (明)"), "程登吉");
    assert_eq!(name_before_paren("車萬育"), "車萬育");
}

/// 锁定 revision 全量检出上的 1% 容差断言。
///
/// 默认不跑：需要 455 MB 的上游检出。设置 `YUNJIAN_CHINESE_POETRY_DIR` 指向
/// `b8594f81…` 的检出后以 `cargo test -- --ignored` 运行。
#[test]
#[ignore = "需要锁定 revision 的完整上游检出，由 YUNJIAN_CHINESE_POETRY_DIR 指定"]
fn full_checkout_totals_match_the_manifest_within_one_percent() {
    let Ok(root) = std::env::var("YUNJIAN_CHINESE_POETRY_DIR") else {
        panic!("请设置 YUNJIAN_CHINESE_POETRY_DIR 指向锁定 revision 的检出");
    };
    let outcome = ingest(&root).expect("全量入库应成功");
    let declared = expected_total_records() as f64;
    let actual = outcome.emitted() as f64;
    assert!(
        ((actual - declared) / declared).abs() <= 0.01,
        "全量入库 {actual} 条与清单声明 {declared} 条相差超过 1%"
    );
    for asset in ASSETS {
        if asset.expected_records == 0 {
            continue;
        }
        let prefix = match asset.select {
            FileSelect::Exact(name) => format!("{}/{name}", asset.dir),
            FileSelect::Prefixed(prefix) => format!("{}/{prefix}", asset.dir),
        };
        let emitted = outcome.emitted_under(&prefix) as f64;
        let expected = asset.expected_records as f64;
        assert!(
            ((emitted - expected) / expected).abs() <= 0.01,
            "资产 {prefix} 入库 {emitted} 条，声明 {expected} 条，超出 1%"
        );
    }

    let unusable = outcome
        .defects
        .iter()
        .filter(|defect| {
            matches!(
                defect.reason,
                DefectReason::StrainsUnavailable | DefectReason::StrainsLineMismatch
            )
        })
        .count();
    assert_eq!(
        outcome.strains.len() + unusable,
        outcome.emitted_under("全唐诗/poet."),
        "每首诗要么挂上平仄，要么留下一条可查的缺陷，不得静默丢失"
    );
    assert!(
        unusable * 100 < outcome.emitted_under("全唐诗/poet."),
        "不可用平仄超过 1%，上游对齐可能已经变形：{unusable} 条"
    );
    let by_locator = outcome
        .records
        .iter()
        .map(|record| (record.source_locator.as_str(), record))
        .collect::<std::collections::BTreeMap<_, _>>();
    for entry in &outcome.strains {
        let record = by_locator
            .get(entry.source_locator.as_str())
            .expect("平仄必须挂在真实记录上");
        assert!(!entry.lines.is_empty(), "不得挂上空平仄");
        assert_eq!(
            entry.lines.len(),
            record.body_lines.len(),
            "已挂平仄的行数必须与正文一致：{}",
            entry.source_locator
        );
    }
    assert!(
        outcome
            .strains
            .iter()
            .map(RecordStrains::unknown_count)
            .sum::<usize>()
            > 0,
        "上游用 ？/○ 标未定音，全量入库必须出现 Unknown"
    );
    for script in [Script::Traditional, Script::Simplified, Script::Mixed] {
        assert!(
            outcome.records.iter().any(|record| record.script == script),
            "同一仓库内应同时出现三种书写系统判定，缺少 {script:?}"
        );
    }
    assert!(
        outcome
            .defects
            .iter()
            .all(|defect| defect.reason != DefectReason::UnknownDynasty),
        "全量入库不应剩下无法归一的朝代串"
    );
}
