//! 黄金查询契约的自检用例。
//!
//! 这个文件**不检索真实语料**——真实语料由 todo 11/12 产出，而契约必须在那之前就能跑。
//! 它做的是三件事：
//!
//! 1. 让契约里每一条都成为一个具名 `#[test]`，于是
//!    `cargo test -p yunjian-core --test golden_queries -- --list` 的枚举数就是契约条数。
//!    这是「契约有多少条」唯一不会说谎的度量：libtest 只能列出编译期注册的用例，
//!    所以数据文件被删空时枚举数不会跟着变，反而会由 `contract_ids_match_generated_tests`
//!    当场报错。
//! 2. 证明每一条**可满足**：按契约声明的计划语义，在随仓 fixture 上真的能命中，
//!    且命中数不低于 `expect_min_hits`。一条无法在 fixture 上满足的契约是坏契约，
//!    会在 todo 22 的 CI 里变成一个没人能修的红灯。
//! 3. 守住覆盖面：18 个类别一个不缺，id 唯一，锚全部可解析。
//!
//! 这里的归一化与命中判定是**参考实现**，刻意写得直白且与 `yunjian-core` 的生产代码
//! 无耦合——todo 24 的 `normalize_query` / `plan_query` 必须与它行为一致，若不一致，
//! 应当是生产代码来对齐契约，而不是反过来改契约。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

// ---------------------------------------------------------------- 契约数据结构

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema_version: u32,
    fixture_file: String,
    #[serde(rename = "query")]
    queries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    query: String,
    class: String,
    expect_plan: String,
    expect_top_id: String,
    expect_min_hits: usize,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    schema_version: u32,
    #[serde(rename = "variant")]
    variants: Vec<Variant>,
    #[serde(rename = "poem")]
    poems: Vec<Poem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Poem {
    stable_id: String,
    title: String,
    author: String,
    dynasty: String,
    ci_tune: String,
    body: String,
    first_line: String,
    last_chars: Vec<String>,
    rhyme_book: String,
    rhyme_group: String,
    tags: Vec<String>,
    note: String,
}

/// 18 个必需类别。元测试断言契约把它们全部覆盖，因此后续无法悄悄删掉一类。
const REQUIRED_CLASSES: [&str; 18] = [
    "two_char_word",
    "two_char_author",
    "three_char_phrase",
    "four_char_phrase",
    "whole_five_char_line",
    "whole_seven_char_line",
    "traditional_input",
    "variant_char_input",
    "rare_char",
    "title_lookup",
    "ci_tune_lookup",
    "ci_tune_title_lookup",
    "first_line_prefix",
    "last_char_lookup",
    "rhyme_group_query",
    "tag_query",
    "punctuation_only",
    "no_three_char_run",
];

/// 合法的计划取值。todo 24 的 `QueryPlan` 变体名必须与此一致。
const VALID_PLANS: [&str; 6] = ["Ngram", "Match", "Like", "Empty", "FullScan", "Meta"];

/// 归一化时被剥离的标点。
///
/// 两处刻意的排除：
/// - **不含 `%` 与 `_`**：那两个是 LIKE 通配符，`no_three_char_run` 类的查询就靠
///   它们表达「没有三字连续段」这一形态，剥掉就等于换了一条契约。
/// - **不含 `·`**：中点在本语料里不是句读，而是「词牌·题目」的结构分隔符
///   （上游宋词的 `rhythmic` 字段就是这个形态）。剥掉它，「念奴娇·赤壁怀古」会变成
///   「念奴娇赤壁怀古」而永远匹配不到任何题目。
const PUNCTUATION: &str = "，。！？；：、「」『』《》〈〉（）【】—…,.!?;:'\"()[]{}<>-";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn contract() -> &'static Contract {
    static CELL: OnceLock<Contract> = OnceLock::new();
    CELL.get_or_init(|| {
        let path = manifest_dir().join("tests/queries.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读取契约失败 {}: {e}", path.display()));
        toml::from_str(&text).unwrap_or_else(|e| panic!("解析契约失败 {}: {e}", path.display()))
    })
}

fn fixtures() -> &'static Fixtures {
    static CELL: OnceLock<Fixtures> = OnceLock::new();
    CELL.get_or_init(|| {
        let path = manifest_dir()
            .join("tests")
            .join(&contract().fixture_file)
            .canonicalize()
            .unwrap_or_else(|e| panic!("fixture 路径无法解析: {e}"));
        load_fixtures(&path)
    })
}

fn load_fixtures(path: &Path) -> Fixtures {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("读取 fixture 失败 {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("解析 fixture 失败 {}: {e}", path.display()))
}

fn variant_map() -> &'static BTreeMap<char, char> {
    static CELL: OnceLock<BTreeMap<char, char>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut map = BTreeMap::new();
        for v in &fixtures().variants {
            let from = single_char(&v.from, "variant.from");
            let to = single_char(&v.to, "variant.to");
            assert!(
                map.insert(from, to).is_none(),
                "变体映射表里 {from} 出现了两次"
            );
        }
        map
    })
}

fn single_char(s: &str, what: &str) -> char {
    let mut it = s.chars();
    let c = it.next().unwrap_or_else(|| panic!("{what} 不能为空"));
    assert!(it.next().is_none(), "{what} 必须是单个字符，实际为 {s:?}");
    c
}

// ---------------------------------------------------------------- 参考实现

/// 剥标点 + 逐字过变体映射。与 todo 24 `normalize_query` 的期望行为一致。
fn normalize(query: &str) -> String {
    let map = variant_map();
    query
        .chars()
        .filter(|c| !PUNCTUATION.contains(*c) && !c.is_whitespace())
        .map(|c| map.get(&c).copied().unwrap_or(c))
        .collect()
}

/// 字面连续段（按 `%` / `_` 切开）里最长的一段有多少个字。
fn max_literal_run(pattern: &str) -> usize {
    pattern
        .split(['%', '_'])
        .map(|seg| seg.chars().count())
        .max()
        .unwrap_or(0)
}

fn has_wildcard(pattern: &str) -> bool {
    pattern.contains('%') || pattern.contains('_')
}

/// 只支持 `%`（任意长度）与 `_`（单字）的极小 glob 匹配器，语义对齐 SQL LIKE。
/// 用递归而非动态规划：模式最长 7 个字符，清晰度比渐进复杂度重要。
fn like_match(haystack: &[char], pattern: &[char]) -> bool {
    match pattern.first() {
        None => haystack.is_empty(),
        Some('%') => (0..=haystack.len()).any(|skip| like_match(&haystack[skip..], &pattern[1..])),
        Some('_') => !haystack.is_empty() && like_match(&haystack[1..], &pattern[1..]),
        Some(&p) => haystack.first() == Some(&p) && like_match(&haystack[1..], &pattern[1..]),
    }
}

/// 按契约声明的计划语义，统计这条查询在随仓 fixture 上命中了哪些诗。
///
/// 返回命中的 `stable_id` 集合。`Empty` 计划恒返回空集——这是它的定义，不是退化。
fn hits(entry: &Entry) -> BTreeSet<String> {
    let norm = normalize(&entry.query);
    let mut out = BTreeSet::new();
    if entry.expect_plan == "Empty" {
        return out;
    }
    for p in &fixtures().poems {
        let hit = match entry.expect_plan.as_str() {
            "Meta" => meta_hit(entry, p, &norm),
            "FullScan" => {
                let pattern: Vec<char> = format!("%{norm}%").chars().collect();
                like_match(&p.body.chars().collect::<Vec<_>>(), &pattern)
            }
            // Ngram / Match / Like 三条计划的**召回语义完全相同**（同一个子串），
            // 区别只在走哪条物理路径。契约在这里只断言召回，物理路径由实测报告
            // (`corpus/reports/index-mode.json`) 与 todo 24 的单测负责。
            _ => p.body.contains(&norm),
        };
        if hit {
            out.insert(p.stable_id.clone());
        }
    }
    out
}

fn meta_hit(entry: &Entry, p: &Poem, norm: &str) -> bool {
    match entry.class.as_str() {
        "two_char_author" => p.author == norm,
        "title_lookup" | "ci_tune_title_lookup" => p.title == norm,
        "ci_tune_lookup" => p.ci_tune == norm,
        "first_line_prefix" => p.first_line.starts_with(norm),
        "last_char_lookup" => p.last_chars.iter().any(|c| c == norm),
        "rhyme_group_query" => !p.rhyme_group.is_empty() && p.rhyme_group == norm,
        "tag_query" => p.tags.iter().any(|t| t == norm),
        other => panic!("类别 {other} 声明了 Meta 计划，但参考实现不知道它该查哪一列"),
    }
}

fn entry_by_id(id: &str) -> &'static Entry {
    contract()
        .queries
        .iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("契约里找不到 id={id}；生成的用例与 queries.toml 已经脱节"))
}

fn poem_by_id(stable_id: &str) -> Option<&'static Poem> {
    fixtures().poems.iter().find(|p| p.stable_id == stable_id)
}

/// 单条契约的全部断言。每个生成的 `#[test]` 都只调用这一个函数。
fn check(id: &str) {
    let e = entry_by_id(id);

    assert!(!e.class.is_empty(), "{id}: class 不能为空");
    assert!(
        REQUIRED_CLASSES.contains(&e.class.as_str()),
        "{id}: class={} 不在 18 个必需类别里",
        e.class
    );
    assert!(
        VALID_PLANS.contains(&e.expect_plan.as_str()),
        "{id}: expect_plan={} 不是合法计划名",
        e.expect_plan
    );
    assert!(!e.query.is_empty(), "{id}: query 不能为空");
    assert!(
        e.note.chars().count() >= 10,
        "{id}: note 太短，契约里的每一条都必须写清它为什么存在"
    );

    let anchor = poem_by_id(&e.expect_top_id).unwrap_or_else(|| {
        panic!(
            "{id}: expect_top_id={} 在 fixtures/poems.toml 里不存在",
            e.expect_top_id
        )
    });

    let norm = normalize(&e.query);
    let len = norm.chars().count();

    // 计划与查询形态必须自洽。这一段是在防「类别写对了但计划写错」的情形——
    // 那种契约会让 todo 24 按错误的期望实现路由。
    match e.expect_plan.as_str() {
        "Empty" => {
            assert!(
                norm.is_empty(),
                "{id}: Empty 计划要求归一化后为空，实际 {norm:?}"
            );
            assert_eq!(e.expect_min_hits, 0, "{id}: Empty 计划的下界必须是 0");
        }
        "Ngram" => {
            assert!(
                (1..3).contains(&len),
                "{id}: Ngram 计划只服务归一化后 1-2 字的查询，实际 {len} 字"
            );
        }
        "Match" => {
            assert!(len >= 3, "{id}: Match 计划要求至少 3 字，实际 {len} 字");
            assert!(!has_wildcard(&norm), "{id}: Match 计划的查询不得含通配符");
        }
        "Like" => {
            assert!(len > 3, "{id}: Like 计划要求超过 3 字，实际 {len} 字");
            assert!(
                max_literal_run(&norm) >= 3,
                "{id}: Like 计划要求存在长度 >= 3 的字面连续段，否则应当是 FullScan"
            );
        }
        "FullScan" => {
            assert!(
                has_wildcard(&norm),
                "{id}: FullScan 类的查询应当含通配符，否则它就该走索引路径"
            );
            assert!(
                max_literal_run(&norm) < 3,
                "{id}: FullScan 的前提是没有长度 >= 3 的字面连续段，实际最长 {} 字",
                max_literal_run(&norm)
            );
        }
        "Meta" => {}
        other => panic!("{id}: 未处理的计划 {other}"),
    }

    if e.expect_plan == "Empty" {
        assert!(
            hits(e).is_empty(),
            "{id}: Empty 计划必须返回空结果，不得退化成扫描"
        );
        return;
    }

    assert!(
        e.expect_min_hits >= 1,
        "{id}: 非 Empty 计划的 expect_min_hits 必须 >= 1，否则这条契约什么也没断言"
    );

    let hit = hits(e);
    assert!(
        hit.len() >= e.expect_min_hits,
        "{id}: 在随仓 fixture 上只命中 {} 首（{:?}），低于下界 {}。契约不可满足，\
         必须补 fixture 或下调下界，不能留给 todo 22 的 CI 去发现",
        hit.len(),
        hit,
        e.expect_min_hits
    );
    assert!(
        hit.contains(&e.expect_top_id),
        "{id}: 锚 {} 竟然不在命中集合里（{:?}）——expect_top_id 指错了诗",
        e.expect_top_id,
        hit
    );

    // 锚必须真的与这条查询有关，而不是随手填的一首。
    assert!(
        !anchor.body.is_empty() && !anchor.title.is_empty() && !anchor.author.is_empty(),
        "{id}: 锚 {} 的必填字段有空值",
        anchor.stable_id
    );
}

// ------------------------------------------------------- 逐条契约的具名用例
//
// 这份 id 列表是 `queries.toml` 的**镜像**，由
// `contract_ids_match_generated_tests` 双向守住。加一条契约要同时在这里加一行——
// 这份冗余是刻意的：没有它，libtest 就无法枚举出契约条数，而「枚举数 >= 30」正是
// 本 todo 的验收判据之一。

macro_rules! contract_cases {
    ($($name:ident => $id:literal),+ $(,)?) => {
        /// 生成的用例覆盖的 id，供元测试与 `queries.toml` 双向比对。
        const GENERATED_IDS: &[&str] = &[$($id),+];

        $(
            #[test]
            fn $name() {
                check($id);
            }
        )+
    };
}

contract_cases! {
    q01_two_char_mingyue => "q01-two-char-mingyue",
    q02_two_char_xiangsi => "q02-two-char-xiangsi",
    q03_two_char_chunfeng => "q03-two-char-chunfeng",
    q04_two_char_author_libai => "q04-two-char-author-libai",
    q05_two_char_author_dufu => "q05-two-char-author-dufu",
    q06_three_char_mingyueguang => "q06-three-char-mingyueguang",
    q07_three_char_gurenxi => "q07-three-char-gurenxi",
    q08_four_char_bairiyishan => "q08-four-char-bairiyishan",
    q09_four_char_haishangmingyue => "q09-four-char-haishangmingyue",
    q10_line5_chuangqianmingyueguang => "q10-line5-chuangqianmingyueguang",
    q11_line5_bairiyishanjin => "q11-line5-bairiyishanjin",
    q12_line7_lianganyuansheng => "q12-line7-lianganyuansheng",
    q13_line7_gusuchengwai => "q13-line7-gusuchengwai",
    q14_traditional_guopo => "q14-traditional-guopo",
    q15_traditional_jutouwangmingyue => "q15-traditional-jutouwangmingyue",
    q16_variant_cechengfeng => "q16-variant-cechengfeng",
    q17_variant_bingsaichuan => "q17-variant-bingsaichuan",
    q18_rare_yixuxi => "q18-rare-yixuxi",
    q19_rare_two_char_xuxi => "q19-rare-two-char-xuxi",
    q20_title_jingyesi => "q20-title-jingyesi",
    q21_title_chunwang => "q21-title-chunwang",
    q22_citune_niannujiao => "q22-citune-niannujiao",
    q23_citune_shuidiaogetou => "q23-citune-shuidiaogetou",
    q24_citune_title_chibihuaigu => "q24-citune-title-chibihuaigu",
    q25_citune_title_mingyuejishiyou => "q25-citune-title-mingyuejishiyou",
    q26_firstline_chuangqian => "q26-firstline-chuangqian",
    q27_firstline_guoposhan => "q27-firstline-guoposhan",
    q28_lastchar_shuang => "q28-lastchar-shuang",
    q29_lastchar_liu => "q29-lastchar-liu",
    q30_rhyme_xiapingqiyang => "q30-rhyme-xiapingqiyang",
    q31_rhyme_xiapingshiersqin => "q31-rhyme-xiapingshiersqin",
    q32_tag_sixiang => "q32-tag-sixiang",
    q33_tag_biansai => "q33-tag-biansai",
    q34_punct_comma_period => "q34-punct-comma-period",
    q35_punct_mixed => "q35-punct-mixed",
    q36_nofullrun_ming_guang => "q36-nofullrun-ming-guang",
    q37_nofullrun_yue_shuang => "q37-nofullrun-yue-shuang",
}

// ---------------------------------------------------------------- 元测试

#[test]
fn contract_holds_at_least_30_entries() {
    let n = contract().queries.len();
    assert!(n >= 30, "契约只有 {n} 条，方案要求至少 30 条");
    assert_eq!(
        n,
        GENERATED_IDS.len(),
        "契约条数与生成用例数不一致：数据文件 {n} 条，生成 {} 条",
        GENERATED_IDS.len()
    );
}

#[test]
fn contract_ids_match_generated_tests() {
    let from_file: BTreeSet<&str> = contract().queries.iter().map(|e| e.id.as_str()).collect();
    let generated: BTreeSet<&str> = GENERATED_IDS.iter().copied().collect();
    let missing: Vec<_> = from_file.difference(&generated).collect();
    let extra: Vec<_> = generated.difference(&from_file).collect();
    assert!(
        missing.is_empty(),
        "queries.toml 里有契约没有对应的 #[test]：{missing:?}。\
         补上 contract_cases! 里的一行，否则 --list 会少算契约条数"
    );
    assert!(
        extra.is_empty(),
        "contract_cases! 里有 id 在 queries.toml 里不存在：{extra:?}"
    );
}

#[test]
fn contract_ids_are_unique() {
    let mut seen = BTreeSet::new();
    for e in &contract().queries {
        assert!(seen.insert(e.id.as_str()), "契约 id 重复：{}", e.id);
    }
}

#[test]
fn contract_covers_all_18_required_classes() {
    let present: BTreeSet<&str> = contract()
        .queries
        .iter()
        .map(|e| e.class.as_str())
        .collect();
    let missing: Vec<&str> = REQUIRED_CLASSES
        .iter()
        .copied()
        .filter(|c| !present.contains(c))
        .collect();
    assert!(missing.is_empty(), "契约缺少这些必需类别：{missing:?}");

    let unknown: Vec<&str> = present
        .iter()
        .copied()
        .filter(|c| !REQUIRED_CLASSES.contains(c))
        .collect();
    assert!(
        unknown.is_empty(),
        "契约出现了未登记的类别：{unknown:?}。新增类别要同时更新 REQUIRED_CLASSES 与方案"
    );
}

#[test]
fn every_anchor_resolves_to_a_committed_fixture() {
    for e in &contract().queries {
        assert!(!e.expect_top_id.is_empty(), "{}: expect_top_id 为空", e.id);
        assert!(
            poem_by_id(&e.expect_top_id).is_some(),
            "{}: 锚 {} 不在 fixtures/poems.toml 里",
            e.id,
            e.expect_top_id
        );
    }
}

#[test]
fn two_character_entries_all_route_to_the_ngram_path() {
    // 这是那个陷阱的守卫：两字查询若被路由到 Match，FTS5 trigram 会稳定返回 0 行；
    // 若被路由到裸 LIKE，会退化成整列扫描。两种坏法都不会报错，只会变慢或变空。
    for e in &contract().queries {
        if e.expect_plan == "Meta" || e.expect_plan == "Empty" {
            continue;
        }
        let len = normalize(&e.query).chars().count();
        if len < 3 && !has_wildcard(&e.query) {
            assert_eq!(
                e.expect_plan, "Ngram",
                "{}: 归一化后 {len} 字却声明了 {} 计划",
                e.id, e.expect_plan
            );
        }
    }
}

#[test]
fn fixture_set_is_internally_consistent() {
    let f = fixtures();
    assert_eq!(
        f.schema_version, 1,
        "fixture schema_version 变了，需要同步契约"
    );
    assert_eq!(contract().schema_version, 1, "契约 schema_version 变了");
    assert!(
        f.poems.len() >= 15,
        "fixture 只有 {} 首，不足以支撑 18 类",
        f.poems.len()
    );

    let mut ids = BTreeSet::new();
    for p in &f.poems {
        assert!(
            ids.insert(p.stable_id.as_str()),
            "fixture id 重复：{}",
            p.stable_id
        );
        assert!(
            p.stable_id.starts_with("fixture:"),
            "{}: fixture 的 stable_id 必须带 fixture: 前缀，以免与真实语料的 id 混淆",
            p.stable_id
        );
        assert!(!p.dynasty.is_empty(), "{}: dynasty 不能为空", p.stable_id);
        assert!(!p.note.is_empty(), "{}: note 不能为空", p.stable_id);

        // 首句与逐句末字都是预计算列（todo 17），必须与正文一致，否则元数据检索
        // 的断言会建立在错的数据上。
        let lines: Vec<&str> = p
            .body
            .split(['，', '。', '！', '？', '；'])
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(
            lines.first().copied().unwrap_or_default(),
            p.first_line,
            "{}: first_line 与正文首句不一致",
            p.stable_id
        );
        assert_eq!(
            lines.len(),
            p.last_chars.len(),
            "{}: last_chars 条数 {} 与句数 {} 不一致",
            p.stable_id,
            p.last_chars.len(),
            lines.len()
        );
        for (line, last) in lines.iter().zip(&p.last_chars) {
            assert_eq!(
                line.chars().last().map(String::from).unwrap_or_default(),
                *last,
                "{}: 句「{line}」的末字与 last_chars 记录的 {last} 不一致",
                p.stable_id
            );
        }
        assert_eq!(
            p.rhyme_book.is_empty(),
            p.rhyme_group.is_empty(),
            "{}: rhyme_book 与 rhyme_group 必须同时有值或同时留空",
            p.stable_id
        );
    }
}

#[test]
fn variant_map_covers_every_non_simplified_query() {
    // 繁体与异体两类契约的可满足性完全依赖这张映射表。逐条检查归一化确实改变了输入，
    // 否则一条「繁体输入」用例可能因为查询本来就是简体而空跑通过。
    for e in &contract().queries {
        if e.class != "traditional_input" && e.class != "variant_char_input" {
            continue;
        }
        let norm = normalize(&e.query);
        assert_ne!(
            norm, e.query,
            "{}: 声明为 {} 但归一化没有改变任何字符，这条用例什么也没验证",
            e.id, e.class
        );
    }
}
