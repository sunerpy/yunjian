use super::*;
use crate::model::{
    Genre, LicenseClass, Provenance, ProvenanceKind, RecordInput, RegistryState, Script,
    SourceLocator, rebuild_corpus,
};

/// 仓库内集评种子目录。测试直接读它，所以「种子集全体合法」是每次提交都受检的。
fn commentary_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/commentary")
}

fn load() -> Vec<CommentarySeed> {
    load_seeds(commentary_dir()).expect("读取集评种子集")
}

fn citation(work: &str, dynasty: &str, completed_by: u16, note: &str) -> Citation {
    Citation {
        work: work.to_owned(),
        author: "罗大经".to_owned(),
        dynasty: dynasty.to_owned(),
        work_completed_by: completed_by,
        source_note: note.to_owned(),
    }
}

const HELIN_NOTE: &str = "甲編・卷一；据维基文库《鶴林玉露·甲編·卷一》校录本，修订号 1545733";

fn seed(id: &str, citation: Citation) -> CommentarySeed {
    CommentarySeed {
        id: id.to_owned(),
        poem: PoemRef {
            author: "杜甫".to_owned(),
            title: "登高".to_owned(),
            first_line: "风急天高猿啸哀".to_owned(),
        },
        text: "古詩云：「人生不滿百，常懷千歲憂。」而淵明以五字盡之。".to_owned(),
        citation,
    }
}

fn reject_reason(entry: CommentarySeed) -> RejectionReason {
    validate_seed(&entry).expect_err("该条目应被拒绝").reason
}

#[test]
fn an_entry_without_a_citation_is_rejected() {
    let empty_work = seed("no-work", citation("", "宋", 1252, HELIN_NOTE));
    assert_eq!(
        reject_reason(empty_work),
        RejectionReason::MissingCitationField
    );

    let empty_note = seed("no-note", citation("鹤林玉露", "宋", 1252, ""));
    assert_eq!(
        reject_reason(empty_note),
        RejectionReason::MissingCitationField
    );

    let empty_dynasty = seed("no-dynasty", citation("鹤林玉露", "", 1252, HELIN_NOTE));
    assert_eq!(
        reject_reason(empty_dynasty),
        RejectionReason::MissingCitationField
    );
}

/// 引用 1980 年代出版物必须被前现代规则拒绝——这是「现代赏析进不来」的门。
#[test]
fn an_entry_citing_a_1980s_publication_is_rejected() {
    let modern = seed(
        "modern-1983",
        citation(
            "唐诗鉴赏辞典",
            "清",
            1983,
            "第三册・卷二；据上海辞书出版社 1983 年版",
        ),
    );
    let rejection = validate_seed(&modern).expect_err("1980 年代出版物应被拒绝");
    assert_eq!(rejection.reason, RejectionReason::ModernGenreMarker);
    assert!(
        rejection.detail.contains("鉴赏辞典"),
        "拒绝理由必须点明现代体裁标记：{}",
        rejection.detail
    );

    // 即便书名伪装成前现代，成书年 1983 仍然过不去。
    let disguised = seed(
        "modern-disguised",
        citation("某氏诗话", "清", 1983, "卷一；据某社排印本"),
    );
    let rejection = validate_seed(&disguised).expect_err("成书年 1983 应被拒绝");
    assert_eq!(rejection.reason, RejectionReason::WorkNotPreModern);
    assert!(
        rejection.detail.contains("1912"),
        "拒绝理由必须点明前 1912 判据：{}",
        rejection.detail
    );
}

/// 方案的失败场景：`citation.dynasty` 为 `现代` 必须带明确理由被拒。
#[test]
fn a_modern_dynasty_is_rejected_with_an_explicit_reason() {
    for dynasty in ["现代", "當代", "民国", "1980s"] {
        let entry = seed(
            "modern-dynasty",
            citation("某书", dynasty, 1200, HELIN_NOTE),
        );
        let rejection = validate_seed(&entry).expect_err("现代朝代应被拒绝");
        assert_eq!(
            rejection.reason,
            RejectionReason::DynastyNotPreModern,
            "朝代 {dynasty} 应按非前现代拒绝"
        );
        assert!(
            rejection.detail.contains(dynasty) && rejection.detail.contains("1912"),
            "拒绝理由必须同时点明输入朝代与前 1912 判据：{}",
            rejection.detail
        );
    }
}

/// 仅有非空 `work` 不够：没有卷次/章节定位符的引用无法复核，必须被拒。
#[test]
fn a_citation_without_a_volume_locator_is_rejected() {
    let entry = seed(
        "no-locator",
        citation("鹤林玉露", "宋", 1252, "据维基文库校录本"),
    );
    assert_eq!(
        reject_reason(entry),
        RejectionReason::SourceNoteMissingLocator
    );

    // 「卷帙浩繁」含「卷」字但不是定位符，同样必须被拒。
    let prose = seed(
        "prose-only",
        citation("鹤林玉露", "宋", 1252, "卷帙浩繁；据维基文库校录本"),
    );
    assert_eq!(
        reject_reason(prose),
        RejectionReason::SourceNoteMissingLocator
    );
}

#[test]
fn a_citation_without_an_edition_is_rejected() {
    let entry = seed("no-edition", citation("鹤林玉露", "宋", 1252, "甲編・卷一"));
    assert_eq!(
        reject_reason(entry),
        RejectionReason::SourceNoteMissingEdition
    );
}

#[test]
fn commentary_text_must_look_pre_modern() {
    let mut ascii = seed("ascii", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    ascii.text = "This is a modern English gloss of the poem.".to_owned();
    assert_eq!(reject_reason(ascii), RejectionReason::InvalidText);

    let mut dated = seed("dated", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    dated.text = "此詩作於一九八二年，見於今人所編選本，語意甚明。".to_owned();
    assert_eq!(reject_reason(dated.clone()), RejectionReason::InvalidText);

    let mut short = seed("short", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    short.text = "甚佳。".to_owned();
    assert_eq!(reject_reason(short), RejectionReason::InvalidText);
}

/// 正确引用《鹤林玉露》的条目必须被接受，并链到正确的 `stable_id`。
#[test]
fn a_correctly_cited_helin_yulu_entry_is_accepted_and_linked() {
    let entry = seed("helin-ok", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    let accepted = validate_seed(&entry).expect("正确引用的鹤林玉露条目应被接受");
    assert_eq!(accepted.work, "鹤林玉露");
    assert_eq!(accepted.dynasty, Dynasty::Song);
    assert_eq!(accepted.dynasty_raw, "宋");
    assert!(accepted.work_completed_by < PRE_MODERN_YEAR_EXCLUSIVE);

    let records = poem_fixture();
    let expected = records
        .iter()
        .find(|record| record.title == "登高")
        .map(|record| record.stable_id.clone())
        .expect("fixture 应含《登高》");

    let outcome = ingest(&[entry], &records).expect("入库应成功");
    assert!(
        outcome.rejections.is_empty(),
        "不应有被拒条目：{:?}",
        outcome.rejections
    );
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].poem_id, expected);
    assert_eq!(
        outcome.records[0].citation.work, "鹤林玉露",
        "出处必须随记录一同保留"
    );
}

#[test]
fn an_unresolvable_poem_reference_is_rejected_rather_than_guessed() {
    let mut entry = seed("unresolved", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    entry.poem.title = "并不存在的题目".to_owned();
    let outcome = ingest(&[entry], &poem_fixture()).expect("入库本身不应报错");
    assert!(outcome.records.is_empty());
    assert_eq!(outcome.rejections.len(), 1);
    assert_eq!(
        outcome.rejections[0].reason,
        RejectionReason::PoemUnresolved
    );
}

#[test]
fn a_wrong_first_line_does_not_silently_link_to_a_same_title_poem() {
    let mut entry = seed(
        "wrong-first-line",
        citation("鹤林玉露", "宋", 1252, HELIN_NOTE),
    );
    entry.poem.first_line = "并非此诗的首句".to_owned();
    let outcome = ingest(&[entry], &poem_fixture()).expect("入库本身不应报错");
    assert!(
        outcome.records.is_empty(),
        "首句不符时绝不能挂到同题的另一首诗上"
    );
    assert_eq!(
        outcome.rejections[0].reason,
        RejectionReason::PoemUnresolved
    );
}

#[test]
fn duplicate_entry_ids_are_rejected() {
    let entry = seed("dup", citation("鹤林玉露", "宋", 1252, HELIN_NOTE));
    let outcome = validate_all(&[entry.clone(), entry]);
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(
        outcome.rejections[0].reason,
        RejectionReason::DuplicateEntryId
    );
}

/// 种子集全体必须通过校验。这条断言让「出处必填且可定位」成为提交门禁。
#[test]
fn every_shipped_entry_passes_validation() {
    let seeds = load();
    validate_all(&seeds)
        .require_all_accepted()
        .expect("种子集不应有被拒条目");
}

#[test]
fn the_seed_corpus_holds_at_least_one_hundred_entries() {
    assert!(
        load().len() >= 100,
        "方案要求至少 100 条已核实条目，实际 {}",
        load().len()
    );
}

/// 逐条断言 citation 的三个可审计字段，空而存在的 citation 无法通过。
#[test]
fn every_accepted_citation_carries_work_pre_modern_dynasty_and_a_locatable_source_note() {
    let seeds = load();
    let accepted = validate_all(&seeds)
        .require_all_accepted()
        .expect("种子集应全部通过");
    assert_eq!(accepted.len(), seeds.len());

    for record in &accepted {
        let citation = &record.citation;
        assert!(!citation.work.trim().is_empty(), "{} 缺 work", record.id);
        assert!(
            !citation.author.trim().is_empty(),
            "{} 缺 citation.author",
            record.id
        );
        assert!(
            Dynasty::ALL.contains(&citation.dynasty),
            "{} 的朝代不在十五个前 1912 规范键内",
            record.id
        );
        assert!(
            citation.work_completed_by < PRE_MODERN_YEAR_EXCLUSIVE,
            "{} 的成书上界 {} 不早于 {PRE_MODERN_YEAR_EXCLUSIVE}",
            record.id,
            citation.work_completed_by
        );
        let locator = find_volume_locator(&citation.source_note).unwrap_or_else(|| {
            panic!(
                "{} 的 source_note 没有卷次/章节定位符：{}",
                record.id, citation.source_note
            )
        });
        assert!(!locator.trim().is_empty(), "{} 的定位符为空", record.id);
        assert!(
            has_edition_marker(&citation.source_note),
            "{} 的 source_note 未说明所据版本：{}",
            record.id,
            citation.source_note
        );
    }
}

/// 每条 `source_note` 都必须带可复核的修订号——这是「可审计」的落地形态。
#[test]
fn every_source_note_pins_a_revision_for_re_verification() {
    for seed in load() {
        assert!(
            seed.citation.source_note.contains("修订号"),
            "{} 的 source_note 未固定修订号：{}",
            seed.id,
            seed.citation.source_note
        );
    }
}

#[test]
fn the_index_is_a_generated_artifact_that_does_not_drift() {
    let count = require_index_matches(commentary_dir()).expect("索引应与 sources/ 一致");
    assert_eq!(count, load().len());
}

#[test]
fn a_tampered_index_is_detected() {
    let dir = temp_commentary_dir("tampered");
    let index_path = dir.join(INDEX_FILE);
    let mut rendered = std::fs::read_to_string(&index_path).expect("读取索引");
    rendered = rendered.replacen("鹤林玉露", "某氏诗话", 1);
    std::fs::write(&index_path, rendered).expect("写回被改的索引");
    let error = require_index_matches(&dir).expect_err("索引漂移必须被发现");
    assert!(
        format!("{error}").contains("索引是生成物"),
        "错误信息应说明索引是生成物：{error}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 集评永远只能引用前现代著作，所以引用侧的朝代键必须止于清。
#[test]
fn the_dynasty_vocabulary_ends_before_1912() {
    assert_eq!(Dynasty::ALL.len(), 15);
    assert_eq!(Dynasty::ALL.last().copied(), Some(Dynasty::Qing));
    for raw in ["现代", "当代", "民国", "共和国"] {
        assert!(
            Dynasty::canonicalize(raw).is_err(),
            "{raw} 不应被归一为前现代朝代"
        );
    }
}

fn temp_commentary_dir(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("yunjian-commentary-{label}-{}", std::process::id()));
    std::fs::create_dir_all(path.join(SOURCES_DIR)).expect("创建临时目录");
    let source = commentary_dir();
    for entry in std::fs::read_dir(source.join(SOURCES_DIR)).expect("枚举种子目录") {
        let entry = entry.expect("读取目录项");
        std::fs::copy(entry.path(), path.join(SOURCES_DIR).join(entry.file_name()))
            .expect("复制种子文件");
    }
    std::fs::copy(source.join(INDEX_FILE), path.join(INDEX_FILE)).expect("复制索引");
    path
}

/// 最小诗篇 fixture。走 `rebuild_corpus` 铸造真实 `stable_id`，不手写 ID。
fn poem_fixture() -> Vec<CanonicalRecord> {
    let inputs = vec![
        poem_input(
            "fixture-denggao",
            "登高",
            "杜甫",
            Dynasty::Tang,
            "唐",
            vec![
                "風急天高猿嘯哀，渚清沙白鳥飛迴。".to_owned(),
                "無邊落木蕭蕭下，不盡長江滾滾來。".to_owned(),
            ],
        ),
        // 同作者同题的另一首：首句不同，用来证明首句消歧真的在起作用。
        poem_input(
            "fixture-denggao-other",
            "登高",
            "杜甫",
            Dynasty::Tang,
            "唐",
            vec!["他年登高處，未必似今朝。".to_owned()],
        ),
        poem_input(
            "fixture-chunwang",
            "春望",
            "杜甫",
            Dynasty::Tang,
            "唐",
            vec!["國破山河在，城春草木深。".to_owned()],
        ),
    ];
    rebuild_corpus(&RegistryState::default(), &[], inputs, "0.1.0-test", &[])
        .expect("fixture 重建应成功")
        .shippable_records
}

fn poem_input(
    native_id: &str,
    title: &str,
    author: &str,
    dynasty: Dynasty,
    dynasty_raw: &str,
    body_lines: Vec<String>,
) -> RecordInput {
    let body_original = body_lines.join("\n");
    RecordInput {
        source_locator: SourceLocator::native("chinese-poetry", native_id),
        genre: Genre::Shi,
        title: title.to_owned(),
        title_raw: title.to_owned(),
        author: author.to_owned(),
        dynasty,
        dynasty_raw: dynasty_raw.to_owned(),
        body_lines,
        body_original,
        script: Script::Traditional,
        provenance: Provenance {
            source_name: "chinese-poetry".to_owned(),
            source_rev: "b8594f81a89752241442f2ce267d6f66f96704ee".to_owned(),
            license: "MIT".to_owned(),
            license_class: LicenseClass::Permissive,
            kind: ProvenanceKind::Original,
        },
    }
}

/// 全量解析：需要锁定 revision 的完整上游检出，`make ci` 不跑。
///
/// 断言每一条种子都能在真实语料里链到唯一 `stable_id`。这条是「逐条出处」
/// 之外的另一半——出处对了但挂错诗，集评一样是错的。
#[test]
#[ignore = "需要锁定 revision 的完整上游检出，由 YUNJIAN_CHINESE_POETRY_DIR 指定"]
fn every_seed_entry_resolves_against_the_real_corpus() {
    let Ok(root) = std::env::var("YUNJIAN_CHINESE_POETRY_DIR") else {
        panic!("请设置 YUNJIAN_CHINESE_POETRY_DIR 指向锁定 revision 的检出");
    };
    let outcome = crate::ingest::chinese_poetry::ingest(&root).expect("上游入库应成功");
    let rebuilt = rebuild_corpus(
        &RegistryState::default(),
        &[],
        outcome.records,
        "0.1.0-test",
        &[],
    )
    .expect("重建应成功");

    let seeds = load();
    let result = ingest(&seeds, &rebuilt.shippable_records).expect("集评入库应成功");
    assert!(
        result.rejections.is_empty(),
        "有 {} 条种子无法解析：{:?}",
        result.rejections.len(),
        result.rejections.iter().take(10).collect::<Vec<_>>()
    );
    assert_eq!(result.records.len(), seeds.len());
    for record in &result.records {
        assert_eq!(
            record.poem_id.len(),
            16,
            "{} 的 poem_id 应是 16 位 stable_id",
            record.id
        );
    }
}
