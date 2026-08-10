//! 对 `corpus/reports/index-mode.json` 这份裁决文件的断言。
//!
//! 报告由 `cargo run -p xtask -- index-spike` 生成，是一份**实测结论**；这个文件负责
//! 保证那份结论自身是自洽的、且真的支撑得住它给出的选型。没有这层断言，报告就只是
//! 一段可以随意手改的 JSON，而 todo 19 与 24 会拿它当权威去建索引和写路由。
//!
//! 具体守四件事：
//!
//! 1. **选定的配置两条硬门槛都不违反**。这是方案事先声明的选型规则，也是这份报告
//!    唯一有约束力的部分。
//! 2. **`detail=none` 真的在整句类上失手**。方案要求 spike 证明它抓到了那个已知陷阱：
//!    如果哪天报告显示 `detail=none` 全绿，只有两种可能——上游 FTS5 变了，或者实测
//!    根本没跑对。两种都必须让构建停下来，而不是让人以为省下的体积是白得的。
//! 3. **两字查询的 p95 在「有 / 无 n-gram 表」两种情况下都被记下来了**，所以启用它
//!    的决定是有证据的，而不是一句「据说更快」。
//! 4. **契约本身的规模与覆盖面**在报告里被如实记录（>= 30 条、18 类）。
//!
//! 报告不存在时这些用例**全部 panic 而不是跳过**：一个语料库门禁若能因为文件缺失而
//! 静默通过，它就不是门禁。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::Value;

const REPORT: &str = "../../corpus/reports/index-mode.json";

fn report() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    CELL.get_or_init(|| {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REPORT);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "读取索引模式裁决文件失败 {}: {e}。\n\
                 先运行 `cargo run -p xtask -- index-spike` 生成它。\n\
                 这里刻意 panic 而不是跳过：能因文件缺失而静默通过的门禁不是门禁。",
                path.display()
            )
        });
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("解析裁决文件失败 {}: {e}", path.display()))
    })
}

fn results() -> &'static Vec<Value> {
    report()["results"]
        .as_array()
        .expect("报告缺少 results 数组")
}

fn chosen() -> &'static Value {
    let mode = report()["chosen_mode"]
        .as_str()
        .expect("报告缺少 chosen_mode");
    let ngram = report()["ngram_aux_enabled"]
        .as_bool()
        .expect("报告缺少 ngram_aux_enabled");
    results()
        .iter()
        .find(|r| r["detail_mode"] == mode && r["ngram_aux"] == ngram)
        .unwrap_or_else(|| {
            panic!("报告选定了 detail={mode} ngram_aux={ngram}，但 results 里没有这个配置")
        })
}

fn config_id(r: &Value) -> &str {
    r["config_id"].as_str().unwrap_or("<无 config_id>")
}

#[test]
fn chosen_configuration_violates_neither_gate() {
    let c = chosen();
    let id = config_id(c);
    assert_eq!(
        c["passes_hits_gate"],
        Value::Bool(true),
        "{id} 被选中，但它未通过命中门槛，缺口：{}",
        c["hits_shortfall"]
    );
    assert_eq!(
        c["passes_latency_gate"],
        Value::Bool(true),
        "{id} 被选中，但它在样本规模上未通过延迟门槛：{}",
        c["latency_violations"]
    );
    assert_eq!(
        c["passes_projected_latency_gate"],
        Value::Bool(true),
        "{id} 被选中，但它外推到发布规模后未通过延迟门槛：{}",
        c["projected_latency_violations"]
    );
}

#[test]
fn chosen_configuration_meets_min_hits_on_every_single_entry() {
    // 不只信配置级的布尔位，逐条复核。布尔位是被计算出来的，聚合一旦写错就会
    // 掩盖真实缺口，而这正是这份报告最不能出错的地方。
    let c = chosen();
    let queries = c["queries"].as_array().expect("配置缺少 queries");
    let bad: Vec<String> = queries
        .iter()
        .filter(|q| q["meets_min_hits"] != Value::Bool(true))
        .map(|q| {
            format!(
                "{}（{}）期望 >= {}，实际 {}",
                q["id"], q["class"], q["expect_min_hits"], q["hits"]
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "选定配置 {} 上有 {} 条契约未达下界：{bad:?}",
        config_id(c),
        bad.len()
    );
}

#[test]
fn chosen_configuration_stays_within_latency_budget_on_every_entry() {
    let budget = report()["selection_rule"]["p95_budget_ms"]
        .as_f64()
        .expect("报告缺少 p95_budget_ms");
    let c = chosen();
    let queries = c["queries"].as_array().expect("配置缺少 queries");

    // 契约自己声明为 FullScan 的条目按定义无索引可用，方案要求把它显式标记出来
    // 提示用户，慢是已被承认的属性。它们必须出现在 acknowledged_full_scans 里，
    // 也就是豁免是可见的，而不是从判定里被悄悄抹掉。
    let acknowledged: BTreeSet<&str> = c["acknowledged_full_scans"]
        .as_array()
        .expect("配置缺少 acknowledged_full_scans")
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();

    for q in queries {
        let id = q["id"].as_str().unwrap_or("<无 id>");
        let p95 = q["p95_ms"]
            .as_f64()
            .unwrap_or_else(|| panic!("{id} 的 p95_ms 缺失或不是数字；每一条都必须有实测延迟"));
        if q["expect_plan"] == "FullScan" {
            assert!(
                acknowledged.contains(id),
                "{id} 声明为 FullScan 却没有出现在 acknowledged_full_scans 里——\
                 豁免必须是可见的"
            );
            continue;
        }
        assert!(
            p95 <= budget,
            "选定配置上 {id} 的 p95 {p95} ms 超出 {budget} ms 预算"
        );
    }
}

#[test]
fn detail_none_really_fails_a_whole_line_class() {
    // 方案要求这份 spike 证明它抓到了那个已知陷阱：detail=none 移除了 phrase 匹配
    // 所需的位置信息，而整句检索正是「只记得半句」时唯一能用的形态。
    // 报告若显示 detail=none 全绿，只有两种可能：上游 FTS5 变了，或者实测没跑对。
    // 两种都必须让构建停下来。
    let none_configs: Vec<&Value> = results()
        .iter()
        .filter(|r| r["detail_mode"] == "none")
        .collect();
    assert!(
        !none_configs.is_empty(),
        "报告里没有 detail=none 的实测结果，无法证明陷阱被抓到"
    );

    let whole_line_classes = ["whole_five_char_line", "whole_seven_char_line"];
    for c in none_configs {
        let shortfall = c["hits_shortfall"]
            .as_array()
            .expect("配置缺少 hits_shortfall");
        let whole_line_misses: Vec<&Value> = shortfall
            .iter()
            .filter(|s| {
                s["class"]
                    .as_str()
                    .is_some_and(|cl| whole_line_classes.contains(&cl))
            })
            .collect();
        assert!(
            !whole_line_misses.is_empty(),
            "{} 在整句类上没有出现召回缺口。这与 FTS5 的已知限制矛盾，\
             说明实测没跑对或上游行为已变，必须人工复核后再更新本断言",
            config_id(c)
        );
        // 缺口的原因必须是 FTS5 自己报的错，而不是「召回为 0」这种可能由别的
        // 原因造成的现象。有原文才能证明是 detail 模式导致的。
        let reasons: Vec<&str> = whole_line_misses
            .iter()
            .filter_map(|s| s["reason"].as_str())
            .collect();
        assert!(
            reasons
                .iter()
                .any(|r| r.contains("phrase queries are not supported")),
            "{} 的整句缺口原因里没有 FTS5 的 phrase 不支持原文，实际为 {reasons:?}",
            config_id(c)
        );
    }
}

#[test]
fn detail_full_is_the_only_mode_serving_whole_line_queries() {
    // 与上一条互补：证明 detail=full 真的解决了这个问题，而不是三种模式都不行、
    // 我们只是挑了个没那么差的。
    for c in results() {
        let shortfall = c["hits_shortfall"]
            .as_array()
            .expect("配置缺少 hits_shortfall");
        if c["detail_mode"] == "full" {
            assert!(
                shortfall.is_empty(),
                "{} 竟然有召回缺口：{shortfall:?}",
                config_id(c)
            );
        } else {
            assert!(
                !shortfall.is_empty(),
                "{} 没有任何召回缺口，与 FTS5 的 phrase 限制矛盾",
                config_id(c)
            );
        }
    }
}

#[test]
fn two_character_p95_is_recorded_both_with_and_without_the_ngram_table() {
    // 启用 n-gram 表是一个有成本的决定（索引体积增加一个数量级），因此必须有
    // 两侧的实测数字，不能只测启用的那一侧然后说它更快。
    const TWO_CHAR: &str = "q01-two-char-mingyue";
    let mut seen: Vec<(bool, f64, &str)> = Vec::new();
    for c in results() {
        if c["detail_mode"] != report()["chosen_mode"] {
            continue;
        }
        let q = c["queries"]
            .as_array()
            .expect("配置缺少 queries")
            .iter()
            .find(|q| q["id"] == TWO_CHAR)
            .unwrap_or_else(|| panic!("{} 缺少 {TWO_CHAR}", config_id(c)));
        let p95 = q["p95_ms"]
            .as_f64()
            .unwrap_or_else(|| panic!("{TWO_CHAR} 在 {} 上没有 p95", config_id(c)));
        seen.push((
            c["ngram_aux"].as_bool().unwrap_or(false),
            p95,
            q["executed_plan"].as_str().unwrap_or("<无>"),
        ));
    }
    assert_eq!(
        seen.len(),
        2,
        "两字查询的 p95 必须在「有 / 无 n-gram 表」两种情况下都被记录，实际记录了 {} 组",
        seen.len()
    );
    let with = seen
        .iter()
        .find(|(aux, _, _)| *aux)
        .expect("缺少启用 n-gram 的一组");
    let without = seen
        .iter()
        .find(|(aux, _, _)| !*aux)
        .expect("缺少不启用 n-gram 的一组");
    assert_eq!(
        with.2, "Ngram",
        "启用 n-gram 表后两字查询应当走 Ngram 路径，实际走了 {}",
        with.2
    );
    assert_eq!(
        without.2, "BareLikeFts",
        "不启用 n-gram 表时两字查询只能退化成裸 LIKE，实际走了 {}。\
         若这里变了，说明退化被别的机制补救了，选型理由需要重写",
        without.2
    );
    assert!(
        with.1 < without.1,
        "启用 n-gram 表后两字查询反而更慢（{} ms vs {} ms），启用它的理由不成立",
        with.1,
        without.1
    );
}

#[test]
fn scale_projection_shows_the_bare_like_path_growing_with_corpus_size() {
    // 这条投射是「n-gram 表在真实规模下必要」这个结论的全部依据。10k 上裸 LIKE 只要
    // 几毫秒，若只看那一个点，六种配置都能过 150 ms，规则一条也筛不掉。
    let points = report()["scale_projection"]
        .as_array()
        .expect("报告缺少 scale_projection");
    assert!(
        points.len() >= 3,
        "规模投射至少需要 3 个点才能看出增长的形状，实际 {} 个",
        points.len()
    );
    let mut prev_scale = 0usize;
    let mut first_bare = None;
    let mut last_bare = None;
    for p in points {
        let n = p["poem_count"].as_u64().expect("投射点缺少 poem_count") as usize;
        assert!(n > prev_scale, "规模投射的点必须按规模递增");
        prev_scale = n;
        let bare = p["bare_like_fts_p95_ms"]
            .as_f64()
            .expect("投射点缺少 bare_like_fts_p95_ms");
        let ngram = p["ngram_path_p95_ms"]
            .as_f64()
            .expect("投射点缺少 ngram_path_p95_ms");
        assert!(
            ngram < bare,
            "规模 {n} 上 n-gram 路径（{ngram} ms）没有快于裸 LIKE（{bare} ms）"
        );
        first_bare.get_or_insert(bare);
        last_bare = Some(bare);
    }
    let (first, last) = (first_bare.unwrap(), last_bare.unwrap());
    assert!(
        last > first * 2.0,
        "裸 LIKE 的延迟没有随规模显著增长（{first} ms -> {last} ms）。\
         若它真的与规模无关，n-gram 表就不必要，选型结论需要重做"
    );
}

#[test]
fn report_records_the_full_contract_and_every_configuration() {
    let r = report();
    assert_eq!(r["schema_version"], 1, "报告 schema_version 变了");
    let entry_count = r["contract"]["entry_count"]
        .as_u64()
        .expect("报告缺少 contract.entry_count");
    assert!(entry_count >= 30, "报告记录的契约只有 {entry_count} 条");
    assert_eq!(
        r["contract"]["class_count"].as_u64(),
        Some(18),
        "报告记录的契约类别数不是 18"
    );
    assert_eq!(
        r["contract"]["path"].as_str(),
        Some("crates/yunjian-core/tests/queries.toml"),
        "契约路径变了；它是不可变契约，只有一处"
    );

    assert!(results().len() >= 3, "至少要有三种 detail 模式的实测结果");
    let modes: BTreeSet<&str> = results()
        .iter()
        .filter_map(|c| c["detail_mode"].as_str())
        .collect();
    for m in ["none", "column", "full"] {
        assert!(modes.contains(m), "报告缺少 detail={m} 的实测结果");
    }

    for c in results() {
        let id = config_id(c);
        assert!(
            c["index_bytes"].as_i64().unwrap_or(0) > 0,
            "{id} 的 index_bytes 不是正数"
        );
        let queries = c["queries"].as_array().expect("配置缺少 queries");
        assert_eq!(
            queries.len() as u64,
            entry_count,
            "{id} 只跑了 {} 条契约，应当跑满 {entry_count} 条",
            queries.len()
        );
        for q in queries {
            assert!(q["p95_ms"].is_number(), "{id} 的 {} 缺少 p95_ms", q["id"]);
            assert!(
                q["explain_query_plan"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
                "{id} 的 {} 缺少 EXPLAIN QUERY PLAN。方案禁止在没有它的情况下\
                 声称一条路径是索引化的",
                q["id"]
            );
        }
    }
}

#[test]
fn report_states_that_the_sample_corpus_is_synthetic() {
    // 报告里的数字很容易被后来的人当成真实语料的实测值。来源说明必须写清是合成的，
    // 否则 todo 20 会拿这些数字当基线去比对真实规模的测量结果。
    let provenance = report()["corpus"]["provenance"]
        .as_str()
        .expect("报告缺少 corpus.provenance");
    assert!(
        provenance.contains("合成"),
        "样本来源说明必须明确写出这是合成语料，实际为：{provenance}"
    );
    let poem_count = report()["corpus"]["poem_count"]
        .as_u64()
        .expect("报告缺少 corpus.poem_count");
    assert!(
        poem_count >= 10_000,
        "样本规模 {poem_count} 首不足方案要求的 1 万首"
    );
    assert!(
        report()["corpus"]["fixture_poems_embedded"]
            .as_u64()
            .is_some_and(|n| n >= 15),
        "报告必须记录嵌入的 fixture 诗数量，且它们是契约锚点存在的依据"
    );
}

#[test]
fn metadata_lookups_use_an_index_rather_than_scanning_the_poem_table() {
    // 这一条是实测出来的教训固化成的断言，不是照抄方案。
    //
    // spike 最初把标签与逐句末字存成 denormalized 字符串列（`tags` 用逗号连接、
    // `last_chars` 直接拼接），用 `LIKE '%思乡%'` 查。`EXPLAIN QUERY PLAN` 当场
    // 报 `SCAN poem`，外推到发布规模后逼近 150 ms 预算——在 1 万首上完全看不出问题。
    // 改成规范化的 `poem_tag` / `poem_last_char` 多对多表 + 覆盖索引后降到 0.1 ms 以下。
    //
    // 断言的是：任何声明为 `Meta` 的契约都不得退化成基表全扫。todo 17 建 schema、
    // todo 26 / 27 写元数据检索时若又用回 denormalized 列，这里会变红。
    let c = chosen();
    let offenders: Vec<String> = c["queries"]
        .as_array()
        .expect("配置缺少 queries")
        .iter()
        .filter(|q| q["expect_plan"] == "Meta")
        .filter(|q| {
            q["explain_query_plan"].as_array().is_some_and(|lines| {
                lines.iter().any(|l| {
                    l.as_str().is_some_and(|s| {
                        s.trim_start().starts_with("SCAN poem") && !s.contains("COVERING INDEX")
                    })
                })
            })
        })
        .map(|q| format!("{}（{}）", q["id"], q["class"]))
        .collect();
    assert!(
        offenders.is_empty(),
        "这些元数据检索退化成了基表全扫：{offenders:?}。\
         元数据检索必须走 B-tree 索引；若是靠 denormalized 字符串列 + LIKE 实现的，\
         改成规范化的多对多表 + 覆盖索引"
    );
}

#[test]
fn chosen_configuration_has_no_entry_scraping_past_the_budget() {
    // 「勉强达标」也要被看见：一条外推到 140 ms 的查询形式上通过，但它离预算只差一点，
    // 且慢的原因通常是走了全扫。把它和外推 4 ms 的条目混在「全部通过」里，
    // 等于把一个已知会在真实规模上吃紧的实现细节藏起来。
    let c = chosen();
    let near = c["projected_near_misses"]
        .as_array()
        .expect("配置缺少 projected_near_misses");
    assert!(
        near.is_empty(),
        "选定配置 {} 上有条目外推后已超过预算的一半：{near:?}。\
         它们形式上通过了门槛，但应当先改掉走全扫的实现再定稿",
        config_id(c)
    );
}
