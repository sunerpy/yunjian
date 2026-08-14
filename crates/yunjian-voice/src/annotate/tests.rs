//! 注音层的四档判定、黄金破读与两条禁令。
//!
//! 本文件里的**期望值分两类，不能混**：
//!
//! - 三个黄金读音（`xiá` / `cuī` / `jì`）与「行」的两个候选（`xíng` / `háng`）是**设计文档
//!   点名的规格**，所以写成字面量；它们变了就该判红。
//! - 上游 `pinyin` 给某个字的候选**全集**不是规格，会随上游数据变，所以一律断言结构性质
//!   （档位、是否包含某个规格读音、候选个数关系），不抄全集。
//!
//! 「骑」这个例子承担了一条别的用例证明不了的事：它的通用候选**只有一个** `qí`，而破读
//! 词表要它在「一骑」里读 `jì`。于是「有据破读压过通用候选」这件事在它身上是可证伪的——
//! 换成「斜」就不行，因为 `xiá` 本来就在「斜」的候选里，断言 `xiá` 不能区分「读了词表」
//! 和「碰巧撞上候选」。

use super::*;
use crate::lexicon::Poyin;

/// 《山行》。破读语境「石径斜」在第一行。
const SHAN_XING: &str = "远上寒山石径斜，白云生处有人家。\n停车坐爱枫林晚，霜叶红于二月花。";

/// 《回乡偶书二首·其二》。破读语境「鬓毛衰」在第一行。
const HUI_XIANG: &str = "少小离家老大回，乡音难改鬓毛衰。\n儿童相见不相识，笑问客从何处来。";

/// 《过华清宫绝句三首·其一》。破读语境「一骑」在第二行。
const HUA_QING: &str = "长安回望绣成堆，山顶千门次第开。\n一骑红尘妃子笑，无人知是荔枝来。";

/// 一个在基本区之内、而上游没有读音数据的汉字。
///
/// 它是实测挑出来的，不是猜的：基本区 20992 个码位里有 68 个没有读音，这是第一个。
const NO_READING: char = '兙';

fn shipped() -> Poyin {
    Poyin::shipped().expect("随包破读词表可解析")
}

fn line_of(body: &str, index: usize) -> &str {
    body.split('\n')
        .filter(|line| !line.trim().is_empty())
        .nth(index)
        .expect("该行存在")
}

fn reading_of(poyin: &Poyin, character: char, line: &str) -> Reading {
    resolve(poyin, character, line).expect("内容字有读音层")
}

#[test]
fn the_three_golden_readings_are_attested_in_their_own_lines() {
    let poyin = shipped();
    let cases = [
        ('斜', SHAN_XING, 0, "xiá", AttestedConfidence::RhymeAttested),
        ('衰', HUI_XIANG, 0, "cuī", AttestedConfidence::RhymeAttested),
        ('骑', HUA_QING, 1, "jì", AttestedConfidence::ToneSplit),
    ];

    for (character, body, index, expected, expected_confidence) in cases {
        let line = line_of(body, index);
        match reading_of(&poyin, character, line) {
            Reading::Attested {
                pinyin,
                confidence,
                evidence,
            } => {
                assert_eq!(pinyin, expected, "{character} 在「{line}」里的带调拼音");
                assert_eq!(confidence, expected_confidence, "{character} 的依据强度");
                assert!(
                    !evidence.trim().is_empty(),
                    "{character} 的依据不能是空而存在的引用"
                );
                assert!(
                    evidence.contains('部') || evidence.contains('卷'),
                    "{character} 的依据要带得住定位符，实际是 {evidence:?}"
                );
            }
            other => panic!("{character} 在「{line}」里应是有据破读，实际是 {other:?}"),
        }
    }
}

#[test]
fn an_override_beats_a_sole_generic_candidate() {
    let poyin = shipped();
    let line = line_of(HUA_QING, 1);

    // 先把「不查词表」那条路上的结果拿到手，再证明查了词表的结果与它不同。
    // 这样这条用例就不依赖任何写死的候选全集，却仍然能区分两条路径。
    let generic = generic_reading('骑');
    let Reading::Generic { pinyin: alone } = generic.clone() else {
        panic!("「骑」的通用候选应恰好一个，实际是 {generic:?}；本用例的前提不再成立");
    };

    match reading_of(&poyin, '骑', line) {
        Reading::Attested { pinyin, .. } => assert_ne!(
            pinyin, alone,
            "有据破读必须压过唯一的通用候选，否则读了词表和没读看不出区别"
        ),
        other => panic!("「骑」在「{line}」里应是有据破读，实际是 {other:?}"),
    }
}

#[test]
fn a_context_row_does_not_reach_a_line_without_that_context() {
    let poyin = shipped();
    let elsewhere = "斜阳照墟落";
    assert!(
        !elsewhere.contains("石径斜"),
        "本用例要求这一行不含破读语境"
    );

    let reading = reading_of(&poyin, '斜', elsewhere);
    assert_ne!(
        reading.kind(),
        "attested",
        "语境行不该漏到别的句子，实际得到 {reading:?}"
    );
}

#[test]
fn a_sole_candidate_is_generic_and_not_claimed_as_a_classical_verdict() {
    let poyin = shipped();
    let line = line_of(SHAN_XING, 1);

    match reading_of(&poyin, '花', line) {
        Reading::Generic { pinyin } => assert!(!pinyin.is_empty(), "通用拼音不能是空串"),
        other => panic!("「花」只有一个候选，应是通用拼音，实际是 {other:?}"),
    }
}

#[test]
fn multiple_candidates_without_evidence_stay_uncertain() {
    let poyin = shipped();
    let line = "行到水穷处";

    match reading_of(&poyin, '行', line) {
        Reading::Uncertain { candidates } => {
            assert!(
                candidates.len() > 1,
                "存疑档必须并列多个候选，实际是 {candidates:?}"
            );
            for wanted in ["xíng", "háng"] {
                assert!(
                    candidates.iter().any(|item| item == wanted),
                    "存疑候选应含设计点名的 {wanted}，实际是 {candidates:?}"
                );
            }
        }
        other => panic!("「行」无破读证据且多候选，应存疑，实际是 {other:?}"),
    }
}

#[test]
fn a_character_without_reading_data_gets_no_invented_pinyin() {
    let poyin = shipped();
    assert!(
        is_content_character(NO_READING),
        "本用例要求它是内容字，否则验的是标点那条路"
    );
    assert_eq!(
        reading_of(&poyin, NO_READING, "兙"),
        Reading::Absent,
        "没有读音数据时只能是暂无注音，不能造一个占位读音"
    );
}

#[test]
fn an_engine_default_row_is_not_promoted_to_an_attested_reading() {
    let poyin = shipped();

    // 这两个字在词表里都有登记处置行（明确表示不覆写），且候选数一多一少，
    // 于是「没被提升」这件事在两条不同的下游分支上都被验到。
    for (character, expected_kind) in [('中', "uncertain"), ('乡', "generic")] {
        let line = format!("{character}");
        let reading = reading_of(&poyin, character, &line);
        assert_ne!(
            reading.kind(),
            "attested",
            "{character} 只有登记处置行，不得成为有据读音，实际是 {reading:?}"
        );
        assert_eq!(
            reading.kind(),
            expected_kind,
            "{character} 应降级到通用候选那几档，实际是 {reading:?}"
        );
    }
}

#[test]
fn every_non_overriding_row_in_the_shipped_table_fails_the_attested_conversion() {
    let poyin = shipped();
    let mut checked = 0usize;

    for row in poyin.rows() {
        if row.confidence.overrides() {
            assert!(
                AttestedConfidence::try_from(row.confidence).is_ok(),
                "覆写行的依据强度应能转成有据档，{:?} 却不能",
                row.confidence
            );
        } else {
            assert_eq!(
                AttestedConfidence::try_from(row.confidence),
                Err(NotAttested),
                "不覆写的行在类型上不能有有据档，{:?} 却过了",
                row.confidence
            );
            checked += 1;
        }
    }

    assert!(
        checked > 0,
        "随包词表里应存在不覆写的登记行，否则这条断言是空的"
    );
}

#[test]
fn punctuation_has_no_reading_layer_at_all() {
    let poyin = shipped();
    for character in ['，', '。', '、', '　'] {
        assert_eq!(
            resolve(&poyin, character, "远上寒山石径斜，"),
            None,
            "{character:?} 不是内容字，应当连读音层都没有；给它标暂无注音是噪声"
        );
    }
}

#[test]
fn coverage_counts_every_content_character_exactly_once() {
    let poyin = shipped();
    let annotation = annotate_poem(&poyin, SHAN_XING);

    let content = SHAN_XING
        .chars()
        .filter(|&character| is_content_character(character))
        .count();
    assert_eq!(
        annotation.coverage.total(),
        content,
        "四档之和必须等于内容字总数，否则公布的覆盖没有分母"
    );

    let cells: usize = annotation.lines.iter().map(|line| line.cells.len()).sum();
    let non_blank: usize = SHAN_XING
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().count())
        .sum();
    assert_eq!(cells, non_blank, "逐格要覆盖每一个字符，界面才能逐格对齐");
}

#[test]
fn the_golden_poem_has_at_least_one_of_each_reachable_tier() {
    let poyin = shipped();
    let annotation = annotate_poem(&poyin, SHAN_XING);

    assert!(
        annotation.coverage.attested >= 1,
        "《山行》含破读语境，有据档应非空：{:?}",
        annotation.coverage
    );
    assert!(
        annotation.coverage.generic >= 1,
        "应有单候选通用拼音：{:?}",
        annotation.coverage
    );
    assert!(
        annotation.coverage.uncertain >= 1,
        "应有多候选存疑：{:?}",
        annotation.coverage
    );
}

#[test]
fn line_index_counts_only_non_blank_lines() {
    let poyin = shipped();
    let with_blanks = format!("\n{SHAN_XING}\n\n");
    let annotation = annotate_poem(&poyin, &with_blanks);

    let indexes: Vec<usize> = annotation
        .lines
        .iter()
        .map(|line| line.line_index)
        .collect();
    assert_eq!(
        indexes,
        vec![0, 1],
        "行号要与界面过滤空行后的下标同一口径，否则注音会整行错位"
    );
}

#[test]
fn the_wire_tag_agrees_with_the_kind_label() {
    let cases = [
        Reading::Attested {
            pinyin: "xiá".to_owned(),
            confidence: AttestedConfidence::RhymeAttested,
            evidence: "《平水韵》下平声部 六麻".to_owned(),
        },
        Reading::Generic {
            pinyin: "huā".to_owned(),
        },
        Reading::Uncertain {
            candidates: vec!["xíng".to_owned(), "háng".to_owned()],
        },
        Reading::Absent,
    ];

    // 界面按序列化出来的那个标签分流，而覆盖统计与本文件的断言走 `kind()`。
    // 两者是两处独立写的字符串，drift 之后 Rust 侧照旧全绿而界面全档位落到 default 分支。
    for reading in cases {
        let wire = serde_json::to_value(&reading).expect("读音可序列化");
        assert_eq!(
            wire["kind"],
            reading.kind(),
            "{reading:?} 的线上标签与 kind() 不一致"
        );
    }

    for confidence in [
        AttestedConfidence::RhymeAttested,
        AttestedConfidence::ToneSplit,
    ] {
        let wire = serde_json::to_value(confidence).expect("依据强度可序列化");
        assert_eq!(
            wire,
            confidence.as_str(),
            "{confidence:?} 的线上取值与 as_str() 不一致"
        );
    }
}

/// 去掉注释之后的模块正文。
///
/// 禁令类断言必须扫正文而不是整份源码：本文件为了说明禁令，必然要写出被禁的那几个列名，
/// 而被扫的那份源码若把解释写在注释里就会自撞。分离扫描对象与解释文字是本仓库反复踩过
/// 的那条坑的通行修法。
fn annotate_source_without_comments() -> String {
    let source = include_str!("../annotate.rs");
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_annotation_layer_never_touches_the_rhyme_book_tables() {
    let body = annotate_source_without_comments();

    for column in ["rhyme_book", "rhyme_group", "tone_raw"] {
        assert!(
            !body.contains(column),
            "注音层引用了韵书表的 {column} 列。韵书记录声部与韵部，推不出现代拼音，\
             从它反推读音就是拿一个猜测冒充考据"
        );
    }
}

#[test]
fn the_annotation_layer_cannot_reach_a_database_or_the_network() {
    let body = annotate_source_without_comments();

    for forbidden in ["CorpusHandle", "rusqlite", "Connection", "SELECT", "ureq"] {
        assert!(
            !body.contains(forbidden),
            "注音层出现了 {forbidden}。整首批量解析必须够不到可查的东西，\
             这样「切换开关不触发逐字查询」才是结构性的，而不是靠调用点自觉"
        );
    }
}
