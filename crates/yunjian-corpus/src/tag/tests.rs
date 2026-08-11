//! 策展词表与构建期打标的测试。
//!
//! 最关键的一条是 [`the_shipped_vocabulary_reproduces_every_tag_the_golden_contract_declares`]：
//! 它拿签入的词表跑在黄金契约 fixture（`../yunjian-core/tests/fixtures/poems.toml`）上，
//! 断言产出的主题标签与契约声明**逐条相等**。跨 crate 只读一个文件、不引入依赖——
//! 依赖方向是 corpus -> core，而那份 fixture 是契约的一部分，两侧必须对同一份数据
//! 达成同一套结论。没有这条断言，词表就只是一份没人验证过的清单。

use super::*;
use std::path::PathBuf;

const GOLDEN_FIXTURE: &str = "../yunjian-core/tests/fixtures/poems.toml";

#[derive(Debug, Deserialize)]
struct GoldenFixtures {
    #[serde(rename = "poem")]
    poems: Vec<GoldenPoem>,
}

#[derive(Debug, Deserialize)]
struct GoldenPoem {
    stable_id: String,
    title: String,
    author: String,
    body: String,
    tags: Vec<String>,
}

fn golden_fixtures() -> GoldenFixtures {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_FIXTURE);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取黄金 fixture 失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析黄金 fixture 失败 {}：{error}", path.display()))
}

fn facts(poems: &[GoldenPoem]) -> Vec<PoemFacts<'_>> {
    poems
        .iter()
        .map(|poem| PoemFacts {
            stable_id: poem.stable_id.as_str(),
            author: poem.author.as_str(),
            title: poem.title.as_str(),
            body: poem.body.as_str(),
        })
        .collect()
}

fn tags_of(assignment: &TagAssignment, stable_id: &str) -> BTreeSet<String> {
    assignment
        .rows
        .iter()
        .filter(|row| row.poem_id == stable_id)
        .map(|row| row.tag.clone())
        .collect()
}

fn anthology_names(vocabulary: &TagVocabulary) -> BTreeSet<&str> {
    vocabulary
        .tags
        .iter()
        .filter(|tag| tag.kind == TagKind::Anthology)
        .map(|tag| tag.name.as_str())
        .collect()
}

// ---------------------------------------------------------------- 词表自身

#[test]
fn the_shipped_vocabulary_parses_and_declares_the_planned_axes() {
    let vocabulary = TagVocabulary::shipped().expect("签入词表必须可解析且自校验通过");
    let names = vocabulary.names();
    for required in ["思乡", "送别", "边塞", "咏物", "闲适"] {
        assert!(
            names.contains(required),
            "缺少方案点名的主题标签 {required}"
        );
    }
    for season in ["春", "夏", "秋", "冬"] {
        let tag = vocabulary
            .declaration(season)
            .unwrap_or_else(|| panic!("缺少季节标签 {season}"));
        assert_eq!(tag.kind, TagKind::Season, "{season} 的类别应为 season");
    }
    // 这四个选本同时是 `search::text` 静态显赫度的来源，缺一个就让排序读到一个
    // 永远为空的标签，而那不会报错、只会让名篇沉底。
    for anthology in ["唐诗三百首", "宋词三百首", "千家诗", "古诗文名篇"] {
        let tag = vocabulary
            .declaration(anthology)
            .unwrap_or_else(|| panic!("缺少选本标签 {anthology}"));
        assert_eq!(tag.kind, TagKind::Anthology);
        assert!(
            !tag.has_rules(),
            "{anthology} 不得有关键词规则：选本收录不是文本特征"
        );
    }
}

#[test]
fn every_declared_tag_either_has_rules_or_says_why_not() {
    let vocabulary = TagVocabulary::shipped().expect("解析词表");
    for tag in &vocabulary.tags {
        assert!(
            tag.has_rules() || !tag.rule_note.trim().is_empty(),
            "{} 既无规则也无 rule_note",
            tag.name
        );
        assert!(!tag.gloss.trim().is_empty(), "{} 缺 gloss", tag.name);
    }
}

// ------------------------------------------------- 与黄金契约 fixture 的对齐

#[test]
fn the_shipped_vocabulary_reproduces_every_tag_the_golden_contract_declares() {
    let vocabulary = TagVocabulary::shipped().expect("解析词表");
    let normalizer = Normalizer::new().expect("初始化归一器");
    let fixtures = golden_fixtures();
    let assignment = assign_tags_to_facts(&vocabulary, &facts(&fixtures.poems), &normalizer)
        .expect("打标不得失败");
    let anthologies = anthology_names(&vocabulary);

    for poem in &fixtures.poems {
        let declared: BTreeSet<String> = poem.tags.iter().cloned().collect();
        let produced: BTreeSet<String> = tags_of(&assignment, &poem.stable_id)
            .into_iter()
            .filter(|name| !anthologies.contains(name.as_str()))
            .collect();
        assert_eq!(
            produced, declared,
            "{}《{}》的主题标签与契约不符：产出 {:?}，契约声明 {:?}。\
             契约是不可变的，所以要改的是词表",
            poem.author, poem.title, produced, declared
        );
    }
}

#[test]
fn no_produced_tag_falls_outside_the_checked_in_vocabulary() {
    let vocabulary = TagVocabulary::shipped().expect("解析词表");
    let normalizer = Normalizer::new().expect("初始化归一器");
    let fixtures = golden_fixtures();
    let assignment =
        assign_tags_to_facts(&vocabulary, &facts(&fixtures.poems), &normalizer).expect("打标");
    let names = vocabulary.names();
    for row in &assignment.rows {
        assert!(
            names.contains(row.tag.as_str()),
            "产出了词表未声明的标签 {}（{}）",
            row.tag,
            row.poem_id
        );
    }
    assert_eq!(assignment.report.rows, assignment.rows.len());
    let mut sorted = assignment.rows.clone();
    sorted.sort_by(|left, right| (&left.poem_id, &left.tag).cmp(&(&right.poem_id, &right.tag)));
    assert_eq!(assignment.rows, sorted, "产出必须按 (poem_id, tag) 有序");
    let unique: BTreeSet<(&str, &str)> = assignment
        .rows
        .iter()
        .map(|row| (row.poem_id.as_str(), row.tag.as_str()))
        .collect();
    assert_eq!(unique.len(), assignment.rows.len(), "不得有重复行");
}

/// 两种来源都必须是承重的，否则其中一种是装饰。
///
/// 反证的形态是**同一个标签在不同诗上分别走两条路**：`边塞` 完全由规则得来（《出塞》
/// 的评审条目刻意不写它），`写景` 完全由评审名单得来（《绝句》的正文与题目里没有
/// 任何字面依据）。只断言「有规则」「有名单」不构成反证——那两件事从文件里就能看出来，
/// 看不出来的是它们是否真的各自决定了某条结果。
#[test]
fn both_a_keyword_rule_and_the_reviewed_list_decide_real_assignments() {
    let vocabulary = TagVocabulary::shipped().expect("解析词表");
    let normalizer = Normalizer::new().expect("初始化归一器");
    let fixtures = golden_fixtures();
    let assignment =
        assign_tags_to_facts(&vocabulary, &facts(&fixtures.poems), &normalizer).expect("打标");

    let chusai = fixtures
        .poems
        .iter()
        .find(|poem| poem.title == "出塞")
        .expect("fixture 里应有《出塞》");
    assert!(
        tags_of(&assignment, &chusai.stable_id).contains("边塞"),
        "《出塞》必须得到边塞标签"
    );
    let reviewed_chusai = vocabulary
        .reviewed
        .iter()
        .find(|entry| entry.title == "出塞")
        .expect("《出塞》应有评审条目");
    assert!(
        !reviewed_chusai.add.contains(&"边塞".to_owned()),
        "边塞必须由规则得来：评审名单里写了它，这条反证就失效了"
    );

    let jueju = fixtures
        .poems
        .iter()
        .find(|poem| poem.title == "绝句")
        .expect("fixture 里应有《绝句》");
    assert!(
        tags_of(&assignment, &jueju.stable_id).contains("写景"),
        "《绝句》必须得到写景标签"
    );
    let xiejing = vocabulary.declaration("写景").expect("写景应已声明");
    assert!(
        !xiejing.has_rules(),
        "写景必须由评审名单得来：给它编了关键词，这条反证就失效了"
    );
}

#[test]
fn keyword_matching_folds_traditional_text_so_rules_do_not_silently_miss() {
    // `全唐诗` 的题目与作者都是繁体。不折叠就会让规则在语料的一大半上一条都不命中，
    // 而这种失效不报错——只是标签为空。
    let vocabulary = TagVocabulary::shipped().expect("解析词表");
    let normalizer = Normalizer::new().expect("初始化归一器");
    let traditional = [PoemFacts {
        stable_id: "fixture:traditional",
        author: "李白",
        title: "早發白帝城",
        body: "朝辭白帝彩雲間，千里江陵一日還。",
    }];
    let assignment =
        assign_tags_to_facts(&vocabulary, &traditional, &normalizer).expect("繁体记录打标");
    let tags = tags_of(&assignment, "fixture:traditional");
    assert!(
        tags.contains("行旅"),
        "繁体题目「早發白帝城」折叠后应命中行旅规则，实际得到 {tags:?}"
    );
    assert!(
        tags.contains("唐诗三百首"),
        "繁体作者与题目折叠后应命中评审名单，实际得到 {tags:?}"
    );
}

// ---------------------------------------------------------------- 校验的牙齿

fn minimal(extra: &str) -> String {
    format!(
        "schema_version = 1\n\
         [[tag]]\nname = \"月\"\nkind = \"imagery\"\ngloss = \"月。\"\n\
         body_keywords = [\"明月\"]\ntitle_keywords = []\n{extra}"
    )
}

#[test]
fn an_anthology_tag_with_keyword_rules_is_rejected() {
    let text = minimal(
        "[[tag]]\nname = \"唐诗三百首\"\nkind = \"anthology\"\ngloss = \"选本。\"\n\
         title_keywords = [\"三百首\"]\n",
    );
    let error = TagVocabulary::parse(&text).expect_err("选本带规则必须被拒");
    assert!(
        error.to_string().contains("唐诗三百首"),
        "错误必须点名那个标签：{error}"
    );
}

#[test]
fn a_tag_with_no_rules_and_no_explanation_is_rejected() {
    let text = minimal("[[tag]]\nname = \"哲理\"\nkind = \"theme\"\ngloss = \"理。\"\n");
    let error = TagVocabulary::parse(&text).expect_err("无规则又无说明必须被拒");
    assert!(error.to_string().contains("哲理"), "{error}");
    assert!(error.to_string().contains("rule_note"), "{error}");
}

#[test]
fn a_reviewed_entry_needs_a_real_reason() {
    let text = minimal(
        "[[reviewed]]\nauthor = \"李白\"\ntitle = \"静夜思\"\nadd = [\"月\"]\nreason = \"好\"\n",
    );
    let error = TagVocabulary::parse(&text).expect_err("理由过短必须被拒");
    assert!(error.to_string().contains("静夜思"), "{error}");
}

#[test]
fn an_undeclared_tag_name_in_the_reviewed_list_is_rejected() {
    let text = minimal(
        "[[reviewed]]\nauthor = \"李白\"\ntitle = \"静夜思\"\nadd = [\"不存在的标签\"]\n\
         reason = \"这是一条足够长的理由文本\"\n",
    );
    let error = TagVocabulary::parse(&text).expect_err("未声明的标签必须被拒");
    assert!(error.to_string().contains("不存在的标签"), "{error}");
}

#[test]
fn the_same_poem_may_not_appear_twice_in_the_reviewed_list() {
    let text = minimal(
        "[[reviewed]]\nauthor = \"李白\"\ntitle = \"静夜思\"\nadd = [\"月\"]\n\
         reason = \"这是一条足够长的理由文本\"\n\
         [[reviewed]]\nauthor = \"李白\"\ntitle = \"静夜思\"\ndeny = [\"月\"]\n\
         reason = \"这是另一条足够长的理由文本\"\n",
    );
    let error = TagVocabulary::parse(&text).expect_err("同一首诗两条评审必须被拒");
    assert!(error.to_string().contains("静夜思"), "{error}");
}

#[test]
fn adding_and_denying_the_same_tag_is_rejected() {
    let text = minimal(
        "[[reviewed]]\nauthor = \"李白\"\ntitle = \"静夜思\"\nadd = [\"月\"]\ndeny = [\"月\"]\n\
         reason = \"这是一条足够长的理由文本\"\n",
    );
    let error = TagVocabulary::parse(&text).expect_err("同时增删必须被拒");
    assert!(error.to_string().contains('月'), "{error}");
}

/// 无效 `deny` 必须让构建失败——这是 `deny` 机制存在的可证伪对照。
///
/// 同时钉住它的**作用域**：所指的诗不在本次范围内时不算死配置，否则
/// `corpus-measure` 在 10k 抽样上跑就会整体失败。
#[test]
fn a_deny_that_removes_nothing_fails_the_build_but_only_when_the_poem_is_in_scope() {
    let normalizer = Normalizer::new().expect("初始化归一器");
    let text = minimal(
        "[[reviewed]]\nauthor = \"王维\"\ntitle = \"鹿柴\"\ndeny = [\"月\"]\n\
         reason = \"这首诗里根本没有月，这条 deny 什么都不会移除\"\n",
    );
    let vocabulary = TagVocabulary::parse(&text).expect("结构本身合法");

    let in_scope = [PoemFacts {
        stable_id: "fixture:lucai",
        author: "王维",
        title: "鹿柴",
        body: "空山不见人，但闻人语响。",
    }];
    let error = assign_tags_to_facts(&vocabulary, &in_scope, &normalizer)
        .expect_err("范围内的无效 deny 必须失败");
    assert!(error.to_string().contains("鹿柴"), "错误要点名它：{error}");
    assert!(error.to_string().contains("死配置"), "{error}");

    let out_of_scope = [PoemFacts {
        stable_id: "fixture:jingyesi",
        author: "李白",
        title: "静夜思",
        body: "床前明月光，疑是地上霜。",
    }];
    let assignment = assign_tags_to_facts(&vocabulary, &out_of_scope, &normalizer)
        .expect("所指的诗不在范围内时不算死配置");
    assert!(
        tags_of(&assignment, "fixture:jingyesi").contains("月"),
        "范围内的那首诗仍要正常打标"
    );
}

#[test]
fn a_wrong_schema_version_is_refused_rather_than_guessed() {
    let text = minimal("").replace("schema_version = 1", "schema_version = 2");
    let error = TagVocabulary::parse(&text).expect_err("版本不符必须被拒");
    assert!(error.to_string().contains('2'), "{error}");
}
