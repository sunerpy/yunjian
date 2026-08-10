//! 仓库任务运行器（`cargo xtask <子命令>`）。
//!
//! 新增子命令的约定，务必遵守以避免并行改动冲突：
//!
//! 1. 在 `xtask/src/` 下新建一个独立模块文件，例如 `corpus_build.rs`；
//! 2. 在下方 `mod` 区块加一行 `mod corpus_build;`；
//! 3. 在 `Commands` 枚举里加一个变体；
//! 4. 在 `main` 的 `match` 里加一条分派臂。
//!
//! 每个子命令只碰自己的模块文件，加上此处两行注册，因此多个任务可以
//! 并行推进而互不覆盖。

use clap::{Parser, Subcommand};

// 子命令模块在此注册（每个任务追加一行）。
mod commentary_index;
mod corpus_contract;
mod corpus_measure;
mod corpus_quality;
mod index_spike;
mod verify_sources;

#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "云笺仓库任务运行器",
    long_about = None,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

/// 可用子命令。由后续任务逐个追加变体。
///
/// 之所以用 `Option<Commands>` 而非 `Commands`：空枚举使 `Cli` 成为无人
/// 居住类型，`Cli::parse()` 会被判定为不可达代码而产生 warning，届时
/// `cargo clippy -- -D warnings` 会失败。包一层 `Option` 即可保持骨架
/// 零 warning，且不影响后续追加变体。
#[derive(Debug, Subcommand)]
enum Commands {
    /// 实测 FTS5 `detail` 模式与辅助 n-gram 候选表，把黄金查询契约
    /// （`crates/yunjian-core/tests/queries.toml`）在合成样本语料上跑满每种配置，
    /// 并写出 `corpus/reports/index-mode.{json,md}` 作为 todo 19 / 24 读取的裁决文件。
    IndexSpike {
        /// 样本语料首数。默认 10000，即方案指定的规模。
        #[arg(long, default_value_t = 10_000)]
        scale: usize,
        /// 每条查询重复测量的次数，p95 取自这些样本。
        #[arg(long, default_value_t = 25)]
        repeats: usize,
    },

    /// 由 `corpus/commentary/sources/` 重新生成 `index.json`，并逐条校验出处。
    CommentaryIndex {
        /// 只校验索引与种子集是否一致，不写文件。
        #[arg(long)]
        check: bool,
    },

    /// 校验 `corpus/sources.toml`：锁定 revision、SPDX 允许列表、LICENSE 摘要、
    /// 逐资产授权判定，以及 `corpus/DENYLIST.md` 的拒绝清单。
    VerifySources {
        /// 只核对随仓保存的 LICENSE，不访问网络。
        #[arg(long)]
        offline: bool,
    },

    /// 在新建出来的样本规模语料库上逐条跑黄金查询契约
    /// （`crates/yunjian-core/tests/queries.toml`），断言每条的物理路径、命中下界与锚点。
    /// 纯门禁，不写任何报告文件；索引模式从 `corpus/reports/index-mode.json` 读。
    CorpusContract {
        /// 样本语料首数。默认 10000，即方案为 CI 指定的规模。
        #[arg(long, default_value_t = 10_000)]
        scale: usize,
    },

    /// 产出重出分组与数据缺陷报告：`corpus/reports/defects.{json,md}`（一行一个
    /// finding）与 `dispositions.json`（一行一条输入记录），并按
    /// `corpus/reports/baseline.json` 的逐 code 容差守住回归基线。
    CorpusQuality {
        /// `chinese-poetry` 检出目录。省略则跑随仓 fixture。
        #[arg(long, requires = "werneror_dir")]
        chinese_poetry_dir: Option<std::path::PathBuf>,
        /// `Werneror/Poetry` 检出目录。省略则跑随仓 fixture。
        #[arg(long, requires = "chinese_poetry_dir")]
        werneror_dir: Option<std::path::PathBuf>,
        /// 用本次实测重写基线，而不是拿基线校验本次实测。
        #[arg(long)]
        write_baseline: bool,
    },

    /// 在**真实语料**上实测索引体积与查询延迟，写出 `corpus/reports/measurements.{json,md}`
    /// 与对照声明预算（gzip 工件 <= 250 MB、p95 <= 150 ms）的明确结论。
    ///
    /// todo 21 的打包按这份结论选定的缓解措施执行，所以它不能建立在估算上。
    CorpusMeasure {
        /// 要实测的规模，可重复：`10k` | `tang-song` | `full`。默认只跑 `10k`。
        /// 未请求的规模在报告里如实标为 NOT MEASURED。
        #[arg(long = "scale", value_name = "SCALE")]
        scales: Vec<String>,
        /// `chinese-poetry` 按锁定 revision 的检出目录。`--render-only` 时不需要。
        #[arg(long, required_unless_present = "render_only")]
        chinese_poetry_dir: Option<std::path::PathBuf>,
        /// `Werneror/Poetry` 按锁定 revision 的检出目录。`--render-only` 时不需要。
        #[arg(long, required_unless_present = "render_only")]
        werneror_dir: Option<std::path::PathBuf>,
        /// `charlesix59/chinese_word_rhyme` 按锁定 revision 的检出目录。`--render-only` 时不需要。
        #[arg(long, required_unless_present = "render_only")]
        rhyme_dir: Option<std::path::PathBuf>,
        /// 每条查询重复测量的次数，p50/p95 取自这些样本。
        #[arg(long, default_value_t = 25)]
        repeats: usize,
        /// 随包工件预算，MiB。**仅用于验证门禁是真的**（把它设成 1 应当让结论翻假）；
        /// 正式结论一律用默认的 250。
        #[arg(long, default_value_t = corpus_measure::DEFAULT_ARTIFACT_BUDGET_MIB)]
        artifact_budget_mib: u64,
        /// 保留建好的 `.db` 供人工复核。默认测完即删，避免几十 GB 残留。
        #[arg(long)]
        keep_databases: bool,
        /// 只按现有 `measurements.json` 重渲染 Markdown，不重跑任何测量。
        /// 用于调整人读报告的排版——全量规模一次构建约 50 分钟。
        #[arg(long, conflicts_with_all = ["scales", "keep_databases"])]
        render_only: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        // 新增子命令时在此追加一条分派臂。
        Some(Commands::IndexSpike { scale, repeats }) => index_spike::run(scale, repeats),
        Some(Commands::CommentaryIndex { check }) => commentary_index::run(check),
        Some(Commands::VerifySources { offline }) => verify_sources::run(offline),
        Some(Commands::CorpusContract { scale }) => corpus_contract::run(scale),
        Some(Commands::CorpusQuality {
            chinese_poetry_dir,
            werneror_dir,
            write_baseline,
        }) => corpus_quality::run(chinese_poetry_dir, werneror_dir, write_baseline),
        Some(Commands::CorpusMeasure {
            render_only: true, ..
        }) => corpus_measure::render_only(),
        Some(Commands::CorpusMeasure {
            scales,
            chinese_poetry_dir,
            werneror_dir,
            rhyme_dir,
            repeats,
            artifact_budget_mib,
            keep_databases,
            render_only: false,
        }) => corpus_measure::run(
            scales,
            // clap 的 `required_unless_present` 已保证非 render-only 时三者都在。
            chinese_poetry_dir.expect("clap 应已要求 --chinese-poetry-dir"),
            werneror_dir.expect("clap 应已要求 --werneror-dir"),
            rhyme_dir.expect("clap 应已要求 --rhyme-dir"),
            repeats,
            artifact_budget_mib * 1024 * 1024,
            keep_databases,
        ),
        None => Ok(()),
    }
}
