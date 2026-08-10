//! `xtask corpus-contract`：在**新建出来的**语料库上逐条跑黄金查询契约，作为门禁。
//!
//! # 这个子命令为什么存在
//!
//! 契约文件（`crates/yunjian-core/tests/queries.toml`）此前有两处消费：
//! `cargo test -p yunjian-core --test golden_queries` 在随仓 fixture 上证明每一条
//! **可满足**，`cargo test -p yunjian-corpus fts::` 在 fixture 规模的真实 FTS5 索引上
//! 证明索引真的被用上。两者都不覆盖「一万首规模的语料库上这些查询还成立吗」——
//! 而索引行为恰恰随规模变化：`%明月%` 在 19 首上无论怎么查都是零点几毫秒，
//! 在 10 万首上裸 LIKE 是 52 ms、走 n-gram 候选表是 0.5 ms（实测见
//! `corpus/reports/index-mode.md`）。所以本子命令建一个样本规模的库再逐条跑。
//!
//! # 判定
//!
//! 逐条断言四件事，任一不成立即失败并点名：
//!
//! 1. 查询能被执行（`MATCH` 在 `detail != full` 下会被 FTS5 直接拒绝）；
//! 2. **实际走的物理路径等于契约声明的 `expect_plan`**——这一条拦的是「结果对但路径退化」，
//!    例如两字查询掉到 `BareLikeFts` 上：召回一模一样，只是在发布规模下慢两个数量级；
//! 3. 命中数达到 `expect_min_hits`；
//! 4. `expect_top_id` 指的那首诗真的在命中集合里（`Empty` 计划除外，它按定义零命中）。
//!
//! 于是「任何一条契约的结果变化而 `queries.toml` 未同步修改即失败」有了可执行形态：
//! 想让本门禁重新变绿，只有两条路——把语料/索引/路由改回去，或者显式修改契约文件。
//!
//! 判定只到契约自己声明的粒度为止。**不断言精确命中数**：契约把 `expect_min_hits`
//! 定义为下界（「语料变大只会让它更容易满足」），拿它当等式会与契约自身的语义矛盾，
//! 也会让每次样本规模调整都变成一次假失败。
//!
//! # 不写任何文件
//!
//! 本子命令是纯门禁：除了 `corpus/build/`（已 gitignore）下的临时库以外不产出任何东西。
//! 刻意不写报告——写报告的子命令在 CI 里会与提交的版本产生漂移，而漂移检查是
//! `corpus-quality` 的职责，不该在这里再来一遍。

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::index_spike::{ContractOutcome, ContractRun, execute_contract, repo_root};
use crate::verify_sources::emit;

/// 索引选型裁决文件。`detail` 模式与 n-gram 开关都从这里读，不在本文件里硬编码——
/// 与 todo 19 建索引时的约定完全一致：想换模式必须先改裁决，而改了裁决契约就会失败。
const VERDICT: &str = "corpus/reports/index-mode.json";

/// 契约至少要有多少条。与 `golden_queries.rs` 的 `contract_holds_at_least_30_entries`
/// 同一个数字：方案要求 30+，实际 37。
const MIN_ENTRIES: usize = 30;

/// 方案声明的类别数。少一类说明有人删掉了一整类查询形态的覆盖。
const REQUIRED_CLASSES: usize = 18;

/// 短查询陷阱的三条具名回归入口，与 `golden_queries.rs` 的
/// `SHORT_QUERY_TRAP_QUERIES` 是同一份名单。
///
/// 为什么在这里再钉一遍而不是只靠「逐条 expect_min_hits」：那条通用判定在有人把
/// `q01` 整条删掉之后仍然通过——剩下的条目照样满足规则。而这三个词是用户最常输入的
/// 形态，FTS5 trigram 在 3 字以下匹配不到任何行，删掉任何一条就等于放弃那个陷阱的回归。
const SHORT_QUERY_TRAPS: [&str; 3] = ["明月", "相思", "李白"];

#[derive(Debug, Deserialize)]
struct Verdict {
    chosen_mode: String,
    ngram_aux_enabled: bool,
}

pub fn run(scale: usize) -> Result<()> {
    if scale < 1_000 {
        bail!("样本规模至少 1000 首，规模以下的库上索引行为对产物不成立；收到 {scale}");
    }

    let root = repo_root()?;
    let verdict = load_verdict(&root)?;

    emit("== 黄金查询契约门禁 ==");
    emit(&format!(
        "裁决 {VERDICT}：detail={} ngram_aux={}",
        verdict.chosen_mode, verdict.ngram_aux_enabled
    ));

    let run = execute_contract(scale, &verdict.chosen_mode, verdict.ngram_aux_enabled)?;
    report(&run);

    let violations = judge(&run);
    if violations.is_empty() {
        emit("");
        emit(&format!(
            "契约门禁通过：{} 条全部达到声明的计划、下界与锚点",
            run.outcomes.len()
        ));
        return Ok(());
    }

    let mut msg = format!(
        "契约门禁未通过，{} 项违规。任一违规都意味着检索行为已经变化——\
         请修回实现，或者显式修改 {} 并说明为什么新的期望是对的：\n",
        violations.len(),
        run.contract_path
    );
    for v in &violations {
        msg.push_str("  - ");
        msg.push_str(v);
        msg.push('\n');
    }
    bail!(msg)
}

/// 逐条判定。返回全部违规而不是遇到第一条就返回：一次跑完能看到所有坏掉的条目，
/// 否则修一条跑一遍，改索引这种会同时影响多类查询的改动要来回十几次。
fn judge(run: &ContractRun) -> Vec<String> {
    let mut out = Vec::new();

    if run.outcomes.len() < MIN_ENTRIES {
        out.push(format!(
            "契约只有 {} 条，方案要求至少 {MIN_ENTRIES} 条",
            run.outcomes.len()
        ));
    }
    if run.class_count != REQUIRED_CLASSES {
        out.push(format!(
            "契约覆盖 {} 类，方案声明 {REQUIRED_CLASSES} 类；类别是覆盖度的度量单位，不得增减",
            run.class_count
        ));
    }

    for o in &run.outcomes {
        out.extend(judge_entry(o));
    }

    for want in SHORT_QUERY_TRAPS {
        let matching: Vec<&ContractOutcome> =
            run.outcomes.iter().filter(|o| o.query == want).collect();
        if matching.is_empty() {
            out.push(format!(
                "契约里没有查询「{want}」的条目——短查询陷阱的回归覆盖被删掉了"
            ));
            continue;
        }
        for o in matching {
            if o.expect_min_hits < 1 {
                out.push(format!(
                    "{}: 查询「{want}」的 expect_min_hits 是 {}，必须 >= 1，否则零命中也算通过",
                    o.id, o.expect_min_hits
                ));
            }
            if o.hits == 0 {
                out.push(format!(
                    "{}: 查询「{want}」在 {} 首规模的语料库上零命中——\
                     这正是 FTS5 trigram 3 字以下匹配不到任何行的那个陷阱",
                    o.id, run.poem_count
                ));
            }
        }
    }

    out
}

fn judge_entry(o: &ContractOutcome) -> Vec<String> {
    let mut out = Vec::new();

    if let Some(err) = &o.error {
        out.push(format!("{}（{}）查询无法执行：{err}", o.id, o.class));
        // 执行都失败了，后面几条判定的输入是无意义的 0，不再叠加噪声。
        return out;
    }

    if o.executed_plan != o.expect_plan {
        out.push(format!(
            "{}（{}）声明走 {} 实际走 {}——召回可能一样，但物理路径变了；\
             路径退化在功能测试里看不出来，只会在发布规模上变慢",
            o.id, o.class, o.expect_plan, o.executed_plan
        ));
    }

    if o.hits < o.expect_min_hits {
        out.push(format!(
            "{}（{}）「{}」归一化为「{}」，只命中 {} 首，低于下界 {}",
            o.id, o.class, o.query, o.normalized, o.hits, o.expect_min_hits
        ));
    }

    if o.expect_plan == "Empty" {
        if o.hits != 0 {
            out.push(format!(
                "{}（{}）Empty 计划命中了 {} 首；纯标点输入必须返回空结果，不得退化成扫描",
                o.id, o.class, o.hits
            ));
        }
    } else if !o.anchor_found {
        out.push(format!(
            "{}（{}）锚 {} 不在命中集合里——expect_top_id 指的那首诗没被查出来",
            o.id, o.class, o.expect_top_id
        ));
    }

    out
}

fn report(run: &ContractRun) {
    emit(&format!(
        "样本语料：{} 首，{} 个不同汉字，SQLite {}，建库 {} ms",
        run.poem_count, run.distinct_chars, run.sqlite_version, run.build_ms
    ));
    // 复述实际建出来的配置而不是裁决文件里读到的值：日志要能证明「跑契约的那个库」
    // 用的就是被选定的模式，而不是只证明「裁决文件里写着那个模式」。
    emit(&format!(
        "实际建出：detail={} ngram_aux={}（{} 行 n-gram）",
        run.detail_mode, run.ngram_aux, run.ngram_rows
    ));
    emit(&format!(
        "契约 {}（schema v{}）：{} 条，{} 类",
        run.contract_path,
        run.schema_version,
        run.outcomes.len(),
        run.class_count
    ));
    emit("");
    emit("  契约 id                          类别                   计划    命中  下界  锚");
    for o in &run.outcomes {
        let plan = if o.executed_plan == o.expect_plan {
            o.expect_plan.clone()
        } else {
            format!("{}!={}", o.expect_plan, o.executed_plan)
        };
        let anchor = if o.expect_plan == "Empty" {
            "—"
        } else if o.anchor_found {
            "有"
        } else {
            "缺"
        };
        emit(&format!(
            "  {:<32} {:<22} {:<8} {:>4} {:>5}  {anchor}{}",
            o.id,
            o.class,
            plan,
            o.hits,
            o.expect_min_hits,
            o.error
                .as_ref()
                .map(|e| format!("  错误：{e}"))
                .unwrap_or_default()
        ));
    }
}

fn load_verdict(root: &Path) -> Result<Verdict> {
    let path = root.join(VERDICT);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "读取索引选型裁决失败 {}；先跑 `cargo run -p xtask -- index-spike`",
            path.display()
        )
    })?;
    serde_json::from_str(&text).with_context(|| format!("解析 {VERDICT} 失败"))
}
