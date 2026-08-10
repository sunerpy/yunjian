//! [`super`] 的测试。
//!
//! 语料 fixture 里只有两类正文：真实的公有领域诗句，以及标注为「字形樣本」的
//! 单字/短句——后者刻意不冒充某人的作品，因为需要的是一个特定字形，不是一首诗。

use super::*;
use crate::model::{
    Dynasty, Genre, LicenseClass, Provenance, ProvenanceKind, SourceLocatorKind,
    compute_content_hash, compute_edition_group, compute_work_group,
};

/// 由正文构造一条规范记录。`stable_id` 直接取自可读的短名，便于断言定位。
fn record(
    stable_id: &str,
    title: &str,
    author: &str,
    body: &str,
    script: Script,
) -> CanonicalRecord {
    let body_lines: Vec<String> = body.split('\n').map(str::to_owned).collect();
    let joined = body_lines.join("\n");
    CanonicalRecord {
        stable_id: stable_id.to_owned(),
        content_hash: compute_content_hash(author, Dynasty::Tang, title, &joined),
        work_group: compute_work_group(&joined),
        edition_group: compute_edition_group(author, &joined),
        source_locator: format!("fixture:{stable_id}"),
        source_locator_kind: SourceLocatorKind::Native,
        genre: Genre::Shi,
        title: title.to_owned(),
        title_raw: title.to_owned(),
        ci_tune: None,
        author: author.to_owned(),
        dynasty: Dynasty::Tang,
        dynasty_raw: "唐".to_owned(),
        body_lines,
        body_original: joined,
        script,
        provenance: Provenance {
            source_name: "fixture".to_owned(),
            source_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            license: "MIT".to_owned(),
            license_class: LicenseClass::PublicDomain,
            kind: ProvenanceKind::Original,
        },
    }
}

/// 杜甫《春望》首联，全唐诗的繁体字形。
const CHUNWANG_TRADITIONAL: &str = "國破山河在，城春草木深";

/// 《國語·晉語》语句，用来把「醫」这个字形放进语料。
const YI_SAMPLE: &str = "上醫醫國，其次疾人";

fn base_corpus() -> Vec<CanonicalRecord> {
    vec![
        record(
            "chunwang",
            "春望",
            "杜甫",
            CHUNWANG_TRADITIONAL,
            Script::Traditional,
        ),
        record("yi", "字形樣本", "測試", YI_SAMPLE, Script::Traditional),
    ]
}

fn normalized<'a>(outcome: &'a NormalizeOutcome, stable_id: &str) -> &'a NormalizedRecord {
    outcome
        .records
        .iter()
        .find(|record| record.stable_id == stable_id)
        .expect("fixture 记录应当存在")
}

#[test]
fn traditional_body_becomes_simplified_and_original_is_preserved_byte_for_byte() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&base_corpus())?;
    let chunwang = normalized(&outcome, "chunwang");
    assert_eq!(chunwang.body, "国破山河在，城春草木深");
    assert_eq!(
        chunwang.body_original.as_bytes(),
        CHUNWANG_TRADITIONAL.as_bytes(),
        "body_original 必须与输入逐字节相同"
    );
    assert_eq!(chunwang.script, Script::Traditional);
    Ok(())
}

#[test]
fn the_plan_sample_line_is_simplified_while_its_original_survives() -> Result<()> {
    // 计划里点名的那一行：`全唐诗` 的繁体首联。
    let source = "秦川雄帝宅，函谷壯皇居";
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&[record(
        "diwang",
        "帝京篇",
        "李世民",
        source,
        Script::Traditional,
    )])?;
    let poem = normalized(&outcome, "diwang");
    assert_eq!(poem.body, "秦川雄帝宅，函谷壮皇居");
    assert_ne!(poem.body, poem.body_original);
    assert_eq!(poem.body_original.as_bytes(), source.as_bytes());
    Ok(())
}

#[test]
fn already_simplified_record_is_untouched() -> Result<()> {
    let source = "气和玉烛，睿化著鸿明";
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&[record(
        "songci",
        "字形樣本",
        "測試",
        source,
        Script::Simplified,
    )])?;
    let poem = normalized(&outcome, "songci");
    assert_eq!(poem.body, source, "已是简体的记录不得被改动");
    assert_eq!(poem.body, poem.body_original);
    Ok(())
}

#[test]
fn mixed_script_record_is_fully_normalized() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&[record(
        "mixed",
        "字形樣本",
        "測試",
        "國破山河在，国破山河在",
        Script::Mixed,
    )])?;
    let poem = normalized(&outcome, "mixed");
    assert_eq!(poem.body, "国破山河在，国破山河在");
    Ok(())
}

#[test]
fn line_structure_survives_normalization() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&[record(
        "multiline",
        "春望",
        "杜甫",
        "國破山河在\n城春草木深",
        Script::Traditional,
    )])?;
    let poem = normalized(&outcome, "multiline");
    assert_eq!(poem.body_lines, vec!["国破山河在", "城春草木深"]);
    assert_eq!(poem.body, poem.body_lines.join("\n"));
    assert_eq!(poem.body_original.lines().count(), poem.body_lines.len());
    Ok(())
}

#[test]
fn per_line_conversion_equals_whole_body_conversion() -> Result<()> {
    // 逐行转换与整段转换必须一致，否则 `body` 与 `body_lines` 会各说各话。
    // 成立的原因是换行不参与任何短语规则。
    let normalizer = Normalizer::new()?;
    let whole = "國破山河在\n城春草木深\n感時花濺淚";
    let per_line: Vec<String> = whole
        .split('\n')
        .map(|line| normalizer.canonicalize(line))
        .collect();
    assert_eq!(normalizer.canonicalize(whole), per_line.join("\n"));
    Ok(())
}

#[test]
fn non_round_trip_stable_record_is_flagged_conversion_unstable() -> Result<()> {
    // 「昵」：`t2s` 不动它，而 `s2t` 把它推成繁体的「暱」，再转回来仍是「暱」。
    // 也就是说 OpenCC 自己的两本字典对这个字形并不自洽，属于必须复核而不是静默
    // 接受的情形。
    let normalizer = Normalizer::new()?;
    assert_eq!(normalizer.simplify("昵"), "昵");
    let mut corpus = base_corpus();
    corpus.push(record("ni", "字形樣本", "測試", "昵", Script::Simplified));
    let outcome = normalizer.normalize(&corpus)?;

    assert_eq!(
        outcome.finding_count(NormalizationReason::ConversionUnstable),
        1
    );
    let finding = &outcome.findings[0];
    assert_eq!(finding.stable_id, "ni");
    assert_eq!(finding.reason.as_reason_code(), "conversion_unstable");
    assert!(
        finding.detail.contains("往返"),
        "detail 应指出是往返不稳定：{}",
        finding.detail
    );
    assert!(
        outcome.records.iter().any(|entry| entry.stable_id == "ni"),
        "conversion_unstable 是告警而非排除处置，记录仍须入库"
    );
    Ok(())
}

#[test]
fn stable_records_carry_no_findings() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&base_corpus())?;
    assert!(
        outcome.findings.is_empty(),
        "往返稳定的记录不应被标记：{:?}",
        outcome.findings
    );
    Ok(())
}

#[test]
fn variant_map_contains_the_guo_and_yi_rows() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&base_corpus())?;
    assert_eq!(outcome.variant_map.get('國'), Some('国'));
    assert_eq!(outcome.variant_map.get('醫'), Some('医'));
    let rows = outcome.variant_map.rows();
    assert!(rows.contains(&VariantRow {
        src_char: '國',
        dst_char: '国',
    }));
    assert!(rows.contains(&VariantRow {
        src_char: '醫',
        dst_char: '医',
    }));
    Ok(())
}

#[test]
fn table_driven_rewrite_turns_traditional_query_into_the_indexed_form() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&base_corpus())?;
    assert_eq!(outcome.variant_map.rewrite("國破山河在"), "国破山河在");
    assert!(
        normalized(&outcome, "chunwang")
            .body
            .starts_with(&outcome.variant_map.rewrite("國破山河在")),
        "改写结果必须逐字出现在被索引的 body 里"
    );
    Ok(())
}

#[test]
fn dropping_the_guo_row_breaks_the_traditional_rewrite() -> Result<()> {
    // 失败场景做成常驻测试：把 `國` 那一行删掉之后，繁体改写必须坏掉。若它照样
    // 通过，说明命中来自某个隐藏的运行时转换依赖，而不是这张表。
    let normalizer = Normalizer::new()?;
    let mut outcome = normalizer.normalize(&base_corpus())?;
    assert_eq!(outcome.variant_map.rewrite("國破山河在"), "国破山河在");

    assert_eq!(outcome.variant_map.remove('國'), Some('国'));
    assert_eq!(outcome.variant_map.get('國'), None);
    assert_ne!(
        outcome.variant_map.rewrite("國破山河在"),
        "国破山河在",
        "删掉 國 行后繁体改写仍然成功，说明命中不是这张表提供的"
    );
    assert_eq!(outcome.variant_map.rewrite("國破山河在"), "國破山河在");
    Ok(())
}

#[test]
fn variant_map_holds_only_mappings_present_in_the_corpus() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&base_corpus())?;
    assert_eq!(
        outcome.variant_map.get('龍'),
        None,
        "语料里没有「龙」，就不该有 龍 -> 龙 这一行"
    );
    assert!(
        outcome.variant_map.len() < normalizer.candidate_count(),
        "表必须被语料裁剪过，而不是把整本字典倒进去"
    );
    assert_eq!(outcome.stats.rows, outcome.variant_map.len());
    assert_eq!(
        outcome.stats.candidates,
        outcome.stats.rows
            + outcome.stats.dropped_target_absent
            + outcome.stats.dropped_source_survives,
        "候选必须被完整分账：入表、目标缺席、源字存活三者之和"
    );
    assert!(
        outcome.stats.dropped_target_absent > 0,
        "两条 fixture 只覆盖极少数字，绝大多数候选应当因目标缺席被丢弃"
    );
    Ok(())
}

#[test]
fn a_glyph_that_survives_normalization_is_never_rewritten() -> Result<()> {
    // 「乾」是短语感知的直接后果：单字转成「干」，而「乾坤」整体不变。于是它同时
    // 出现在索引里（乾坤）与被转换的位置上（干戈）。这时把 乾 -> 干 放进改写表，
    // 会让照抄语料原文的「乾坤」查询一条都搜不到。
    let normalizer = Normalizer::new()?;
    assert_eq!(normalizer.canonicalize("乾坤日夜浮"), "乾坤日夜浮");
    assert_eq!(normalizer.canonicalize("乾"), "干");

    let corpus = vec![
        record(
            "qiankun",
            "登岳陽樓",
            "杜甫",
            "乾坤日夜浮",
            Script::Traditional,
        ),
        record(
            "ganggo",
            "過零丁洋",
            "文天祥",
            "干戈寥落四周星",
            Script::Simplified,
        ),
    ];
    let outcome = normalizer.normalize(&corpus)?;
    assert!(outcome.variant_map.get('乾').is_none());
    assert_eq!(outcome.variant_map.rewrite("乾坤日夜浮"), "乾坤日夜浮");
    assert!(outcome.stats.dropped_source_survives > 0);
    Ok(())
}

#[test]
fn variant_map_rows_are_deterministically_ordered() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let first = normalizer.normalize(&base_corpus())?.variant_map.rows();
    let mut reordered = base_corpus();
    reordered.reverse();
    let second = normalizer.normalize(&reordered)?.variant_map.rows();
    assert_eq!(first, second, "构建产物要求逐字节可复现");
    assert!(
        first
            .windows(2)
            .all(|pair| pair[0].src_char < pair[1].src_char),
        "行必须按 src_char 升序"
    );
    Ok(())
}

#[test]
fn empty_corpus_yields_an_empty_variant_map() -> Result<()> {
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&[])?;
    assert!(outcome.variant_map.is_empty());
    assert_eq!(outcome.stats.rows, 0);
    assert_eq!(outcome.stats.dropped_source_survives, 0);
    Ok(())
}

#[test]
fn residual_variants_close_the_documented_yu_gap() -> Result<()> {
    // `crates/yunjian-corpus/src/ingest/werneror/tests.rs` 记录了这个缺口：上游同时
    // 写「送寒馀雪尽」与「有余香」，而 `t2s` 不动「馀」。往返推导能从同一套字典里
    // 把这条映射推出来，不需要人工数据。
    let normalizer = Normalizer::new()?;
    assert_eq!(normalizer.simplify("馀"), "馀", "t2s 确实不动「馀」");
    assert_eq!(normalizer.canonicalize("馀"), "余");
    assert!(normalizer.residual_variants().contains(&VariantRow {
        src_char: '馀',
        dst_char: '余',
    }));
    Ok(())
}

#[test]
fn the_residual_table_never_maps_toward_a_traditional_glyph() -> Result<()> {
    // 往返推导若不加过滤会收进 `昵 -> 暱`：`t2s` 不认识「暱」，于是往返把简体推成
    // 了繁体。过滤条件是「目标字自己还有一个不同的繁体对应」。
    let normalizer = Normalizer::new()?;
    assert!(
        !normalizer
            .residual_variants()
            .iter()
            .any(|row| row.src_char == '昵'),
        "昵 -> 暱 是反向映射，不得进表"
    );
    for row in normalizer.residual_variants() {
        assert_eq!(
            normalizer.simplify(&row.dst_char.to_string()),
            row.dst_char.to_string(),
            "残余异体的目标字必须已是 t2s 的不动点：{row:?}"
        );
    }
    Ok(())
}

#[test]
fn variant_supplement_rows_are_genuinely_missing_from_the_dictionaries() -> Result<()> {
    let normalizer = Normalizer::new()?;
    assert!(
        VARIANT_SUPPLEMENT.len() <= 4,
        "补充表是「字典漏了这一个」的补丁，不是第二本字典"
    );
    for (source, target) in VARIANT_SUPPLEMENT {
        let source_text = source.to_string();
        assert_eq!(
            normalizer.simplify(&source_text),
            source_text,
            "{source} 已被 t2s 覆盖，不该出现在补充表里"
        );
        assert_eq!(
            normalizer.simplify(&target.to_string()),
            target.to_string(),
            "{target} 必须已是简体"
        );
        assert_eq!(normalizer.canonicalize(&source_text), target.to_string());
    }
    Ok(())
}

#[test]
fn candidate_table_size_is_pinned_to_the_measured_dictionary() -> Result<()> {
    // 这三个数是对 `ferrous-opencc` 0.4 内嵌字典的实测值，钉住它们是为了让字典变化
    // 变成一次失败的测试而不是一次静默的行为漂移：升级后表大了或小了，必须重新
    // 测量并在此更新，同时复核 `variant_map` 的体积假设。
    let normalizer = Normalizer::new()?;
    assert_eq!(normalizer.candidate_count(), 4137, "字级候选总数");
    assert_eq!(normalizer.residual_variants().len(), 32, "残余异体行数");
    let outcome = normalizer.normalize(&base_corpus())?;
    assert_eq!(outcome.stats.dropped_not_single_char, 0, "一对多候选数");
    Ok(())
}

#[test]
fn reason_code_is_the_string_the_defect_report_uses() {
    assert_eq!(
        NormalizationReason::ConversionUnstable.as_reason_code(),
        "conversion_unstable"
    );
}

/// 黄金查询契约里声明的字形映射。
#[derive(Debug, serde::Deserialize)]
struct ContractFixture {
    #[serde(default)]
    variant: Vec<ContractVariant>,
    #[serde(default)]
    poem: Vec<ContractPoem>,
}

#[derive(Debug, serde::Deserialize)]
struct ContractVariant {
    from: String,
    to: String,
}

#[derive(Debug, serde::Deserialize)]
struct ContractPoem {
    stable_id: String,
    title: String,
    author: String,
    body: String,
}

#[test]
fn the_generated_table_covers_every_variant_the_query_contract_needs() -> Result<()> {
    // 跨 crate 读 `yunjian-core` 的契约 fixture 是刻意的：那份文件声明了运行时必须
    // 能改写哪些字形（todo 24 的 `normalize_query` 依赖它），而这些字形只能由本模块
    // 生成的表提供。若两边脱节，故障会推迟到实现查询时才暴露，而那时错误现象是
    // 「某些繁体查询零命中」，很难回溯到这里。
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../yunjian-core/tests/fixtures/poems.toml"
    );
    let raw = std::fs::read_to_string(path)?;
    let fixture: ContractFixture = toml::from_str(&raw)
        .map_err(|error| crate::ingest::corpus_error(format!("解析契约 fixture 失败：{error}")))?;
    assert!(!fixture.variant.is_empty() && !fixture.poem.is_empty());

    let corpus: Vec<CanonicalRecord> = fixture
        .poem
        .iter()
        .map(|poem| {
            record(
                &poem.stable_id,
                &poem.title,
                &poem.author,
                &poem.body,
                Script::Simplified,
            )
        })
        .collect();
    let normalizer = Normalizer::new()?;
    let outcome = normalizer.normalize(&corpus)?;

    let missing: Vec<String> = fixture
        .variant
        .iter()
        .filter_map(|variant| {
            let source = variant.from.chars().next()?;
            let target = variant.to.chars().next()?;
            (outcome.variant_map.get(source) != Some(target))
                .then(|| format!("{source} -> {target}"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "契约要求的字形映射未被生成：{missing:?}"
    );
    assert_eq!(
        outcome.variant_map.len(),
        155,
        "19 首真实诗词裁剪出的表规模；语料变大时行数只会上升"
    );
    assert_eq!(
        outcome.finding_count(NormalizationReason::ConversionUnstable),
        0,
        "逐字校对过的公有领域正文不应触发不稳定告警，否则这个信号就是噪声"
    );
    Ok(())
}
