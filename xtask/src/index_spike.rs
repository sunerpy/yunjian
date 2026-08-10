//! `xtask index-spike`：FTS5 `detail` 模式与辅助 n-gram 候选表的去风险实测。
//!
//! # 这个子命令为什么存在
//!
//! 方案研究阶段没能定下 FTS5 的 `detail` 模式，而选错会让**最常见的查询静默失效**：
//! `detail=none` 把体积压掉一半（第三方在约 20 倍于本项目规模上量到的），但它移除了
//! phrase 匹配所需的位置信息——恰好就是「只记得半句」时的整句检索。这类缺陷不会报错，
//! 只会让某一类查询查不到东西，因此必须实测而不是推断。
//!
//! 第二个被实测的问题独立于 `detail` 模式：`%明月%` 只有两个字面字符，FTS5 推不出
//! 任何 trigram 约束，所谓「索引 LIKE」在 1-2 字模式下会退化成对整个 body 列的虚表
//! 全扫。用户最常输入的形态反而最慢。方案因此要求同时实测一张辅助 n-gram 候选表。
//!
//! # 选型规则（事先声明、具有约束力）
//!
//! 选满足下面两条的**最小**配置：
//!
//! 1. 每一条契约都达到其 `expect_min_hits`；
//! 2. 每一条的 p95 <= 150 ms。
//!
//! 正确性与延迟都是硬门槛——一个靠扫 85 万行拿到正确答案的配置要被否掉。
//! 只有两条都通过时，体积才作为 tiebreaker。
//!
//! # 输出
//!
//! `corpus/reports/index-mode.json`（机器读，todo 19 / 24 的构建门禁读它，
//! 建出来的索引与结论不符就让构建失败）与同名 `.md`（人读）。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};

use crate::verify_sources::emit;

const CONTRACT: &str = "crates/yunjian-core/tests/queries.toml";
const REPORT_JSON: &str = "corpus/reports/index-mode.json";
const REPORT_MD: &str = "corpus/reports/index-mode.md";
const BUILD_DIR: &str = "corpus/build/index-spike";

/// p95 硬门槛，毫秒。方案事先声明，本子命令不得放宽。
const P95_BUDGET_MS: f64 = 150.0;

/// 每次测量前丢弃的预热轮数。第一轮把页读进 page cache，计进 p95 会把冷启动成本
/// 混进稳态延迟里，而用户面对的是一个已打开的语料库。
const WARMUP: usize = 3;

/// n-gram 候选表在真实规模下的收益投射所用的规模点。
///
/// 只在 `detail=full` + 有 n-gram 表这一种配置上跑，因为要观察的是**两字 LIKE 路径
/// 随规模的增长曲线**，而那条曲线与 `detail` 模式无关（LIKE 走的是 trigram 表的
/// body 列，不碰位置信息）。上游总量草案是 853,385 首，10k 一个点看不出线性性。
const PROJECTION_SCALES: [usize; 3] = [10_000, 50_000, 100_000];

/// 发布时的真实语料规模（上游总量草案，`.omo/drafts` 的 D3 记载）。
///
/// **为什么延迟门槛必须在这个规模上判定，而不只看 10k 的样本**：方案的选型规则原话是
/// 「一个靠扫 85 万行拿到正确答案的配置要被否掉」——那句话点名的就是这个数字。
/// 在 10k 样本上裸 LIKE 只要几毫秒，任何配置都能过 150 ms；但那条路径的成本随语料
/// 线性增长，等语料真的到 85 万首时它会远超预算。**只在样本规模上判定，等于让
/// 抽样规模替产品做决定**，正是这次 spike 要避免的那类静默失效。
const PRODUCTION_SCALE: usize = 853_385;

// ---------------------------------------------------------------- 契约数据结构
//
// 只反序列化实测需要的字段。`note` 也读进来是为了让 `deny_unknown_fields` 能生效——
// 契约加了字段而这里没跟上时，应当当场失败而不是静默忽略。

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema_version: u32,
    fixture_file: String,
    #[serde(rename = "query")]
    queries: Vec<ContractEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractEntry {
    id: String,
    query: String,
    class: String,
    expect_plan: String,
    expect_top_id: String,
    expect_min_hits: usize,
    #[allow(dead_code)]
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    #[allow(dead_code)]
    schema_version: u32,
    #[serde(rename = "variant")]
    variants: Vec<FixtureVariant>,
    #[serde(rename = "poem")]
    poems: Vec<FixturePoem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureVariant {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixturePoem {
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
    #[allow(dead_code)]
    note: String,
}

// ---------------------------------------------------------------- 报告数据结构

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    /// 选定的 `detail` 模式。todo 19 的 `CREATE VIRTUAL TABLE ... detail=<MODE>` 读这里。
    chosen_mode: String,
    /// 选定配置是否启用辅助 n-gram 候选表。todo 24 的 `len < 3` 分支读这里。
    ngram_aux_enabled: bool,
    /// 一句话理由。
    justification: String,
    selection_rule: SelectionRule,
    environment: Environment,
    corpus: CorpusInfo,
    contract: ContractInfo,
    results: Vec<ConfigResult>,
    /// 两字 LIKE 路径随规模的增长，用于把 n-gram 表的收益从 10k 外推到真实规模。
    scale_projection: Vec<ScalePoint>,
}

#[derive(Debug, Serialize)]
struct SelectionRule {
    p95_budget_ms: f64,
    /// 发布规模。延迟门槛在这个规模上判定，而不只在样本规模上。
    production_scale: usize,
    hits_gate: String,
    latency_gate: String,
    tiebreaker: String,
}

#[derive(Debug, Serialize)]
struct Environment {
    sqlite_version: String,
    page_size: i64,
    repeats_per_query: usize,
    warmup_per_query: usize,
    reference_machine: String,
}

#[derive(Debug, Serialize)]
struct CorpusInfo {
    poem_count: usize,
    /// 样本语料的来源说明。必须写清是合成的，否则读报告的人会以为这是真实语料的数字。
    provenance: String,
    synthesis_seed: u64,
    distinct_chars: usize,
    total_body_chars: usize,
    fixture_poems_embedded: usize,
}

#[derive(Debug, Serialize)]
struct ContractInfo {
    path: String,
    schema_version: u32,
    entry_count: usize,
    class_count: usize,
}

#[derive(Debug, Serialize)]
struct ConfigResult {
    config_id: String,
    detail_mode: String,
    ngram_aux: bool,
    /// FTS 索引（含 n-gram 辅助表）占用的字节，来自 `dbstat`。
    index_bytes: i64,
    fts_bytes: i64,
    ngram_bytes: i64,
    ngram_rows: i64,
    total_file_bytes: u64,
    build_ms: u128,
    passes_hits_gate: bool,
    /// 样本规模上实测的延迟门槛。
    passes_latency_gate: bool,
    /// **外推到发布规模后**的延迟门槛，见 `PRODUCTION_SCALE`。选型两条硬门槛里的
    /// 「延迟」指的是这一条：只看样本规模，等于让抽样规模替产品做决定。
    passes_projected_latency_gate: bool,
    /// 未达 `expect_min_hits` 的契约 id 与原因。
    hits_shortfall: Vec<Shortfall>,
    /// 超出 p95 预算的契约 id（样本规模实测）。
    latency_violations: Vec<LatencyViolation>,
    /// 外推到发布规模后超出预算的契约 id。
    projected_latency_violations: Vec<ProjectedViolation>,
    /// 契约自己声明为 `FullScan`、因而被延迟门槛豁免的条目。逐条记下来，
    /// 使豁免是可见的：调用方必须为这些形态提示用户，而不是假装它们很快。
    acknowledged_full_scans: Vec<ProjectedViolation>,
    /// 通过了门槛但外推值已超过预算一半的条目。
    ///
    /// 单独列出来的理由：一条外推到 137 ms 的查询在形式上是「通过」的，但它离 150 ms
    /// 只差一点，且它之所以慢是因为走了基表全扫。把它和外推 4 ms 的条目混在一张
    /// 「全部通过」的表里，等于把一个已知会在真实规模上出问题的实现细节藏起来。
    projected_near_misses: Vec<ProjectedViolation>,
    per_class: Vec<ClassSummary>,
    queries: Vec<QueryResult>,
}

#[derive(Debug, Serialize)]
struct Shortfall {
    id: String,
    class: String,
    expected_min_hits: usize,
    actual_hits: usize,
    reason: String,
}

#[derive(Debug, Serialize)]
struct LatencyViolation {
    id: String,
    class: String,
    p95_ms: f64,
}

/// 外推到发布规模后超出预算的契约。
///
/// 外推只对**扫描型路径**（`BareLikeFts` / `FullScan` / `FullScanFallback`）按规模
/// 线性放大，因为那些路径的成本正比于被扫过的行数；`Ngram` / `Match` / `Like` / `Meta`
/// 都由索引定位，成本随规模是对数级，线性外推会把它们严重高估。这个区分是保守的：
/// 它只会让「扫描型」配置更容易被否掉，不会放过任何一个真的会超预算的配置。
#[derive(Debug, Serialize)]
struct ProjectedViolation {
    id: String,
    class: String,
    executed_plan: String,
    measured_p95_ms: f64,
    projected_p95_ms: f64,
}

#[derive(Debug, Serialize)]
struct ClassSummary {
    class: String,
    entries: usize,
    entries_meeting_min_hits: usize,
    hit_rate: f64,
    worst_p95_ms: f64,
}

#[derive(Debug, Serialize)]
struct QueryResult {
    id: String,
    class: String,
    query: String,
    normalized: String,
    expect_plan: String,
    /// 实际执行的物理路径。与 `expect_plan` 不一致时说明该配置服务不了这条契约。
    executed_plan: String,
    expect_min_hits: usize,
    hits: usize,
    meets_min_hits: bool,
    anchor_found: bool,
    p95_ms: f64,
    median_ms: f64,
    /// `EXPLAIN QUERY PLAN` 的原始输出。这是「索引 LIKE 到底有没有避开全扫」
    /// 唯一可接受的证据——方案明确禁止在没有它的情况下声称一条路径是「索引化」的。
    explain_query_plan: Vec<String>,
    /// FTS5 在这条查询上直接报错时的原文（例如 detail!=full 下的 phrase 查询）。
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScalePoint {
    poem_count: usize,
    /// 两字查询「明月」经辅助 n-gram 表的 p95。
    ngram_path_p95_ms: f64,
    /// 同一条查询走裸 `LIKE` 打在 FTS 虚表 body 列上的 p95。
    bare_like_fts_p95_ms: f64,
    /// 同一条查询走裸 `LIKE` 打在基表上的 p95。
    bare_like_base_p95_ms: f64,
    speedup: f64,
    ngram_rows: i64,
    ngram_bytes: i64,
}

// ---------------------------------------------------------------- 入口

pub fn run(scale: usize, repeats: usize) -> Result<()> {
    if scale < 1_000 {
        bail!("样本规模至少 1000 首，实测才有意义；收到 {scale}");
    }
    if repeats < 5 {
        bail!("重复次数至少 5 次，否则 p95 只是噪声；收到 {repeats}");
    }

    let root = repo_root()?;
    let contract = load_contract(&root)?;
    let fixtures = load_fixtures(&root, &contract)?;

    emit("== FTS5 索引模式实测 ==");
    emit(&format!(
        "契约 {} 条（{} 类），样本规模 {scale} 首，每条查询 {WARMUP} 轮预热 + {repeats} 轮测量",
        contract.queries.len(),
        distinct_classes(&contract).len()
    ));

    let sample = synthesize_corpus(scale, &fixtures);
    emit(&format!(
        "合成样本：{} 首，{} 个不同汉字，正文共 {} 字（种子 {}）",
        sample.poems.len(),
        sample.distinct_chars,
        sample.total_body_chars,
        SYNTHESIS_SEED
    ));

    let build_dir = root.join(BUILD_DIR);
    std::fs::create_dir_all(&build_dir)
        .with_context(|| format!("创建构建目录失败 {}", build_dir.display()))?;

    let sqlite_version =
        Connection::open_in_memory()?
            .query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))?;

    let mut results = Vec::new();
    for mode in ["none", "column", "full"] {
        for ngram_aux in [false, true] {
            let r = measure_config(
                &build_dir, mode, ngram_aux, &sample, &contract, &fixtures, repeats,
            )?;
            emit(&format!(
                "  detail={mode:<6} ngram={:<5} 索引 {:>10} B  文件 {:>10} B  命中门槛 {}  样本延迟 {}  发布规模延迟 {}",
                ngram_aux,
                r.index_bytes,
                r.total_file_bytes,
                gate_mark(r.passes_hits_gate),
                gate_mark(r.passes_latency_gate),
                gate_mark(r.passes_projected_latency_gate),
            ));
            results.push(r);
        }
    }

    let projection = measure_scale_projection(&build_dir, &fixtures, repeats)?;
    for p in &projection {
        emit(&format!(
            "  规模 {:>7} 首：明月 走 n-gram {:>7.3} ms / 裸 LIKE(FTS) {:>8.3} ms / 裸 LIKE(基表) {:>8.3} ms  加速 {:.1}x",
            p.poem_count,
            p.ngram_path_p95_ms,
            p.bare_like_fts_p95_ms,
            p.bare_like_base_p95_ms,
            p.speedup
        ));
    }

    let (chosen, justification) = choose(&results, &projection)?;
    emit(&format!(
        "选定：detail={} ngram_aux={} —— {justification}",
        chosen.detail_mode, chosen.ngram_aux
    ));

    let report = Report {
        schema_version: 1,
        chosen_mode: chosen.detail_mode.clone(),
        ngram_aux_enabled: chosen.ngram_aux,
        justification,
        selection_rule: SelectionRule {
            p95_budget_ms: P95_BUDGET_MS,
            production_scale: PRODUCTION_SCALE,
            hits_gate: "每一条契约都达到其 expect_min_hits".to_string(),
            latency_gate: format!(
                "每一条契约的 p95 <= {P95_BUDGET_MS} ms，且外推到 {PRODUCTION_SCALE} 首发布规模后依然 <= {P95_BUDGET_MS} ms"
            ),
            tiebreaker: "仅在两条门槛都通过的配置之间，取索引字节最小者".to_string(),
        },
        environment: Environment {
            sqlite_version,
            page_size: PAGE_SIZE,
            repeats_per_query: repeats,
            warmup_per_query: WARMUP,
            reference_machine: reference_machine(),
        },
        corpus: CorpusInfo {
            poem_count: sample.poems.len(),
            provenance: PROVENANCE.to_string(),
            synthesis_seed: SYNTHESIS_SEED,
            distinct_chars: sample.distinct_chars,
            total_body_chars: sample.total_body_chars,
            fixture_poems_embedded: fixtures.poems.len(),
        },
        contract: ContractInfo {
            path: CONTRACT.to_string(),
            schema_version: contract.schema_version,
            entry_count: contract.queries.len(),
            class_count: distinct_classes(&contract).len(),
        },
        results,
        scale_projection: projection,
    };

    write_reports(&root, &report)?;
    emit(&format!("已写出 {REPORT_JSON} 与 {REPORT_MD}"));
    Ok(())
}

fn gate_mark(ok: bool) -> &'static str {
    if ok { "通过" } else { "未通过" }
}

fn reference_machine() -> String {
    // 参考机的标识只用于让报告可比对，不参与任何判定，因此拿不到就写 unknown 而不是失败。
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{os}/{arch}, {cpus} 逻辑核")
}

fn repo_root() -> Result<PathBuf> {
    // `CARGO_MANIFEST_DIR` 指向 `xtask/`，仓库根是它的父目录。用它而不是当前工作目录：
    // `cargo run -p xtask` 的 cwd 可能是任意子目录。
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .context("无法从 xtask/ 推出仓库根目录")?
        .to_path_buf();
    if !root.join(CONTRACT).exists() {
        bail!("在 {} 下找不到契约文件 {CONTRACT}", root.display());
    }
    Ok(root)
}

fn load_contract(root: &Path) -> Result<Contract> {
    let path = root.join(CONTRACT);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读取契约失败 {}", path.display()))?;
    let contract: Contract =
        toml::from_str(&text).with_context(|| format!("解析契约失败 {}", path.display()))?;
    if contract.queries.len() < 30 {
        bail!("契约只有 {} 条，方案要求至少 30 条", contract.queries.len());
    }
    Ok(contract)
}

fn load_fixtures(root: &Path, contract: &Contract) -> Result<Fixtures> {
    let path = root
        .join("crates/yunjian-core/tests")
        .join(&contract.fixture_file);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读取 fixture 失败 {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("解析 fixture 失败 {}", path.display()))
}

fn distinct_classes(contract: &Contract) -> BTreeSet<&str> {
    contract.queries.iter().map(|e| e.class.as_str()).collect()
}

// ---------------------------------------------------------------- 样本语料合成
//
// # 样本从哪来（报告里也会原样记录这段）
//
// **不下载、不 vendor 任何真实语料。** 真实语料入库是 todo 11 / 12 的工作，在它之前
// 把上游数据拉进来会造成两份来源不明的副本。这里合成一份规模足够、字频分布贴近真实
// 汉语的样本，方式是：
//
// 1. **字表与频率**取自随仓 fixture 的正文——那 19 首是真实的公有领域唐宋诗词，
//    其字频就是真实古典汉语的一个（小）样本。合成时按这份实测频率加权抽样，
//    因此高频字（月、山、风、人）在样本里同样高频，而这正是「明月」这类查询选择性
//    极低的原因。用均匀分布会把两字查询的候选集人为压小，实测结论就不成立了。
// 2. **句式**沿用五言 / 七言 / 长短句三种真实形态，按 fixture 里的实际比例。
// 3. **fixture 诗逐字嵌入**，因此契约里每个 `expect_top_id` 在样本中真实存在，
//    契约在样本上是可满足的。
// 4. **确定性**：自带 SplitMix64，固定种子。同一 scale 两次运行产出逐字节相同的样本，
//    所以两次实测的差异只能来自 SQLite 侧，不会来自数据。
//
// 合成语料**不入库**（`.gitignore` 已排除 `*.db` 与 `corpus/build/`）。

const PROVENANCE: &str = "合成样本，非真实语料。字表与字频取自随仓 19 首公有领域 fixture 诗的实测字频（按频率加权抽样，保留真实汉语的长尾分布），句式沿用五言/七言/长短句三种真实形态，19 首 fixture 诗逐字嵌入以保证契约锚点存在。固定种子的 SplitMix64，同规模下逐字节可复现。不下载、不 vendor 任何上游语料——真实语料入库是 todo 11/12 的工作。";

const SYNTHESIS_SEED: u64 = 0x59_75_6E_4A_69_61_6E_01;

/// 固定 `page_size` 是体积可比的前提：页大小不同，`dbstat` 的字节数就没法横向比。
const PAGE_SIZE: i64 = 4096;

struct SamplePoem {
    stable_id: String,
    title: String,
    author: String,
    dynasty: String,
    ci_tune: String,
    body: String,
    first_line: String,
    last_chars: String,
    rhyme_book: String,
    rhyme_group: String,
    tags: String,
}

struct Sample {
    poems: Vec<SamplePoem>,
    distinct_chars: usize,
    total_body_chars: usize,
}

/// SplitMix64。选它而不是引入 `rand`：确定性是这里唯一的要求，而多加一个依赖就要
/// 多一条工作区 pin，且 `rand` 的分布实现跨版本可能变化，那会让「同规模可复现」失效。
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

const PUNCTUATION: &str = "，。！？；：、「」『』《》〈〉（）【】—…,.!?;:'\"()[]{}<>-";

fn is_punct(c: char) -> bool {
    PUNCTUATION.contains(c) || c.is_whitespace() || c == '·'
}

fn synthesize_corpus(scale: usize, fixtures: &Fixtures) -> Sample {
    // 按 fixture 实测字频建加权字池：一个字在 fixture 正文里出现 k 次，就在池里占 k 个位置。
    // 于是均匀抽池 == 按真实字频抽字，长尾分布被完整保留。
    let mut pool: Vec<char> = Vec::new();
    for p in &fixtures.poems {
        pool.extend(p.body.chars().filter(|c| !is_punct(*c)));
    }
    // 只有 19 首诗，字表偏小，会让 trigram 的选择性系统性偏低。补一段常用字扩表，
    // 但**权重压到 1**（每字只占一个位置），因此它只加宽长尾、不改变高频区的形状。
    for c in COMMON_CHARS.chars() {
        pool.push(c);
    }

    let mut rng = Rng(SYNTHESIS_SEED);
    let mut poems = Vec::with_capacity(scale + fixtures.poems.len());

    // fixture 诗先放，逐字嵌入。契约的锚必须在样本里真实存在。
    for p in &fixtures.poems {
        poems.push(SamplePoem {
            stable_id: p.stable_id.clone(),
            title: p.title.clone(),
            author: p.author.clone(),
            dynasty: p.dynasty.clone(),
            ci_tune: p.ci_tune.clone(),
            body: p.body.clone(),
            first_line: p.first_line.clone(),
            last_chars: p.last_chars.join(""),
            rhyme_book: p.rhyme_book.clone(),
            rhyme_group: p.rhyme_group.clone(),
            tags: p.tags.join(","),
        });
    }

    let authors = SYNTHETIC_AUTHORS;
    let dynasties = ["唐", "宋", "元", "明", "清"];
    let remaining = scale.saturating_sub(fixtures.poems.len());
    for i in 0..remaining {
        // 句式分布贴近真实：五言绝句最多，其次七言，长短句（词）最少。
        let shape = match rng.below(10) {
            0..=4 => &[5usize, 5, 5, 5][..],
            5..=8 => &[7usize, 7, 7, 7][..],
            _ => &[4usize, 5, 7, 5, 6][..],
        };
        let mut lines: Vec<String> = Vec::with_capacity(shape.len());
        for len in shape {
            let mut s = String::with_capacity(*len * 3);
            for _ in 0..*len {
                s.push(pool[rng.below(pool.len())]);
            }
            lines.push(s);
        }
        let mut body = String::new();
        for (n, line) in lines.iter().enumerate() {
            body.push_str(line);
            body.push(if n + 1 == lines.len() {
                '。'
            } else if n % 2 == 0 {
                '，'
            } else {
                '。'
            });
        }
        let last_chars: String = lines.iter().filter_map(|l| l.chars().last()).collect();
        let title: String = lines[0].chars().take(3).collect();
        poems.push(SamplePoem {
            stable_id: format!("sample:{i:07}"),
            title,
            author: authors[rng.below(authors.len())].to_string(),
            dynasty: dynasties[rng.below(dynasties.len())].to_string(),
            ci_tune: String::new(),
            body,
            first_line: lines[0].clone(),
            last_chars,
            rhyme_book: String::new(),
            rhyme_group: String::new(),
            tags: String::new(),
        });
    }

    let mut distinct = BTreeSet::new();
    let mut total = 0usize;
    for p in &poems {
        for c in p.body.chars().filter(|c| !is_punct(*c)) {
            distinct.insert(c);
            total += 1;
        }
    }

    Sample {
        poems,
        distinct_chars: distinct.len(),
        total_body_chars: total,
    }
}

/// 扩表用的常用字。权重为 1，只加宽长尾。
///
/// 逐字取自随仓 fixture 之外的常见古典用字，不来自任何第三方数据集——一份字表
/// 谈不上原创性，但仍然按项目的「不引入来源不明数据」惯例手工列出而非抓取。
const COMMON_CHARS: &str = "\
一二三四五六七八九十百千万上下左右前后东西南北中外内高低远近深浅长短大小多少\
天地日月星辰云雨雪风霜露雷电水火土木金石山川江河湖海溪泉潭渚洲岸滩浦湾峰岭\
花草树叶枝根果实苗禾麦稻桑柳松柏梅兰竹菊莲荷桃李杏梨枫桐槐榆椿棠蕉葵藤薇蓼\
春夏秋冬朝暮晨昏昼夜晓晚寒暖凉热清明暗幽静喧寂寥空虚满盈亏残缺全备\
人君臣民士农工商僧道客主宾友朋侣伴妻子女儿孙祖父母兄弟姊妹翁媪叟童\
心思情意志念愁恨怨悲喜欢乐忧惧惊叹嗟吟啸歌哭笑语言辞赋诗词文章书画琴棋\
行坐卧立走奔驰飞跃登临望眺顾盼见闻听嗅触感知觉悟省察观照映射\
衣裳裙袖冠巾履屐杯盏壶樽觞酒茶饭羹肴馔箸案席帷幕帘屏榻枕衾褥\
城郭门户庭院墙壁楼台亭阁轩榭廊庑堂室斋馆寺观宫殿陵墓碑碣桥梁舟船帆桨\
马牛羊犬豕鸡鸭鹅雁鹤鸥鹭鹃莺燕雀鸦鹊鸿鹏凤凰龙虎豹熊鹿猿猴蝉蝶蜂蚁鱼虾蟹龟\
剑刀弓矢戈矛盾甲胄旗鼓钟磬笛箫筝琵瑟笙簧弦管丝竹\
道德仁义礼智信忠孝节廉耻贤圣愚拙巧朴真伪善恶美丑吉凶祸福寿夭穷通贵贱荣辱兴衰\
关塞戍垒烽燧营帐征戍旅途驿邮亭馆程站里程遥迢邈渺茫苍莽荒芜寥廓辽阔浩瀚渺弥\
";

/// 合成诗的作者名。刻意用与真实诗人不同的名字，避免报告里的样本被误读成真实作品。
const SYNTHETIC_AUTHORS: [&str; 12] = [
    "样本甲",
    "样本乙",
    "样本丙",
    "样本丁",
    "样本戊",
    "样本己",
    "样本庚",
    "样本辛",
    "样本壬",
    "样本癸",
    "样本子",
    "样本丑",
];

// ---------------------------------------------------------------- 建库

/// 表结构。刻意与 todo 17 的 `schema.sql` 对齐的**子集**：`poem` 的列名、
/// `poem_fts` 的外部内容配置、`ngram` 的形状都必须与将来真正建出来的一致，
/// 否则实测的数字对产物不成立。
///
/// 不在这里复制整份 schema：韵书、集评、defect 等表不影响索引体积与检索延迟，
/// 把它们搬进来只会让这份 spike 与 todo 17 产生真实的耦合。
const BASE_SCHEMA: &str = "\
CREATE TABLE poem (
  stable_id   TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  author      TEXT NOT NULL,
  dynasty     TEXT NOT NULL,
  ci_tune     TEXT NOT NULL,
  body        TEXT NOT NULL,
  first_line  TEXT NOT NULL,
  last_chars  TEXT NOT NULL,
  rhyme_book  TEXT NOT NULL,
  rhyme_group TEXT NOT NULL,
  tags        TEXT NOT NULL
) STRICT;
CREATE INDEX poem_author_idx     ON poem(author);
CREATE INDEX poem_title_idx      ON poem(title);
CREATE INDEX poem_ci_tune_idx    ON poem(ci_tune);
CREATE INDEX poem_first_line_idx ON poem(first_line);
CREATE INDEX poem_rhyme_idx      ON poem(rhyme_book, rhyme_group);

-- 标签与逐句末字必须是**规范化的多对多表**，不能是 denormalized 字符串列 + `LIKE`。
-- 这一条是本次 spike 实测出来的，不是照抄方案：最初为了省事把它们存成
-- 「以逗号连接的 tags」与「拼接起来的 last_chars」，用 `LIKE '%思乡%'` 查——
-- `EXPLAIN QUERY PLAN` 当场报 `SCAN poem`，外推到发布规模后逼近 150 ms 预算。
-- 也就是说那种存法在 1 万首上看着没事，到 85 万首就不行了。
-- **todo 17 建 schema、todo 26 / 27 写元数据检索时必须用下面这个形状。**
CREATE TABLE poem_tag (
  stable_id TEXT NOT NULL,
  tag       TEXT NOT NULL
) STRICT;
CREATE INDEX poem_tag_idx ON poem_tag(tag, stable_id);

CREATE TABLE poem_last_char (
  stable_id  TEXT NOT NULL,
  line_index INTEGER NOT NULL,
  ch         TEXT NOT NULL
) STRICT;
CREATE INDEX poem_last_char_idx ON poem_last_char(ch, stable_id);
";

fn open_fresh(path: &Path) -> Result<Connection> {
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("删除旧库失败 {}", path.display()))?;
    }
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", path.display()));
        if side.exists() {
            let _ = std::fs::remove_file(side);
        }
    }
    let conn = Connection::open(path).with_context(|| format!("建库失败 {}", path.display()))?;
    // `page_size` 必须在任何表创建之前设定，否则不生效。
    conn.pragma_update(None, "page_size", PAGE_SIZE)?;
    conn.pragma_update(None, "journal_mode", "delete")?;
    conn.pragma_update(None, "synchronous", "off")?;
    Ok(conn)
}

fn build_db(
    path: &Path,
    detail_mode: &str,
    ngram_aux: bool,
    sample: &Sample,
) -> Result<(Connection, u128)> {
    let started = Instant::now();
    let mut conn = open_fresh(path)?;
    conn.execute_batch(BASE_SCHEMA)?;

    {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO poem (stable_id,title,author,dynasty,ci_tune,body,first_line,\
                 last_chars,rhyme_book,rhyme_group,tags) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            )?;
            for p in &sample.poems {
                stmt.execute(params![
                    p.stable_id,
                    p.title,
                    p.author,
                    p.dynasty,
                    p.ci_tune,
                    p.body,
                    p.first_line,
                    p.last_chars,
                    p.rhyme_book,
                    p.rhyme_group,
                    p.tags,
                ])?;
            }
        }
        {
            let mut tag_stmt =
                tx.prepare("INSERT INTO poem_tag (stable_id, tag) VALUES (?1, ?2)")?;
            let mut lc_stmt = tx.prepare(
                "INSERT INTO poem_last_char (stable_id, line_index, ch) VALUES (?1, ?2, ?3)",
            )?;
            for p in &sample.poems {
                for tag in p.tags.split(',').filter(|t| !t.is_empty()) {
                    tag_stmt.execute(params![p.stable_id, tag])?;
                }
                for (i, ch) in p.last_chars.chars().enumerate() {
                    lc_stmt.execute(params![p.stable_id, i as i64, ch.to_string()])?;
                }
            }
        }
        tx.commit()?;
    }

    // 外部内容表 + trigram 分词器。**不开 `remove_diacritics`**：它会让索引 LIKE/GLOB
    // 在 trigram 表上失效，而 LIKE 路径正是长查询与两字回表核对的物理载体。
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE poem_fts USING fts5(
           body,
           content='poem',
           content_rowid='rowid',
           tokenize='trigram',
           detail={detail_mode}
         );
         INSERT INTO poem_fts(poem_fts) VALUES('rebuild');
         INSERT INTO poem_fts(poem_fts) VALUES('optimize');"
    ))?;

    if ngram_aux {
        build_ngram(&mut conn, sample)?;
    }

    conn.execute_batch("PRAGMA integrity_check; ANALYZE; VACUUM;")?;
    Ok((conn, started.elapsed().as_millis()))
}

/// 辅助 n-gram 候选表：`body` 的全部 1-gram 与 2-gram，`gram` 上建覆盖索引。
///
/// 为什么需要它：`%明月%` 只有两个字面字符，FTS5 推不出任何 trigram 约束，
/// 「索引 LIKE」在 1-2 字模式下退化成对整个 body 列的虚表全扫。先用 `gram` 精确
/// 等值查出候选 `stable_id`，再回表 `LIKE` 核对，把扫描量从「全语料」降到「含该
/// 二字组的少数几首」。
///
/// 逐诗去重（同一首诗里「明月」出现两次只记一行），否则表会随重复字组线性膨胀
/// 而候选集并不变小。
fn build_ngram(conn: &mut Connection, sample: &Sample) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE ngram (
           gram      TEXT NOT NULL,
           stable_id TEXT NOT NULL
         ) STRICT;",
    )?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("INSERT INTO ngram (gram, stable_id) VALUES (?1, ?2)")?;
        let mut grams: BTreeSet<String> = BTreeSet::new();
        for p in &sample.poems {
            grams.clear();
            let chars: Vec<char> = p.body.chars().filter(|c| !is_punct(*c)).collect();
            for (i, c) in chars.iter().enumerate() {
                grams.insert(c.to_string());
                if let Some(next) = chars.get(i + 1) {
                    let mut two = String::with_capacity(8);
                    two.push(*c);
                    two.push(*next);
                    grams.insert(two);
                }
            }
            for g in &grams {
                stmt.execute(params![g, p.stable_id])?;
            }
        }
    }
    tx.commit()?;
    // 覆盖索引：两列都在索引里，候选集查询完全不必回 `ngram` 主表。
    conn.execute_batch("CREATE INDEX ngram_gram_idx ON ngram(gram, stable_id);")?;
    Ok(())
}

/// 从 `dbstat` 取逐表/逐索引占用的字节。
///
/// `dbstat` 是虚表，需要 `SQLITE_ENABLE_DBSTAT_VTAB`（`rusqlite` 的 bundled 构建已开）。
/// 用它而不是文件大小差值：文件大小含基表、B-tree 索引与自由页，无法把「索引成本」
/// 单独摘出来，而选型的 tiebreaker 恰好是索引成本。
fn table_bytes(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1).unwrap_or(0)))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (name, bytes) = row?;
        out.insert(name, bytes);
    }
    Ok(out)
}

// ---------------------------------------------------------------- 查询执行
//
// 每条契约按其 `expect_plan` 走一条**具体的 SQL**，而不是「某种检索」。这份映射
// 就是 todo 24 路由规则的可执行形态，报告里逐条记下 `EXPLAIN QUERY PLAN`，
// 因此「索引 LIKE 有没有避开全扫」是有证据的，不是推断出来的。

/// 归一化：剥标点 + 逐字过变体映射。与 `golden_queries.rs` 的参考实现同构。
///
/// `%` 与 `_` 不剥（LIKE 通配符，`no_three_char_run` 类靠它们表达形态），
/// `·` 不剥（「词牌·题目」的结构分隔符，剥了就永远匹配不到合成题目）。
fn normalize(query: &str, variants: &BTreeMap<char, char>) -> String {
    query
        .chars()
        .filter(|c| !is_punct(*c) || *c == '·')
        .map(|c| variants.get(&c).copied().unwrap_or(c))
        .collect()
}

fn variant_map(fixtures: &Fixtures) -> BTreeMap<char, char> {
    let mut map = BTreeMap::new();
    for v in &fixtures.variants {
        if let (Some(from), Some(to)) = (v.from.chars().next(), v.to.chars().next()) {
            map.insert(from, to);
        }
    }
    map
}

/// LIKE 模式里最长的字面连续段有多少字。
fn max_literal_run(pattern: &str) -> usize {
    pattern
        .split(['%', '_'])
        .map(|seg| seg.chars().count())
        .max()
        .unwrap_or(0)
}

/// 一条契约在某个配置下实际要执行的 SQL 与它的物理路径名。
struct Executable {
    plan: &'static str,
    sql: String,
    binds: Vec<String>,
}

/// 把契约条目翻译成可执行 SQL。
///
/// 关键取舍逐条说明：
///
/// - **Ngram**（1-2 字）：有辅助表时走 `ngram` 覆盖索引精确等值 + 回表 LIKE 核对；
///   没有辅助表时**只能**退化成裸 `LIKE` 打在 FTS 虚表的 body 列上——这正是要被测
///   出来的退化，所以不做任何补救。
/// - **Match**（>= 3 字）：`detail=full` 下用 phrase 查询（引号包起来，**不加 `*`**）。
///   `detail != full` 时 FTS5 直接报错，那条错误原文会被记进报告——它就是那个
///   本来会静默上线的缺陷。
/// - **Like**（> 3 字）：`LIKE '%…%'` 打在 FTS 虚表 body 列上，靠 trigram 约束。
///   **不发 `ESCAPE`**：那会让 FTS5 放弃使用索引。
/// - **Empty**：不执行任何检索，恒空。它的意义是「一次误触不能耗掉几秒」。
/// - **FullScan**：显式承认在扫，SQL 打在基表上。
/// - **Meta**：普通 B-tree，绝不碰 FTS 表。
fn plan_sql(
    entry: &ContractEntry,
    normalized: &str,
    detail_mode: &str,
    ngram_aux: bool,
) -> Executable {
    let like = format!("%{normalized}%");
    match entry.expect_plan.as_str() {
        "Empty" => Executable {
            plan: "Empty",
            sql: "SELECT stable_id FROM poem WHERE 0".to_string(),
            binds: vec![],
        },
        "Ngram" if ngram_aux => Executable {
            plan: "Ngram",
            sql: "SELECT p.stable_id FROM ngram n JOIN poem p ON p.stable_id = n.stable_id \
                  WHERE n.gram = ?1 AND p.body LIKE ?2"
                .to_string(),
            binds: vec![normalized.to_string(), like],
        },
        "Ngram" => Executable {
            plan: "BareLikeFts",
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.body LIKE ?1"
                .to_string(),
            binds: vec![like],
        },
        "Match" if detail_mode == "full" => Executable {
            plan: "Match",
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.poem_fts MATCH ?1"
                .to_string(),
            binds: vec![format!("\"{normalized}\"")],
        },
        // detail != full：phrase 查询不被支持。仍然照发，让 FTS5 自己报错并把原文记下来——
        // 这比我们替它判断更可信，也正是本次 spike 要证明的那个陷阱。
        "Match" => Executable {
            plan: "MatchUnsupported",
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.poem_fts MATCH ?1"
                .to_string(),
            binds: vec![format!("\"{normalized}\"")],
        },
        // 路由按**查询形态**判定，不盲信契约声明的计划：没有长度 >= 3 的字面连续段时，
        // trigram 推不出任何约束，那就是一次全扫，必须如实走 FullScan 并如实记录。
        // 这样一条契约若把计划写错，报告会显示 `expect_plan` 与 `executed_plan` 不一致，
        // 而不是让错误的声明决定实测怎么跑。
        "Like" if max_literal_run(normalized) >= 3 => Executable {
            plan: "Like",
            sql: "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                  WHERE f.body LIKE ?1"
                .to_string(),
            binds: vec![like],
        },
        "Like" => Executable {
            plan: "FullScanFallback",
            sql: "SELECT stable_id FROM poem WHERE body LIKE ?1".to_string(),
            binds: vec![like],
        },
        "FullScan" => Executable {
            plan: "FullScan",
            sql: "SELECT stable_id FROM poem WHERE body LIKE ?1".to_string(),
            binds: vec![like],
        },
        "Meta" => meta_sql(entry, normalized),
        other => Executable {
            plan: "Unknown",
            sql: format!("SELECT stable_id FROM poem WHERE 0 -- 未知计划 {other}"),
            binds: vec![],
        },
    }
}

fn meta_sql(entry: &ContractEntry, normalized: &str) -> Executable {
    let (sql, binds): (&str, Vec<String>) = match entry.class.as_str() {
        "two_char_author" => (
            "SELECT stable_id FROM poem WHERE author = ?1",
            vec![normalized.to_string()],
        ),
        "title_lookup" | "ci_tune_title_lookup" => (
            "SELECT stable_id FROM poem WHERE title = ?1",
            vec![normalized.to_string()],
        ),
        "ci_tune_lookup" => (
            "SELECT stable_id FROM poem WHERE ci_tune = ?1",
            vec![normalized.to_string()],
        ),
        // 首句前缀用 `>= ?1 AND < ?2` 而不是 `LIKE 'x%'`：前者能用上 B-tree 的有序性
        // 且与排序规则无关，后者在某些 collation 下会退化成扫描。
        "first_line_prefix" => (
            "SELECT stable_id FROM poem WHERE first_line >= ?1 AND first_line < ?2",
            vec![normalized.to_string(), prefix_upper_bound(normalized)],
        ),
        // 尾字打在预计算的逐句末字上，语义是「某句以此字收尾」，而不是「正文含此字」。
        // 走规范化表的等值查询，因此吃得到索引；`DISTINCT` 是因为一首诗可能有多句
        // 以同一个字收尾（律诗的韵脚就常常如此），而检索结果的单位是「诗」。
        "last_char_lookup" => (
            "SELECT DISTINCT stable_id FROM poem_last_char WHERE ch = ?1",
            vec![normalized.to_string()],
        ),
        "rhyme_group_query" => (
            "SELECT stable_id FROM poem WHERE rhyme_group = ?1",
            vec![normalized.to_string()],
        ),
        "tag_query" => (
            "SELECT stable_id FROM poem_tag WHERE tag = ?1",
            vec![normalized.to_string()],
        ),
        _ => ("SELECT stable_id FROM poem WHERE 0", vec![]),
    };
    Executable {
        plan: "Meta",
        sql: sql.to_string(),
        binds,
    }
}

/// 前缀区间的上界：把最后一个字符加一。用于把前缀匹配变成有序区间扫描。
fn prefix_upper_bound(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    match chars.pop() {
        None => String::new(),
        Some(last) => {
            let bumped = char::from_u32(last as u32 + 1).unwrap_or(last);
            chars.push(bumped);
            chars.into_iter().collect()
        }
    }
}

fn bind_refs(binds: &[String]) -> Vec<&dyn rusqlite::ToSql> {
    binds.iter().map(|b| b as &dyn rusqlite::ToSql).collect()
}

fn run_once(conn: &Connection, exe: &Executable) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&exe.sql)?;
    let rows = stmt.query_map(bind_refs(&exe.binds).as_slice(), |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn explain(conn: &Connection, exe: &Executable) -> Vec<String> {
    let sql = format!("EXPLAIN QUERY PLAN {}", exe.sql);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return vec!["EXPLAIN QUERY PLAN 无法准备（查询本身不被支持）".to_string()];
    };
    let Ok(rows) = stmt.query_map(bind_refs(&exe.binds).as_slice(), |r| r.get::<_, String>(3))
    else {
        return vec!["EXPLAIN QUERY PLAN 无法执行".to_string()];
    };
    rows.filter_map(Result::ok).collect()
}

/// 百分位。`ceil(p * n) - 1` 的最近秩定义：n=25、p=0.95 时取第 24 个样本（0 基下标 23），
/// 即「95% 的观测不慢于它」。用最近秩而不是插值，因为插值出的数字在样本里并不存在，
/// 而门禁判定的应当是真实观测到的延迟。
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)]
}

fn measure(
    conn: &Connection,
    exe: &Executable,
    repeats: usize,
) -> (Vec<f64>, Option<Vec<String>>, Option<String>) {
    let mut error = None;
    let mut rows = None;
    for _ in 0..WARMUP {
        match run_once(conn, exe) {
            Ok(r) => rows = Some(r),
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    if error.is_some() {
        return (vec![0.0; repeats], None, error);
    }
    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let t = Instant::now();
        match run_once(conn, exe) {
            Ok(r) => {
                samples.push(t.elapsed().as_secs_f64() * 1000.0);
                rows = Some(r);
            }
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    if error.is_some() {
        return (vec![0.0; repeats], None, error);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (samples, rows, None)
}

// ---------------------------------------------------------------- 逐配置实测

fn measure_config(
    build_dir: &Path,
    detail_mode: &str,
    ngram_aux: bool,
    sample: &Sample,
    contract: &Contract,
    fixtures: &Fixtures,
    repeats: usize,
) -> Result<ConfigResult> {
    let config_id = format!(
        "detail={detail_mode}+ngram={}",
        if ngram_aux { "on" } else { "off" }
    );
    let db = build_dir.join(format!(
        "spike-{detail_mode}-{}.db",
        if ngram_aux { "ngram" } else { "plain" }
    ));
    let (conn, build_ms) = build_db(&db, detail_mode, ngram_aux, sample)?;

    let bytes = table_bytes(&conn)?;
    let fts_bytes: i64 = bytes
        .iter()
        .filter(|(name, _)| name.starts_with("poem_fts"))
        .map(|(_, b)| *b)
        .sum();
    let ngram_bytes: i64 = bytes
        .iter()
        .filter(|(name, _)| name.starts_with("ngram"))
        .map(|(_, b)| *b)
        .sum();
    let ngram_rows: i64 = if ngram_aux {
        conn.query_row("SELECT count(*) FROM ngram", [], |r| r.get(0))?
    } else {
        0
    };

    // 测量前把库重开为只读：产物是只读语料库（todo 23），带写权限的连接在
    // SQLite 里走的锁路径不同，测出来的延迟对产物不成立。
    drop(conn);
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("以只读方式打开失败 {}", db.display()))?;
    conn.pragma_update(None, "query_only", true)?;

    let variants = variant_map(fixtures);
    let mut queries = Vec::with_capacity(contract.queries.len());
    for e in &contract.queries {
        let normalized = normalize(&e.query, &variants);
        let exe = plan_sql(e, &normalized, detail_mode, ngram_aux);
        let eqp = explain(&conn, &exe);
        let (samples, rows, error) = measure(&conn, &exe, repeats);
        let hits = rows.as_ref().map(Vec::len).unwrap_or(0);
        let anchor_found = rows
            .as_ref()
            .map(|r| r.contains(&e.expect_top_id))
            .unwrap_or(false);
        queries.push(QueryResult {
            id: e.id.clone(),
            class: e.class.clone(),
            query: e.query.clone(),
            normalized,
            expect_plan: e.expect_plan.clone(),
            executed_plan: exe.plan.to_string(),
            expect_min_hits: e.expect_min_hits,
            hits,
            meets_min_hits: hits >= e.expect_min_hits,
            anchor_found,
            p95_ms: round3(percentile(&samples, 0.95)),
            median_ms: round3(percentile(&samples, 0.50)),
            explain_query_plan: eqp,
            error,
        });
    }

    let hits_shortfall: Vec<Shortfall> = queries
        .iter()
        .filter(|q| !q.meets_min_hits)
        .map(|q| Shortfall {
            id: q.id.clone(),
            class: q.class.clone(),
            expected_min_hits: q.expect_min_hits,
            actual_hits: q.hits,
            reason: q
                .error
                .clone()
                .unwrap_or_else(|| "查询成功执行但召回不足".to_string()),
        })
        .collect();
    let latency_violations: Vec<LatencyViolation> = queries
        .iter()
        .filter(|q| q.p95_ms > P95_BUDGET_MS)
        .map(|q| LatencyViolation {
            id: q.id.clone(),
            class: q.class.clone(),
            p95_ms: q.p95_ms,
        })
        .collect();

    // 契约**自己**声明为 `FullScan` 的条目不受延迟门槛约束。方案要求这类计划被显式
    // 标记「以便调用方提示用户，而不是静默耗掉几秒」——也就是说慢是它已被承认的属性，
    // 不是缺陷。拿 150 ms 去卡它，等于要求一个按定义无索引可用的形态跑出索引的速度，
    // 那会让六种配置全部落选、规则一条也筛不出来。它们仍然逐条记在
    // `acknowledged_full_scans` 里，因此是豁免而不是隐藏。
    let projected_latency_violations: Vec<ProjectedViolation> = queries
        .iter()
        .filter(|q| q.expect_plan != "FullScan")
        .filter_map(|q| {
            let projected = project_to_production(q, sample.poems.len());
            (projected > P95_BUDGET_MS).then(|| ProjectedViolation {
                id: q.id.clone(),
                class: q.class.clone(),
                executed_plan: q.executed_plan.clone(),
                measured_p95_ms: q.p95_ms,
                projected_p95_ms: round3(projected),
            })
        })
        .collect();
    // 阈值取预算的一半：足够宽，不会把索引定位型路径（外推后仍是零点几毫秒）误报进来；
    // 又足够严，能抓住任何靠全扫勉强达标的条目。
    let near_miss_floor = P95_BUDGET_MS / 2.0;
    let projected_near_misses: Vec<ProjectedViolation> = queries
        .iter()
        .filter(|q| q.expect_plan != "FullScan")
        .filter_map(|q| {
            let projected = project_to_production(q, sample.poems.len());
            (projected > near_miss_floor && projected <= P95_BUDGET_MS).then(|| {
                ProjectedViolation {
                    id: q.id.clone(),
                    class: q.class.clone(),
                    executed_plan: q.executed_plan.clone(),
                    measured_p95_ms: q.p95_ms,
                    projected_p95_ms: round3(projected),
                }
            })
        })
        .collect();
    let acknowledged_full_scans: Vec<ProjectedViolation> = queries
        .iter()
        .filter(|q| q.expect_plan == "FullScan")
        .map(|q| ProjectedViolation {
            id: q.id.clone(),
            class: q.class.clone(),
            executed_plan: q.executed_plan.clone(),
            measured_p95_ms: q.p95_ms,
            projected_p95_ms: round3(project_to_production(q, sample.poems.len())),
        })
        .collect();

    let mut per_class: BTreeMap<&str, (usize, usize, f64)> = BTreeMap::new();
    for q in &queries {
        let slot = per_class.entry(q.class.as_str()).or_insert((0, 0, 0.0));
        slot.0 += 1;
        if q.meets_min_hits {
            slot.1 += 1;
        }
        if q.p95_ms > slot.2 {
            slot.2 = q.p95_ms;
        }
    }

    Ok(ConfigResult {
        config_id,
        detail_mode: detail_mode.to_string(),
        ngram_aux,
        index_bytes: fts_bytes + ngram_bytes,
        fts_bytes,
        ngram_bytes,
        ngram_rows,
        total_file_bytes: std::fs::metadata(&db)?.len(),
        build_ms,
        passes_hits_gate: hits_shortfall.is_empty(),
        passes_latency_gate: latency_violations.is_empty(),
        passes_projected_latency_gate: projected_latency_violations.is_empty(),
        hits_shortfall,
        latency_violations,
        projected_latency_violations,
        acknowledged_full_scans,
        projected_near_misses,
        per_class: per_class
            .into_iter()
            .map(|(class, (entries, ok, worst))| ClassSummary {
                class: class.to_string(),
                entries,
                entries_meeting_min_hits: ok,
                hit_rate: round3(ok as f64 / entries as f64),
                worst_p95_ms: round3(worst),
            })
            .collect(),
        queries,
    })
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// 按定义无法从模式推出约束的路径名。
///
/// `BareLikeFts` 打在 1-2 字模式上时，FTS5 的 `EXPLAIN QUERY PLAN` **仍然报 `L0`**
/// （它接受了 LIKE 约束），但两个字面字符推不出任何 trigram，内部实际扫遍整列。
/// 也就是说这一种退化**看 EQP 是看不出来的**，只能靠路径名识别——这正是本次 spike
/// 最初要测的那个陷阱。
const UNCONSTRAINED_PLANS: [&str; 3] = ["BareLikeFts", "FullScan", "FullScanFallback"];

/// 这条查询的成本是否正比于语料行数。
///
/// 两个来源，缺一不可：
///
/// 1. **实测到的 `EXPLAIN QUERY PLAN` 里有基表全扫**（`SCAN poem`，且不带
///    `USING ... INDEX ... (...=?)` 约束）。靠 EQP 而不是靠计划名判断，是因为一条
///    被声明为 `Meta` 的查询完全可能因为列的存法而退化成全扫——本次实测里
///    尾字与标签两类就是如此（它们打在 denormalized 的字符串列上）。**只按计划名
///    判断会把这类查询漏掉**，而它们恰好是「召回对但靠扫全表」的典型。
/// 2. **路径名属于 `UNCONSTRAINED_PLANS`**。补上 EQP 看不出来的那一类退化。
///
/// 索引定位型路径原样返回：成本随规模是对数级，线性外推会严重高估。这个区分是保守的，
/// 只会让扫描型配置更容易被否掉。
fn is_scan_bound(q: &QueryResult) -> bool {
    if UNCONSTRAINED_PLANS.contains(&q.executed_plan.as_str()) {
        return true;
    }
    q.explain_query_plan
        .iter()
        .any(|line| line.trim_start().starts_with("SCAN poem") && !line.contains("COVERING INDEX"))
}

/// 把样本规模上实测的 p95 外推到发布规模。
fn project_to_production(q: &QueryResult, sample_size: usize) -> f64 {
    if !is_scan_bound(q) || sample_size == 0 {
        return q.p95_ms;
    }
    q.p95_ms * (PRODUCTION_SCALE as f64 / sample_size as f64)
}

// ------------------------------------------------------- 规模投射（n-gram 收益）
//
// 10k 一个点看不出「裸 LIKE 随规模线性增长、n-gram 路径基本持平」这条关键差异，
// 而真实语料是 85 万首。三个规模点足以确定增长的形状，同时不至于让 spike 跑上几分钟。
// 固定在 `detail=full` 上跑：LIKE 走的是 trigram 表的 body 列，与位置信息无关，
// 所以这条曲线对三种 detail 模式是同一条。

fn measure_scale_projection(
    build_dir: &Path,
    fixtures: &Fixtures,
    repeats: usize,
) -> Result<Vec<ScalePoint>> {
    let mut out = Vec::new();
    for scale in PROJECTION_SCALES {
        let sample = synthesize_corpus(scale, fixtures);
        let db = build_dir.join(format!("projection-{scale}.db"));
        let (conn, _) = build_db(&db, "full", true, &sample)?;
        let ngram_rows: i64 = conn.query_row("SELECT count(*) FROM ngram", [], |r| r.get(0))?;
        let ngram_bytes: i64 = table_bytes(&conn)?
            .iter()
            .filter(|(name, _)| name.starts_with("ngram"))
            .map(|(_, b)| *b)
            .sum();
        drop(conn);
        let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        let probes = [
            (
                "ngram",
                "SELECT p.stable_id FROM ngram n JOIN poem p ON p.stable_id = n.stable_id \
                 WHERE n.gram = ?1 AND p.body LIKE ?2",
                vec!["明月".to_string(), "%明月%".to_string()],
            ),
            (
                "bare_like_fts",
                "SELECT p.stable_id FROM poem_fts f JOIN poem p ON p.rowid = f.rowid \
                 WHERE f.body LIKE ?1",
                vec!["%明月%".to_string()],
            ),
            (
                "bare_like_base",
                "SELECT stable_id FROM poem WHERE body LIKE ?1",
                vec!["%明月%".to_string()],
            ),
        ];
        let mut p95 = BTreeMap::new();
        for (name, sql, binds) in probes {
            let exe = Executable {
                plan: name,
                sql: sql.to_string(),
                binds,
            };
            let (samples, _, err) = measure(&conn, &exe, repeats);
            if let Some(e) = err {
                bail!("规模投射 {scale} 的 {name} 探针失败：{e}");
            }
            p95.insert(name, round3(percentile(&samples, 0.95)));
        }
        let ngram_p95 = p95["ngram"];
        let bare_fts = p95["bare_like_fts"];
        out.push(ScalePoint {
            poem_count: sample.poems.len(),
            ngram_path_p95_ms: ngram_p95,
            bare_like_fts_p95_ms: bare_fts,
            bare_like_base_p95_ms: p95["bare_like_base"],
            speedup: if ngram_p95 > 0.0 {
                round3(bare_fts / ngram_p95)
            } else {
                0.0
            },
            ngram_rows,
            ngram_bytes,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- 选型与报告

/// 应用事先声明的选型规则。
///
/// 两条硬门槛都通过的配置里取索引字节最小者：
///
/// 1. 每一条契约都达到 `expect_min_hits`；
/// 2. 每一条的 p95 <= 150 ms，**且外推到发布规模后依然 <= 150 ms**。
///
/// 第二条为什么要带外推：方案原话是「一个靠扫 85 万行拿到正确答案的配置要被否掉」。
/// 在 10k 样本上裸 LIKE 只要几毫秒，六种配置全都能过 150 ms——**只看样本规模，
/// 那句规则就一条也筛不掉，等于让抽样规模替产品做决定**。
///
/// **体积只是 tiebreaker**，仅在两条硬门槛都通过时才参与比较。
fn choose<'a>(
    results: &'a [ConfigResult],
    projection: &[ScalePoint],
) -> Result<(&'a ConfigResult, String)> {
    let mut passing: Vec<&ConfigResult> = results
        .iter()
        .filter(|r| r.passes_hits_gate && r.passes_latency_gate && r.passes_projected_latency_gate)
        .collect();
    if passing.is_empty() {
        let mut msg = String::from("没有任何配置同时通过两条门槛，选型规则无法给出结论：\n");
        for r in results {
            let _ = writeln!(
                msg,
                "  {}：命中门槛 {}（缺 {} 条），延迟门槛 {}（超 {} 条），\
                 发布规模延迟门槛 {}（超 {} 条）",
                r.config_id,
                gate_mark(r.passes_hits_gate),
                r.hits_shortfall.len(),
                gate_mark(r.passes_latency_gate),
                r.latency_violations.len(),
                gate_mark(r.passes_projected_latency_gate),
                r.projected_latency_violations.len()
            );
        }
        bail!(msg);
    }
    passing.sort_by_key(|r| (r.index_bytes, r.total_file_bytes));
    let chosen = passing[0];

    // 理由里必须带上被否掉配置的具体原因，否则读报告的人无法判断这个结论是不是被推翻过。
    let rejected_none: Vec<&ConfigResult> = results
        .iter()
        .filter(|r| r.detail_mode == "none" && !r.passes_hits_gate)
        .collect();
    let none_reason = rejected_none
        .first()
        .and_then(|r| r.hits_shortfall.first())
        .map(|s| format!("detail=none 在 {} 类上召回不足（{}）", s.class, s.reason))
        .unwrap_or_else(|| "detail=none 未出现召回缺口".to_string());
    let ngram_gain = projection
        .last()
        .map(|p| {
            format!(
                "n-gram 表在 {} 首规模上把两字查询的 p95 从 {} ms 降到 {} ms（{:.1}x）",
                p.poem_count, p.bare_like_fts_p95_ms, p.ngram_path_p95_ms, p.speedup
            )
        })
        .unwrap_or_default();

    // 被延迟门槛（而非召回门槛）否掉的配置要单独点名：它们召回全对，只是靠扫全表拿到的，
    // 这正是选型规则里「一个靠扫 85 万行拿到正确答案的配置要被否掉」那一句针对的情形。
    let scan_rejected: Vec<String> = results
        .iter()
        .filter(|r| r.passes_hits_gate && !r.passes_projected_latency_gate)
        .map(|r| {
            let worst = r
                .projected_latency_violations
                .iter()
                .max_by(|a, b| {
                    a.projected_p95_ms
                        .partial_cmp(&b.projected_p95_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|v| {
                    format!(
                        "{} 走 {} 在发布规模下外推为 {:.1} ms",
                        v.id, v.executed_plan, v.projected_p95_ms
                    )
                })
                .unwrap_or_default();
            format!("{} 召回全对但被延迟门槛否掉（{worst}）", r.config_id)
        })
        .collect();
    let scan_reason = if scan_rejected.is_empty() {
        "没有配置因发布规模延迟被否掉".to_string()
    } else {
        scan_rejected.join("；")
    };

    let justification = format!(
        "{} 是同时通过「每条契约达到 expect_min_hits」与「每条 p95 <= {P95_BUDGET_MS} ms\
         （含外推到 {PRODUCTION_SCALE} 首发布规模）」两条硬门槛的配置中索引字节最小者（{} B）；\
         {none_reason}；{scan_reason}；{ngram_gain}。",
        chosen.config_id, chosen.index_bytes
    );
    Ok((chosen, justification))
}

fn write_reports(root: &Path, report: &Report) -> Result<()> {
    let json_path = root.join(REPORT_JSON);
    if let Some(dir) = json_path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("创建报告目录失败 {}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&json_path, format!("{json}\n"))
        .with_context(|| format!("写出 {REPORT_JSON} 失败"))?;
    std::fs::write(root.join(REPORT_MD), render_markdown(report))
        .with_context(|| format!("写出 {REPORT_MD} 失败"))?;
    Ok(())
}

fn render_markdown(r: &Report) -> String {
    let mut m = String::new();
    let _ = writeln!(m, "# FTS5 索引模式实测报告\n");
    let _ = writeln!(
        m,
        "> 本文件由 `cargo run -p xtask -- index-spike` 生成，**不要手工编辑**。\
         机器可读版本在 `{REPORT_JSON}`，todo 19 与 24 读的是那一份：建出来的索引与\
         结论不符时构建应当失败。\n"
    );

    let _ = writeln!(m, "## 结论\n");
    let _ = writeln!(m, "| 项 | 值 |");
    let _ = writeln!(m, "|---|---|");
    let _ = writeln!(m, "| 选定 `detail` 模式 | **`{}`** |", r.chosen_mode);
    let _ = writeln!(
        m,
        "| 辅助 n-gram 候选表 | **{}** |",
        if r.ngram_aux_enabled {
            "启用"
        } else {
            "不启用"
        }
    );
    let _ = writeln!(m, "| 理由 | {} |", r.justification);
    let _ = writeln!(m);

    let _ = writeln!(m, "## 选型规则（事先声明、具有约束力）\n");
    let _ = writeln!(
        m,
        "选满足下面两条的**最小**配置，体积只在两条都通过时作为 tiebreaker：\n"
    );
    let _ = writeln!(m, "1. {}；", r.selection_rule.hits_gate);
    let _ = writeln!(m, "2. {}。\n", r.selection_rule.latency_gate);
    let _ = writeln!(
        m,
        "第 2 条**含外推到 {} 首的发布规模**。只在 10k 样本上判定的话，六种配置全都能过 \
         150 ms——那句规则一条也筛不掉，等于让抽样规模替产品做决定。外推只对扫描型路径\
         （`BareLikeFts` / `FullScan` / `FullScanFallback`）按规模线性放大，索引定位型路径\
         原样保留，因此这个判定是保守的：只会让扫描型配置更容易被否掉。\n",
        r.selection_rule.production_scale
    );
    let _ = writeln!(
        m,
        "Tiebreaker：{}。一个靠扫全表拿到正确答案的配置即便体积最小也要被否掉。\n",
        r.selection_rule.tiebreaker
    );

    let _ = writeln!(m, "## 实测环境与样本\n");
    let _ = writeln!(m, "| 项 | 值 |");
    let _ = writeln!(m, "|---|---|");
    let _ = writeln!(m, "| SQLite | {} |", r.environment.sqlite_version);
    let _ = writeln!(m, "| `page_size` | {} |", r.environment.page_size);
    let _ = writeln!(m, "| 参考机 | {} |", r.environment.reference_machine);
    let _ = writeln!(
        m,
        "| 每条查询测量 | {} 轮预热 + {} 轮计时 |",
        r.environment.warmup_per_query, r.environment.repeats_per_query
    );
    let _ = writeln!(m, "| 样本首数 | {} |", r.corpus.poem_count);
    let _ = writeln!(m, "| 不同汉字 | {} |", r.corpus.distinct_chars);
    let _ = writeln!(m, "| 正文总字数 | {} |", r.corpus.total_body_chars);
    let _ = writeln!(
        m,
        "| 嵌入的 fixture 诗 | {} 首 |",
        r.corpus.fixture_poems_embedded
    );
    let _ = writeln!(m, "| 合成种子 | `{:#x}` |", r.corpus.synthesis_seed);
    let _ = writeln!(m);
    let _ = writeln!(m, "**样本来源**：{}\n", r.corpus.provenance);
    let _ = writeln!(
        m,
        "**契约**：`{}`，schema v{}，{} 条，{} 类。\n",
        r.contract.path, r.contract.schema_version, r.contract.entry_count, r.contract.class_count
    );

    let _ = writeln!(m, "## 六种配置的实测对照\n");
    let _ = writeln!(
        m,
        "| 配置 | 索引字节 | 其中 FTS | 其中 n-gram | n-gram 行数 | 文件字节 | 建库 ms | 命中门槛 | 样本延迟门槛 | 发布规模延迟门槛 | 最差 p95 | 最差外推 p95 |"
    );
    let _ = writeln!(m, "|---|---|---|---|---|---|---|---|---|---|---|---|");
    for c in &r.results {
        let worst = c.queries.iter().map(|q| q.p95_ms).fold(0.0_f64, f64::max);
        let worst_projected = c
            .projected_latency_violations
            .iter()
            .map(|v| v.projected_p95_ms)
            .fold(worst, f64::max);
        let _ = writeln!(
            m,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.3} ms | {:.1} ms |",
            c.config_id,
            c.index_bytes,
            c.fts_bytes,
            c.ngram_bytes,
            c.ngram_rows,
            c.total_file_bytes,
            c.build_ms,
            gate_mark(c.passes_hits_gate),
            gate_mark(c.passes_latency_gate),
            gate_mark(c.passes_projected_latency_gate),
            worst,
            worst_projected
        );
    }
    let _ = writeln!(m);

    let _ = writeln!(m, "## 召回缺口逐条\n");
    let _ = writeln!(
        m,
        "这一节是本次 spike 的核心产出：它记录了**本来会静默上线的缺陷**。\n"
    );
    for c in &r.results {
        if c.hits_shortfall.is_empty() {
            let _ = writeln!(m, "### `{}`\n\n无召回缺口。\n", c.config_id);
            continue;
        }
        let _ = writeln!(
            m,
            "### `{}`\n\n缺 {} 条：\n",
            c.config_id,
            c.hits_shortfall.len()
        );
        let _ = writeln!(m, "| 契约 id | 类别 | 期望下界 | 实际 | 原因 |");
        let _ = writeln!(m, "|---|---|---|---|---|");
        for s in &c.hits_shortfall {
            let _ = writeln!(
                m,
                "| `{}` | {} | {} | {} | {} |",
                s.id, s.class, s.expected_min_hits, s.actual_hits, s.reason
            );
        }
        let _ = writeln!(m);
    }

    let _ = writeln!(m, "## 延迟违规逐条\n");
    let _ = writeln!(m, "### 样本规模（{} 首）实测\n", r.corpus.poem_count);
    for c in &r.results {
        if c.latency_violations.is_empty() {
            let _ = writeln!(
                m,
                "- `{}`：无条目超出 {} ms 预算。",
                c.config_id, r.selection_rule.p95_budget_ms
            );
            continue;
        }
        let _ = writeln!(m, "- `{}`：", c.config_id);
        for v in &c.latency_violations {
            let _ = writeln!(m, "  - `{}`（{}）p95 {:.3} ms", v.id, v.class, v.p95_ms);
        }
    }
    let _ = writeln!(m);
    let _ = writeln!(
        m,
        "### 外推到发布规模（{} 首）\n\n这一节才是选型规则里那条延迟门槛的判据。\n",
        r.selection_rule.production_scale
    );
    for c in &r.results {
        if c.projected_latency_violations.is_empty() {
            let _ = writeln!(m, "- `{}`：无条目在发布规模下超出预算。", c.config_id);
            continue;
        }
        let _ = writeln!(m, "- `{}`：", c.config_id);
        for v in &c.projected_latency_violations {
            let _ = writeln!(
                m,
                "  - `{}`（{}）走 `{}`，样本实测 {:.3} ms，外推 **{:.1} ms**",
                v.id, v.class, v.executed_plan, v.measured_p95_ms, v.projected_p95_ms
            );
        }
    }
    let _ = writeln!(m);

    let _ = writeln!(
        m,
        "### 勉强达标的条目（外推后 > {:.0} ms 但 <= {:.0} ms）\n\n这些条目形式上通过了\
         延迟门槛，但它们之所以接近预算，是因为实测走了基表全扫。列出来是为了不让一个\
         已知会在真实规模上吃紧的实现细节被「全部通过」四个字盖住——**todo 17 与 26 必须\
         为它们改用规范化的多对多表（`poem_tag` / 逐句末字表），而不是沿用本 spike 里\
         为了简化而采用的 denormalized 字符串列 + `LIKE`。**\n",
        r.selection_rule.p95_budget_ms / 2.0,
        r.selection_rule.p95_budget_ms
    );
    if let Some(chosen) = r
        .results
        .iter()
        .find(|c| c.detail_mode == r.chosen_mode && c.ngram_aux == r.ngram_aux_enabled)
    {
        if chosen.projected_near_misses.is_empty() {
            let _ = writeln!(m, "无。\n");
        } else {
            let _ = writeln!(m, "| 契约 id | 类别 | 实际路径 | 样本 p95 | 发布规模外推 |");
            let _ = writeln!(m, "|---|---|---|---|---|");
            for v in &chosen.projected_near_misses {
                let _ = writeln!(
                    m,
                    "| `{}` | {} | `{}` | {:.3} ms | **{:.1} ms** |",
                    v.id, v.class, v.executed_plan, v.measured_p95_ms, v.projected_p95_ms
                );
            }
            let _ = writeln!(m);
        }
    }

    let _ = writeln!(
        m,
        "### 被延迟门槛豁免的 `FullScan` 条目\n\n契约自己声明为 `FullScan` 的形态按定义\
         无索引可用，方案要求把它显式标记出来「以便调用方提示用户，而不是静默耗掉几秒」——\
         慢是它已被承认的属性，不是缺陷。逐条记在此处，使豁免可见。\n"
    );
    if let Some(chosen) = r
        .results
        .iter()
        .find(|c| c.detail_mode == r.chosen_mode && c.ngram_aux == r.ngram_aux_enabled)
    {
        if chosen.acknowledged_full_scans.is_empty() {
            let _ = writeln!(m, "无。\n");
        } else {
            let _ = writeln!(m, "| 契约 id | 类别 | 实际路径 | 样本 p95 | 发布规模外推 |");
            let _ = writeln!(m, "|---|---|---|---|---|");
            for v in &chosen.acknowledged_full_scans {
                let _ = writeln!(
                    m,
                    "| `{}` | {} | `{}` | {:.3} ms | {:.1} ms |",
                    v.id, v.class, v.executed_plan, v.measured_p95_ms, v.projected_p95_ms
                );
            }
            let _ = writeln!(m);
        }
    }

    let _ = writeln!(m, "## 逐类命中率\n");
    let _ = writeln!(
        m,
        "| 类别 | 条数 | {} |",
        r.results
            .iter()
            .map(|c| format!("`{}`", c.config_id))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let _ = writeln!(
        m,
        "|---|---|{}",
        r.results.iter().map(|_| "---|").collect::<String>()
    );
    let classes: BTreeSet<&str> = r
        .results
        .iter()
        .flat_map(|c| c.per_class.iter().map(|s| s.class.as_str()))
        .collect();
    for class in &classes {
        let entries = r
            .results
            .first()
            .and_then(|c| c.per_class.iter().find(|s| s.class == *class))
            .map(|s| s.entries)
            .unwrap_or(0);
        let cells: Vec<String> = r
            .results
            .iter()
            .map(|c| {
                c.per_class
                    .iter()
                    .find(|s| s.class == *class)
                    .map(|s| {
                        format!(
                            "{}/{}（最差 p95 {:.3} ms）",
                            s.entries_meeting_min_hits, s.entries, s.worst_p95_ms
                        )
                    })
                    .unwrap_or_else(|| "—".to_string())
            })
            .collect();
        let _ = writeln!(m, "| {} | {} | {} |", class, entries, cells.join(" | "));
    }
    let _ = writeln!(m);

    let _ = writeln!(m, "## 两字查询的 n-gram 收益与规模投射\n");
    let _ = writeln!(
        m,
        "`%明月%` 只有两个字面字符，FTS5 推不出任何 trigram 约束，「索引 LIKE」\
         因此退化成对整个 body 列的虚表全扫——用户最常输入的形态反而最慢。\
         下表是同一条查询在三条物理路径上的 p95。\n"
    );
    let _ = writeln!(
        m,
        "| 样本首数 | 走 n-gram 候选表 | 裸 LIKE（FTS 虚表） | 裸 LIKE（基表） | 加速 | n-gram 行数 | n-gram 字节 |"
    );
    let _ = writeln!(m, "|---|---|---|---|---|---|---|");
    for p in &r.scale_projection {
        let _ = writeln!(
            m,
            "| {} | {:.3} ms | {:.3} ms | {:.3} ms | {:.1}x | {} | {} |",
            p.poem_count,
            p.ngram_path_p95_ms,
            p.bare_like_fts_p95_ms,
            p.bare_like_base_p95_ms,
            p.speedup,
            p.ngram_rows,
            p.ngram_bytes
        );
    }
    let _ = writeln!(m);

    let _ = writeln!(m, "## `EXPLAIN QUERY PLAN` 证据\n");
    let _ = writeln!(
        m,
        "方案禁止在没有 `EXPLAIN QUERY PLAN` 的情况下声称一条 LIKE 路径是「索引化」的。\
         下面按选定配置逐条列出。`SCAN … VIRTUAL TABLE INDEX 0:L0` 里的 `L0` 是 FTS5 \
         接受了 LIKE 约束的标记，`M1` 是 MATCH 约束，两者都不是无约束全扫；\
         而打在基表上的 `SCAN poem` 才是真正的全表扫描。\n"
    );
    if let Some(chosen) = r
        .results
        .iter()
        .find(|c| c.detail_mode == r.chosen_mode && c.ngram_aux == r.ngram_aux_enabled)
    {
        let _ = writeln!(
            m,
            "| 契约 id | 归一化后 | 期望计划 | 实际路径 | 命中 | p95 | EXPLAIN QUERY PLAN |"
        );
        let _ = writeln!(m, "|---|---|---|---|---|---|---|");
        for q in &chosen.queries {
            let _ = writeln!(
                m,
                "| `{}` | `{}` | {} | {} | {} | {:.3} ms | `{}` |",
                q.id,
                q.normalized,
                q.expect_plan,
                q.executed_plan,
                q.hits,
                q.p95_ms,
                q.explain_query_plan.join(" / ")
            );
        }
    }
    let _ = writeln!(m);

    let _ = writeln!(m, "## 下游怎么消费这份结论\n");
    let _ = writeln!(
        m,
        "- **todo 19** 建 `poem_fts` 时的 `detail=` 取 `chosen_mode`（本次为 `{}`），\
         不得硬编码；建完后应比对本文件，不一致即让构建失败。",
        r.chosen_mode
    );
    let _ = writeln!(
        m,
        "- **todo 24** 的 `len < 3` 分支按 `ngram_aux_enabled`（本次为 `{}`）决定是否走\
         辅助候选表；`len > 3` 分支是否可用 `MATCH` 取决于 `chosen_mode` 是否支持 phrase 查询，\
         运行时从 `corpus_meta.index_detail_mode` 读，不得假定。",
        r.ngram_aux_enabled
    );
    let _ = writeln!(
        m,
        "- **todo 22** 的语料 CI 逐条跑 `{}`，任何一条结果变化而契约未同步修改即失败。",
        r.contract.path
    );
    m
}
