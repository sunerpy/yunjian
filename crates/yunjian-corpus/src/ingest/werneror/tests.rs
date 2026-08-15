use super::*;
use crate::ingest::chinese_poetry;
use crate::model::{Script, split_ci_tune};
use std::sync::atomic::{AtomicU64, Ordering};

/// fixture 里现代与未知分桶的占位作者。它们一旦出现在任何入库产物里，
/// 就说明白名单漏了。
const MODERN_SENTINELS: [&str; 4] = ["占位作者甲", "占位作者乙", "占位作者丙", "占位作者丁"];

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/werneror")
}

fn chinese_poetry_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chinese_poetry")
}

fn sources_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sources.toml")
}

fn fixture_buckets() -> Vec<Bucket> {
    buckets_by_file(FIXTURE_BUCKETS).expect("fixture 分桶都应在白名单上")
}

fn ingest_fixture(covered: &CoveredWorks) -> WernerorOutcome {
    ingest_buckets(fixture_root(), &fixture_buckets(), covered).expect("fixture 入库应成功")
}

fn chinese_poetry_records() -> Vec<RecordInput> {
    chinese_poetry::ingest(chinese_poetry_fixture_root())
        .expect("chinese-poetry fixture 入库")
        .records
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "yunjian-werneror-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("创建临时目录");
    path
}

fn fixture_copy(label: &str) -> PathBuf {
    let root = temp_dir(label);
    for entry in std::fs::read_dir(fixture_root()).expect("枚举 fixture") {
        let entry = entry.expect("读取目录项");
        std::fs::copy(entry.path(), root.join(entry.file_name())).expect("复制 fixture 文件");
    }
    root
}

fn record_by_title<'a>(outcome: &'a WernerorOutcome, title_raw: &str) -> Option<&'a RecordInput> {
    outcome
        .records
        .iter()
        .find(|record| record.title_raw == title_raw)
}

#[test]
fn allow_list_is_the_manifest_and_holds_no_modern_bucket() {
    let raw = std::fs::read_to_string(sources_manifest()).expect("读取 corpus/sources.toml");
    let manifest: toml::Value = toml::from_str(&raw).expect("解析 corpus/sources.toml");
    let source = manifest
        .get("source")
        .and_then(toml::Value::as_array)
        .expect("清单里应有 [[source]]")
        .iter()
        .find(|source| source.get("name").and_then(toml::Value::as_str) == Some("Werneror/Poetry"))
        .expect("清单里应有 Werneror/Poetry");
    assert_eq!(
        source.get("git_rev").and_then(toml::Value::as_str),
        Some(SOURCE_REV),
        "入库锁定的 revision 必须与清单一致"
    );

    let mut shippable = BTreeSet::new();
    let mut withheld = BTreeSet::new();
    for asset in source
        .get("assets")
        .and_then(toml::Value::as_array)
        .expect("Werneror 条目应有 assets")
    {
        let path = asset
            .get("path")
            .and_then(toml::Value::as_str)
            .expect("资产应有 path");
        let is_shippable = asset
            .get("shippable")
            .and_then(toml::Value::as_bool)
            .expect("资产应有 shippable");
        // 清单把分片家族写成一个逗号分隔的 path（如 `宋_1.csv, 宋_2.csv, …`）。
        for file in path
            .split(',')
            .map(str::trim)
            .filter(|file| file.ends_with(".csv"))
        {
            if is_shippable {
                shippable.insert(file.to_owned());
            } else {
                withheld.insert(file.to_owned());
            }
        }
    }

    let allow_list: BTreeSet<String> = CLASSICAL_BUCKETS
        .iter()
        .map(|bucket| bucket.file.to_owned())
        .collect();
    assert_eq!(
        allow_list.len(),
        CLASSICAL_BUCKETS.len(),
        "白名单里有重复文件名"
    );
    assert_eq!(
        allow_list, shippable,
        "白名单与 sources.toml 的可分发 CSV 不一致"
    );
    assert_eq!(allow_list.len(), 28, "古典分桶应为 28 个");
    assert_eq!(withheld.len(), 6, "扣留的现代分桶应为 6 个");
    for file in &withheld {
        assert!(
            !allow_list.contains(file),
            "白名单里出现了被扣留的分桶 {file}"
        );
    }
    let known: BTreeSet<String> = KNOWN_MODERN_BUCKETS
        .iter()
        .map(|file| (*file).to_owned())
        .collect();
    assert_eq!(known, withheld, "已知现代分桶名单应与清单里被扣留的一致");
}

/// `FIXTURE_BUCKETS` 必须逐条等于 fixture 目录里真实存在且在白名单上的 CSV。
///
/// 它扫真实目录而不读任何记录值：两边任一侧漂移都会让「按 fixture 裁剪分桶」的
/// 调用方去要一个不存在的文件——那正是 `corpus-measure --scale 10k` 曾经报成
/// 「数据缺失」的成因。常量自锁的 7 是为了挡住「测试红了就把新文件加进名单」这条
/// 捷径：真要改 fixture 覆盖范围，先改方案。
#[test]
fn fixture_bucket_list_matches_the_fixture_directory() {
    assert_eq!(
        FIXTURE_BUCKETS.len(),
        7,
        "要调整 fixture 覆盖范围先改方案，不要改这条断言"
    );
    let allow_list: BTreeSet<&str> = CLASSICAL_BUCKETS.iter().map(|bucket| bucket.file).collect();
    let on_disk: BTreeSet<String> = std::fs::read_dir(fixture_root())
        .expect("读取 werneror fixture 目录")
        .map(|entry| {
            entry
                .expect("目录项")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| allow_list.contains(name.as_str()))
        .collect();
    let declared: BTreeSet<String> = FIXTURE_BUCKETS
        .iter()
        .map(|file| (*file).to_owned())
        .collect();
    assert_eq!(
        declared, on_disk,
        "FIXTURE_BUCKETS 与 fixture 目录里在白名单上的 CSV 不一致"
    );
    for bucket in buckets_by_file(FIXTURE_BUCKETS).expect("fixture 分桶都应在白名单上") {
        assert!(
            fixture_root().join(bucket.file).is_file(),
            "fixture 目录缺 {}",
            bucket.file
        );
    }
}

#[test]
fn every_declared_file_yields_its_expected_record_count() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let expected = [
        ("先秦.csv", 3, 2),
        ("秦.csv", 2, 2),
        ("魏晋末南北朝初.csv", 1, 1),
        ("隋末唐初.csv", 2, 1),
        ("唐.csv", 2, 2),
        ("宋末金初.csv", 4, 3),
        ("辽.csv", 3, 3),
    ];
    for (file, input, emitted) in expected {
        let tally = outcome
            .tally(file)
            .unwrap_or_else(|| panic!("缺少文件账目：{file}"));
        assert_eq!(tally.input_records, input, "{file} 读入行数不符");
        assert_eq!(tally.emitted_records, emitted, "{file} 入库条数不符");
        assert_eq!(
            tally.input_records,
            tally.emitted_records + outcome.quarantined_in(file) + outcome.duplicates_in(file),
            "{file} 的读入行数必须等于入库 + 隔离 + 判重，不得有行凭空消失"
        );
    }
    assert_eq!(outcome.emitted(), 14);
    assert_eq!(
        outcome
            .tallies
            .iter()
            .map(|tally| tally.emitted_records)
            .sum::<usize>(),
        outcome.emitted(),
        "逐文件账目之和必须等于总入库数"
    );
}

#[test]
fn modern_bucket_is_excluded_by_policy_and_counted() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let exclusion = outcome
        .exclusion("当代.csv")
        .expect("当代.csv 应被记为策略排除");
    assert_eq!(
        exclusion.reason,
        ExclusionReason::ModernAuthorsLikelyInCopyright
    );
    assert_eq!(exclusion.rows, 2, "被排除的行数应照数不照收");
    assert!(
        exclusion.detail.contains("保护期"),
        "排除理由应写明保护期：{}",
        exclusion.detail
    );

    assert!(
        outcome.tally("当代.csv").is_none(),
        "被排除的分桶不该出现在入库账目里"
    );
    assert!(
        outcome
            .records
            .iter()
            .all(|record| record.dynasty_raw != "当代"),
        "当代记录不得进入入库产物"
    );
    for sentinel in MODERN_SENTINELS {
        assert!(
            outcome
                .records
                .iter()
                .all(|record| record.author != sentinel)
                && outcome
                    .quarantined
                    .iter()
                    .all(|record| record.author != sentinel),
            "现代分桶的哨兵作者 {sentinel} 出现在产物里，白名单漏了"
        );
    }
}

/// 失败场景：白名单之外的分桶必须带策略理由被排除，而不是被入库。
#[test]
fn unknown_bucket_defaults_to_excluded() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let exclusion = outcome
        .exclusion("未来.csv")
        .expect("未来.csv 应被记为策略排除");
    assert_eq!(exclusion.reason, ExclusionReason::NotOnClassicalAllowList);
    assert_eq!(exclusion.rows, 2);
    assert!(
        exclusion.detail.contains("白名单"),
        "排除理由应写明不在白名单上：{}",
        exclusion.detail
    );
    assert!(
        outcome
            .records
            .iter()
            .all(|record| record.dynasty_raw != "未来"),
        "白名单外的分桶不得入库"
    );
    assert_eq!(
        outcome.exclusions.len(),
        2,
        "fixture 里应恰好排除 当代 与 未来 两个分桶"
    );
    assert_eq!(outcome.excluded_rows(), 4);
}

#[test]
fn lossy_records_are_quarantined_reported_and_kept_out_of_the_main_table() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let expected = [
        ("先秦.csv", "狐援辞", vec![LossyField::Body]),
        (
            "隋末唐初.csv",
            "五郊乐章 赤帝徵音 雍和",
            vec![LossyField::Body],
        ),
        (
            "宋末金初.csv",
            "小重山 予可自解?霜脂粉也",
            vec![LossyField::Title, LossyField::Body],
        ),
    ];
    for (file, title_raw, fields) in expected {
        let quarantined = outcome
            .quarantined
            .iter()
            .find(|record| record.relative_path == file && record.title_raw == title_raw)
            .unwrap_or_else(|| panic!("{file} 的《{title_raw}》应被隔离"));
        assert!(quarantined.has_lossy_char);
        assert_eq!(quarantined.lossy_fields, fields, "{title_raw} 的缺字列不符");
        let defect = outcome
            .defects
            .iter()
            .find(|defect| defect.relative_path == file && defect.ordinal == quarantined.ordinal)
            .unwrap_or_else(|| panic!("{file} 的《{title_raw}》应有缺陷记录"));
        assert_eq!(defect.reason, DefectReason::LossyCharacter);
        assert!(defect.detail.contains("不可恢复"), "缺陷说明应写明不可恢复");
        assert!(
            record_by_title(&outcome, title_raw).is_none(),
            "《{title_raw}》不得进入主表"
        );
    }
    assert_eq!(outcome.quarantined.len(), 3);
    for record in &outcome.records {
        for text in [
            &record.title,
            &record.title_raw,
            &record.author,
            &record.body_original,
        ] {
            assert!(!has_lossy_char(text), "主表里出现了缺字文本：{text}");
        }
    }
}

/// 计划点名的形态：正文里的「李?」。fixture 保持上游原样，因此这一条追加到
/// 临时副本上，既拿到字面断言，又不把编造的行混进 committed fixture。
#[test]
fn body_containing_li_question_mark_is_flagged_and_reported() {
    assert!(has_lossy_char("李?"));
    let root = fixture_copy("li-question-mark");
    let appended = "\"赠李?\",\"唐\",\"佚名\",\"江上逢李?，相看泪满巾。\"\n";
    let path = root.join("唐.csv");
    let mut raw = std::fs::read_to_string(&path).expect("读取 唐.csv");
    raw.push_str(appended);
    std::fs::write(&path, raw).expect("写回 唐.csv");

    let outcome =
        ingest_buckets(&root, &fixture_buckets(), &CoveredWorks::empty()).expect("入库应成功");
    let quarantined = outcome
        .quarantined
        .iter()
        .find(|record| record.title_raw == "赠李?")
        .expect("含「李?」的记录应被隔离");
    assert!(quarantined.has_lossy_char);
    assert_eq!(
        quarantined.lossy_fields,
        vec![LossyField::Title, LossyField::Body]
    );
    assert!(
        quarantined.body_original.contains("李?"),
        "隔离留档应保留原文"
    );
    assert!(
        outcome.defects.iter().any(|defect| {
            defect.relative_path == "唐.csv"
                && defect.ordinal == quarantined.ordinal
                && defect.reason == DefectReason::LossyCharacter
        }),
        "含「李?」的记录应出现在缺陷报告里"
    );
    assert!(record_by_title(&outcome, "赠李?").is_none());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn lone_question_mark_is_lossy_only_inside_cjk_context() {
    for lossy in [
        "李?",
        "鲋入而?居。",
        "琼羞溢俎，玉?浮觞。",
        "?车既工。",
        "麀鹿??。",
        "其来趩趩。????。",
        "?",
    ] {
        assert!(has_lossy_char(lossy), "应判为缺字：{lossy}");
    }
    for clean in [
        "问君何能尔？心远地自偏。",
        "明月几时有",
        "",
        "why not?",
        "SELECT ?",
    ] {
        assert!(!has_lossy_char(clean), "不应判为缺字：{clean}");
    }
}

#[test]
fn poem_present_in_both_sources_resolves_to_the_chinese_poetry_copy() {
    let detector = ScriptDetector::new().expect("初始化繁简探测器");
    let chinese_poetry_records = chinese_poetry_records();
    let covered = CoveredWorks::from_records(&detector, &chinese_poetry_records);
    assert!(!covered.is_empty());
    assert_eq!(
        covered.len(),
        chinese_poetry_records.len(),
        "fixture 内不应有重复作品"
    );

    let without = ingest_fixture(&CoveredWorks::empty());
    let with = ingest_fixture(&covered);
    for title_raw in ["帝京篇十首 一", "关雎"] {
        assert!(
            record_by_title(&without, title_raw).is_some(),
            "不判重时《{title_raw}》应当入库，否则这个测试证明不了判重是原因"
        );
        assert!(
            record_by_title(&with, title_raw).is_none(),
            "《{title_raw}》已由 chinese-poetry 收录，Werneror 不应重复入库"
        );
        let duplicate = with
            .duplicates
            .iter()
            .find(|record| record.title_raw == title_raw)
            .unwrap_or_else(|| panic!("《{title_raw}》应记入判重清单"));
        let survivors = chinese_poetry_records
            .iter()
            .filter(|record| {
                dedup_key(&detector, &record.body_lines.join("\n")) == duplicate.work_group
            })
            .collect::<Vec<_>>();
        assert!(
            !survivors.is_empty(),
            "留下来的那一份必须来自 chinese-poetry"
        );
        for survivor in survivors {
            assert_eq!(survivor.provenance.source_name, chinese_poetry::SOURCE_NAME);
        }
    }
    assert_eq!(with.duplicates.len(), 2);
    assert_eq!(with.emitted(), without.emitted() - 2);
}

/// 判重键必须跨越繁简：`chinese-poetry/全唐诗` 是繁体、Werneror 是简体，
/// 按原字形算 `work_group` 两边分属不同组，一条都判不出来。
#[test]
fn dedup_key_bridges_the_traditional_and_simplified_copies() {
    let detector = ScriptDetector::new().expect("初始化繁简探测器");
    let traditional = "秦川雄帝宅，函谷壯皇居。";
    let simplified = "秦川雄帝宅，函谷壮皇居。";
    assert_ne!(
        crate::model::compute_work_group(traditional),
        crate::model::compute_work_group(simplified),
        "原字形的 work_group 本就不同，这正是不能直接拿它判重的理由"
    );
    assert_eq!(
        dedup_key(&detector, traditional),
        dedup_key(&detector, simplified),
        "简体化后的判重键必须相同"
    );
}

/// 已知缺口，落在代码里而不是只写在笔记里：`t2s` 把 `餘` 归一成 `余`，却让
/// 异体字 `馀` 原样通过。两边都真实出现在上游（Werneror 自己既写「送寒馀雪尽」
/// 也写「有余香」），于是「百雉餘」与「百雉馀」判不出重，会各自入库一条。
/// 这条断言故意锁住**当前**行为：等构建期的字形映射表把异体字一并归一，它就会
/// 失败，从而逼着那一步显式决定要不要把异体字纳入判重键。
#[test]
fn known_gap_the_yu_variant_is_not_normalized_by_t2s() {
    let detector = ScriptDetector::new().expect("初始化繁简探测器");
    assert_eq!(detector.simplify("餘"), "余");
    assert_eq!(detector.simplify("馀"), "馀", "异体字 馀 未被 t2s 归一");
    assert_ne!(
        dedup_key(&detector, "离宫百雉余。"),
        dedup_key(&detector, "离宫百雉馀。"),
        "异体字未归一，同一首诗的两种写法目前判不出重"
    );
}

#[test]
fn ci_tune_is_split_only_for_whitelisted_tunes() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let split = record_by_title(&outcome, "人月圆 宴北人张侍御家有感").expect("应有这一首词");
    assert_eq!(split.genre, Genre::Ci);
    assert_eq!(split.title, "人月圆·宴北人张侍御家有感");
    assert_eq!(split_ci_tune(&split.title).as_deref(), Some("人月圆"));
    assert_eq!(
        split.title_raw, "人月圆 宴北人张侍御家有感",
        "上游原串必须原样保留"
    );

    let bare = record_by_title(&outcome, "长相思").expect("应有这一首词");
    assert_eq!(bare.genre, Genre::Ci);
    assert_eq!(bare.title, "长相思");

    let shi = record_by_title(&outcome, "杂诗二首 其一").expect("应有这一首诗");
    assert_eq!(shi.genre, Genre::Shi);
    assert_eq!(shi.title, "杂诗二首 其一");
    assert_eq!(split_ci_tune(&shi.title), None);
}

#[test]
fn two_character_tunes_do_not_hijack_poem_titles() {
    let cipai = CipaiList::load(fixture_root().join(CIPAI_FILE)).expect("读取词牌白名单");
    assert!(cipai.contains("九日") && !cipai.is_empty());
    assert_eq!(cipai.len(), 7);
    assert_eq!(
        resolve_title("九日 登高", &cipai),
        ("九日 登高".to_owned(), Genre::Shi)
    );
    assert_eq!(
        resolve_title("九日", &cipai),
        ("九日".to_owned(), Genre::Shi)
    );
    assert_eq!(
        resolve_title("念奴娇·赤壁怀古", &cipai),
        ("念奴娇·赤壁怀古".to_owned(), Genre::Ci)
    );
    assert_eq!(
        resolve_title("如梦令 昨夜雨疏风骤", &cipai),
        ("如梦令·昨夜雨疏风骤".to_owned(), Genre::Ci)
    );
    assert_eq!(
        resolve_title("春日·其一", &cipai),
        ("春日·其一".to_owned(), Genre::Shi)
    );
}

#[test]
fn positional_locators_are_used_because_the_csv_has_no_native_key() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    assert!(!outcome.records.is_empty());
    for record in &outcome.records {
        assert_eq!(
            record.source_locator.kind(),
            crate::model::SourceLocatorKind::Positional
        );
        assert!(
            record.source_locator.as_str().starts_with("werneror:"),
            "locator 前缀应为来源名：{}",
            record.source_locator.as_str()
        );
    }
    let first = record_by_title(&outcome, "白水诗").expect("应有《白水诗》");
    assert_eq!(first.source_locator.as_str(), "werneror:先秦.csv:0");
    let locators = outcome
        .records
        .iter()
        .map(|record| record.source_locator.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        locators.len(),
        outcome.records.len(),
        "locator 必须逐条唯一"
    );
}

#[test]
fn bodies_are_split_into_lines_and_dynasties_are_canonicalized() {
    let outcome = ingest_fixture(&CoveredWorks::empty());
    let record = record_by_title(&outcome, "春晓").expect("应有《春晓》");
    assert_eq!(
        record.body_lines,
        vec!["春眠不觉晓，处处闻啼鸟。", "夜来风雨声，花落知多少。"]
    );
    assert_eq!(record.dynasty, Dynasty::Tang);
    assert_eq!(record.dynasty_raw, "唐");
    assert_eq!(record.script, Script::Simplified);
    assert_eq!(record.provenance.source_name, SOURCE_NAME);
    assert_eq!(record.provenance.source_rev, SOURCE_REV);
    assert_eq!(record.provenance.license_class, LicenseClass::PublicDomain);
    assert_eq!(record.provenance.kind, ProvenanceKind::Original);

    // 跨朝代标签也必须归一，否则该桶唯一的一条会被当成无法归一而丢掉。
    let cross = record_by_title(&outcome, "璇玑图诗").expect("应有《璇玑图诗》");
    assert_eq!(cross.dynasty, Dynasty::Jin);
    assert_eq!(cross.dynasty_raw, "魏晋末南北朝初");
    for bucket in CLASSICAL_BUCKETS {
        assert!(
            Dynasty::canonicalize(bucket.dynasty_label).is_ok(),
            "分桶 {} 的朝代标签「{}」无法归一，该桶会整桶落进缺陷报告",
            bucket.file,
            bucket.dynasty_label
        );
    }
    assert!(
        outcome
            .defects
            .iter()
            .all(|defect| defect.reason != DefectReason::UnknownDynasty),
        "fixture 不该剩下无法归一的朝代串"
    );
}

#[test]
fn quoted_fields_and_both_line_endings_are_parsed() {
    let text = "\"题目\",\"朝代\",\"作者\",\"内容\"\r\n\
                \"甲\",\"唐\",\"某\",\"含，逗号与\"\"引号\"\"的正文\"\r\n\
                \"乙\",\"唐\",\"某\",\"第一行\n第二行\"\n";
    let rows = parse_csv(text).expect("应能解析");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], HEADER.map(str::to_owned).to_vec());
    assert_eq!(rows[1][3], "含，逗号与\"引号\"的正文");
    assert_eq!(rows[2][3], "第一行\n第二行");
    assert!(parse_csv("\"未闭合").is_err(), "引号未闭合应报错而不是猜测");
}

#[test]
fn header_change_fails_with_the_file_name() {
    let root = fixture_copy("header");
    let path = root.join("秦.csv");
    let raw = std::fs::read_to_string(&path).expect("读取 秦.csv");
    let rewritten = raw.replacen("\"题目\"", "\"标题\"", 1);
    std::fs::write(&path, rewritten).expect("写回 秦.csv");
    let error = ingest_buckets(&root, &fixture_buckets(), &CoveredWorks::empty())
        .expect_err("表头变化必须失败");
    let message = error.to_string();
    assert!(message.contains("秦.csv"), "错误必须点名文件：{message}");
    assert!(message.contains("表头"), "错误应说明是表头问题：{message}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn header_only_file_fails_with_the_file_name() {
    let root = fixture_copy("empty");
    std::fs::write(root.join("辽.csv"), "\"题目\",\"朝代\",\"作者\",\"内容\"\n")
        .expect("截断 辽.csv");
    let error = ingest_buckets(&root, &fixture_buckets(), &CoveredWorks::empty())
        .expect_err("零条数据行必须失败");
    let message = error.to_string();
    assert!(message.contains("辽.csv"), "错误必须点名文件：{message}");
    assert!(message.contains("空吞"), "错误应说明不得空吞：{message}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_declared_bucket_that_is_absent_fails() {
    let error = ingest(fixture_root(), &CoveredWorks::empty())
        .expect_err("fixture 只有 7 个古典分桶，全量入库必须因缺文件而失败");
    assert!(
        error.to_string().contains("汉.csv"),
        "错误必须点名缺失的文件"
    );
}

#[test]
fn an_empty_cipai_allow_list_fails() {
    let root = fixture_copy("cipai");
    std::fs::write(root.join(CIPAI_FILE), "\n\n").expect("清空词牌白名单");
    let error = CipaiList::load(root.join(CIPAI_FILE)).expect_err("空白名单必须失败");
    assert!(
        error.to_string().contains("词牌白名单"),
        "错误应说明白名单为空"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn row_whose_dynasty_does_not_match_its_bucket_is_not_ingested() {
    let root = fixture_copy("label");
    let path = root.join("辽.csv");
    let mut raw = std::fs::read_to_string(&path).expect("读取 辽.csv");
    raw.push_str("\"混入的一条\",\"当代\",\"占位作者甲\",\"分桶标签与朝代列不符。\"\n");
    std::fs::write(&path, raw).expect("写回 辽.csv");
    let outcome =
        ingest_buckets(&root, &fixture_buckets(), &CoveredWorks::empty()).expect("入库应成功");
    assert!(
        outcome
            .records
            .iter()
            .all(|record| record.title_raw != "混入的一条"),
        "朝代列与分桶不符的行不得入库"
    );
    assert!(
        outcome.defects.iter().any(|defect| {
            defect.relative_path == "辽.csv" && defect.reason == DefectReason::BucketLabelMismatch
        }),
        "应留下一条可查的分桶标签不符缺陷"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 逐来源取舍的实测值：`chinese-poetry` 全量在前，Werneror 只补它没有的古典诗。
///
/// 两个数字都是在锁定 revision 上跑出来的，写死在这里是为了让**随包记录数是一个
/// 声明过的数字**，而不是两次重叠导入撞出来的结果：上游哪次改动让重叠范围变了，
/// 这个断言就会失败，而不是让语料悄悄多出或少掉几千首。
const FULL_DUPLICATES_WITH_CHINESE_POETRY: usize = 218_750;
const FULL_EMITTED_AFTER_DEDUP: usize = 538_679;

/// 默认不跑：需要 Werneror 与 `chinese-poetry` 两份完整检出。设置
/// `YUNJIAN_WERNEROR_DIR` 与 `YUNJIAN_CHINESE_POETRY_DIR` 后以
/// `cargo test -- --ignored` 运行。
#[test]
#[ignore = "需要 Werneror 与 chinese-poetry 两份完整上游检出"]
fn full_checkout_contributes_only_the_works_chinese_poetry_lacks() {
    let (Ok(werneror_root), Ok(chinese_poetry_root)) = (
        std::env::var("YUNJIAN_WERNEROR_DIR"),
        std::env::var("YUNJIAN_CHINESE_POETRY_DIR"),
    ) else {
        panic!("请同时设置 YUNJIAN_WERNEROR_DIR 与 YUNJIAN_CHINESE_POETRY_DIR");
    };
    let detector = ScriptDetector::new().expect("初始化繁简探测器");
    let covered = {
        let chinese_poetry = chinese_poetry::ingest(&chinese_poetry_root).expect("上游入库");
        CoveredWorks::from_records(&detector, &chinese_poetry.records)
    };
    let outcome = ingest(&werneror_root, &covered).expect("Werneror 入库应成功");
    assert!(
        !outcome.duplicates.is_empty(),
        "两份语料在唐宋上大量重叠，判重一条都没判掉说明判重键失效了"
    );
    for record in &outcome.records {
        let key = dedup_key(&detector, &record.body_lines.join("\n"));
        assert!(
            !covered.contains(&key),
            "入库的《{}》其实已由 chinese-poetry 收录",
            record.title_raw
        );
    }
    let rows: usize = outcome
        .tallies
        .iter()
        .map(|tally| tally.input_records)
        .sum();
    assert_eq!(
        rows,
        outcome.emitted() + outcome.quarantined.len() + outcome.duplicates.len(),
        "读入行数必须等于入库 + 隔离 + 判重"
    );
    assert_eq!(
        (outcome.duplicates.len(), outcome.emitted()),
        (
            FULL_DUPLICATES_WITH_CHINESE_POETRY,
            FULL_EMITTED_AFTER_DEDUP
        ),
        "逐来源取舍的记录数与声明值不符（covered 键 {} 个）",
        covered.len()
    );
}

/// 默认不跑：需要锁定 revision 上约 380 MB 的上游检出。设置
/// `YUNJIAN_WERNEROR_DIR` 后以 `cargo test -- --ignored` 运行。
#[test]
#[ignore = "需要锁定 revision 的完整上游检出，由 YUNJIAN_WERNEROR_DIR 指定"]
fn full_checkout_totals_match_the_manifest_within_one_percent() {
    let Ok(root) = std::env::var("YUNJIAN_WERNEROR_DIR") else {
        panic!("请设置 YUNJIAN_WERNEROR_DIR 指向锁定 revision 的检出");
    };
    let outcome = ingest(&root, &CoveredWorks::empty()).expect("全量入库应成功");
    for bucket in CLASSICAL_BUCKETS {
        let tally = outcome
            .tally(bucket.file)
            .unwrap_or_else(|| panic!("缺少文件账目：{}", bucket.file));
        let actual = tally.input_records as f64;
        let declared = bucket.expected_rows as f64;
        assert!(
            ((actual - declared) / declared).abs() <= 0.01,
            "{} 读入 {actual} 行，声明 {declared} 行，超出 1%",
            bucket.file
        );
        let lossy = outcome.quarantined_in(bucket.file);
        assert_eq!(
            lossy, bucket.expected_lossy_rows,
            "{} 的缺字行数不符",
            bucket.file
        );
    }
    let rows: usize = outcome
        .tallies
        .iter()
        .map(|tally| tally.input_records)
        .sum();
    let declared = expected_total_rows() as f64;
    assert!(
        ((rows as f64 - declared) / declared).abs() <= 0.01,
        "全量读入 {rows} 行与清单声明 {declared} 行相差超过 1%"
    );
    assert_eq!(outcome.quarantined.len(), expected_total_lossy_rows());
    assert_eq!(
        rows,
        outcome.emitted() + outcome.quarantined.len() + outcome.duplicates.len(),
        "读入行数必须等于入库 + 隔离 + 判重"
    );
    assert_eq!(
        outcome.excluded_rows(),
        0,
        "完整检出里六个现代分桶应各自被数出行数"
    );
}
