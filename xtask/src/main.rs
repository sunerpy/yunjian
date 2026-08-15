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
mod acceptance;
mod assets_manifest;
mod cer_spike;
mod clean_install_report;
mod commentary_index;
mod corpus_build;
mod corpus_contract;
mod corpus_measure;
mod corpus_package;
mod corpus_quality;
mod index_spike;
mod pregenerate;
mod prerequisite;
mod provider_calls;
mod verify_icons;
mod verify_models;
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
        /// `chinese-poetry` 检出目录。默认使用随仓 fixture；实测完整语料时显式覆盖。
        #[arg(
            long,
            default_value = "crates/yunjian-corpus/tests/fixtures/chinese_poetry"
        )]
        chinese_poetry_dir: Option<std::path::PathBuf>,
        /// `Werneror/Poetry` 检出目录。默认使用随仓 fixture；实测完整语料时显式覆盖。
        #[arg(long, default_value = "crates/yunjian-corpus/tests/fixtures/werneror")]
        werneror_dir: Option<std::path::PathBuf>,
        /// `charlesix59/chinese_word_rhyme` 检出目录。默认使用随仓 fixture；完整实测时显式覆盖。
        #[arg(
            long,
            default_value = "crates/yunjian-corpus/tests/fixtures/rhyme_book"
        )]
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

    /// 构建**待发布**的那一对文件：`corpus.db`（随包，无诊断表、无候选表）与
    /// `corpus-audit.db`（审计，不随包）。随包默认集由实测结论选定，不接受 `--scale`。
    ///
    /// 刻意不复用 `corpus-measure --keep-databases`：那条路径会在库上跑一次首启构建，
    /// 留下的文件带着候选表，正是随包工件不该有的形态。
    CorpusBuild {
        /// `chinese-poetry` 按锁定 revision 的检出目录。
        #[arg(long)]
        chinese_poetry_dir: std::path::PathBuf,
        /// `Werneror/Poetry` 按锁定 revision 的检出目录。
        #[arg(long)]
        werneror_dir: std::path::PathBuf,
        /// `charlesix59/chinese_word_rhyme` 按锁定 revision 的检出目录。
        #[arg(long)]
        rhyme_dir: std::path::PathBuf,
        /// 输出目录。默认 `corpus/build/release`。
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
    },

    /// 把随包语料库打成可发布工件：`yunjian-corpus-<版本>.db.gz`、`.sha256` 旁文件与
    /// `manifest.json`（含兼容范围与实测结论）。
    ///
    /// 写出任何文件之前跑完五条中止断言（完整性、无诊断表、跨文件守恒、结论在预算内、
    /// 形态与结论一致）；第六条「最终 gzip 是否超预算」只能落盘后判定，超了就把刚写出的
    /// 文件删掉。工件发布在 `corpus-v*` tag 上，与应用发布 tag 分离。
    CorpusPackage {
        /// 随包库路径。审计库路径由它机械推出（`corpus.db` -> `corpus-audit.db`）。
        #[arg(long, default_value = "corpus/build/release/corpus.db")]
        corpus_db: std::path::PathBuf,
        /// 输出目录。默认 `corpus/build/package`。
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
    },

    /// 用**开放权重模型**预生成随包赏析数据集，写出 `dataset/appreciations.json`、
    /// 它的 `sha256` 旁文件与清单。
    ///
    /// 覆盖范围**显式声明**为四个选本（唐诗三百首、宋词三百首、千家诗、古诗文名篇），
    /// 不尝试全语料。闭源 API 供应商一律中止：随包数据集必须由可下载权重生成，
    /// 下载的权重不附带限制输出再分发的 API 条款。
    ///
    /// 不给 `--endpoint` 时**不执行推理**，只跑管道、门禁与溯源字段，清单如实标
    /// `generation_executed=false`，绝不编造正文。
    Pregenerate {
        /// 只读源语料库。源库一个字节都不会被改动。
        #[arg(long, default_value = "corpus/build/release/corpus.db")]
        corpus_db: std::path::PathBuf,
        /// 只生成前 N 首。试运行用。
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// 输出目录。默认仓库根下的 `dataset`。
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
        /// 权重标识。
        #[arg(long, default_value = pregenerate::DEFAULT_MODEL)]
        model: String,
        /// 权重许可（SPDX）。**仅用于验证门禁是真的**（设成非白名单值应当让本命令中止）；
        /// 正式产物一律用默认值。
        #[arg(long, default_value = pregenerate::DEFAULT_MODEL_LICENSE)]
        model_license: String,
        /// 本地运行时。**仅用于验证门禁是真的**（设成闭源 API 供应商应当让本命令中止）；
        /// 正式产物一律用默认值。
        #[arg(long, default_value = pregenerate::DEFAULT_PROVIDER)]
        provider: String,
        /// 本地开放权重运行时的 base URL。给了才真的调模型；不给则如实标未执行。
        #[arg(long, value_name = "URL")]
        endpoint: Option<String>,
    },

    /// 把容器里的观测行裁决成净机验收报告（`docs/reports/clean-install-<date>.{md,json}`）。
    ///
    /// 断言集是**预声明**的：少一条观测即中止，多一条断言集之外的 id 也中止。
    /// 这样一条难验的断言不能靠「不交观测」从报告里消失。
    CleanInstallReport {
        /// 容器写出的观测文件，可给多次（联网段与断网段各一份）。
        #[arg(long = "observed", value_name = "PATH", required = true)]
        observed: Vec<std::path::PathBuf>,
        /// `xtask provider-calls` 的输出。
        #[arg(long, default_value = "docs/reports/clean-install-provider-calls.json")]
        provider_calls: std::path::PathBuf,
        /// `pregenerate` 的数据集清单，用来裁决「正文是否为模型输出」。
        #[arg(long, default_value = "dataset/appreciations.manifest.json")]
        dataset_manifest: std::path::PathBuf,
        /// 待发布工件所在目录（语料、种子、统一清单与各自的 `.sha256`）。
        #[arg(long)]
        artifacts_dir: std::path::PathBuf,
        /// 净机镜像名。写进报告是验收要求：「报告指明所用的净环境」。
        #[arg(long)]
        image: String,
        /// 镜像摘要。
        #[arg(long, default_value = "")]
        image_digest: String,
        /// 容器内自报的系统。
        #[arg(long, default_value = "")]
        os_release: String,
        /// 容器内自报的内核。
        #[arg(long, default_value = "")]
        kernel: String,
        /// 容器启动时用户主目录里的条目数。
        #[arg(long, default_value_t = 0)]
        preexisting_home_entries: u32,
        /// 断网段用的隔离手段。
        #[arg(long, default_value = "")]
        offline_isolation: String,
        /// 报告输出目录。
        #[arg(long, default_value = "docs/reports")]
        out_dir: std::path::PathBuf,
        /// 报告日期，构成文件名。
        #[arg(long)]
        date: String,
        /// 被测提交。
        #[arg(long, default_value = "")]
        commit_sha: String,
    },

    /// 把语料工件清单与随包赏析种子清单合成一份统一 `assets_manifest.json`
    /// （`yunjian_core::assets::AssetsManifest` 的形状），供 `yunjian corpus fetch` 消费。
    ///
    /// **写盘之前先用应用运行期那个解析器验一遍**：应用会拒绝的清单在这里就发不出去，
    /// 而不是等用户执行 `corpus fetch` 时才发现。两份清单的 `corpus_version` 不一致亦中止。
    AssetsManifest {
        /// `corpus-package` 产出的语料清单。
        #[arg(long, default_value = "corpus/build/package/manifest.json")]
        corpus_manifest: std::path::PathBuf,
        /// `pregenerate` 产出的数据集清单。
        #[arg(long, default_value = "dataset/appreciations.manifest.json")]
        seed_manifest: std::path::PathBuf,
        /// 随包赏析种子 JSON。摘要当场重算，不信清单里那一行。
        #[arg(long, default_value = "dataset/appreciations.json")]
        seed: std::path::PathBuf,
        /// 两件工件的下载前缀，通常是某个 `corpus-v*` Release 的资产地址。
        #[arg(long, value_name = "URL")]
        base_url: String,
        /// 输出路径。缺省与语料清单同目录。
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },

    /// 实测「随包命中零次模型调用、冷诗恰好一次」。用 **fixture 种子**，验的是缓存路径
    /// 而不是产品内容——待发布数据集当前 `generation_executed=false`，用它验会得到一个假 PASS。
    ///
    /// 计数不符即以非零码中止：本子命令的产物是一条断言，不是一份观测记录。
    ProviderCalls {
        /// 只读源语料库。源库一个字节都不会被改动，也**不会**在它里面建派生结构。
        #[arg(long, default_value = "corpus/build/release/corpus.db")]
        corpus_db: std::path::PathBuf,
        /// 计数结果的输出路径。
        #[arg(long, default_value = "docs/reports/clean-install-provider-calls.json")]
        out: std::path::PathBuf,
    },

    /// 验收图标集：解析 `icon.ico` 的字节确认六种尺寸齐备且 32 px 层在最前、断言源图为
    /// 1024×1024 RGBA 且四角透明、断言托盘图标四角 alpha 为 0、断言小尺寸层未被降采样，并写出六档联系表
    /// （`docs/reports/icon-contact-sheet.png`）供人眼裁决。
    ///
    /// **生成器退出 0 不算验收**：`cargo tauri icon` 会把 `icons/icon.png` 覆写成 512×512，
    /// 且层序由它内部决定——两者都只能靠解析字节发现。
    VerifyIcons,

    /// 校验 `models.toml`：SPDX 允许列表（只认 MIT 与 Apache-2.0）、锁定 revision 的
    /// 许可证据摘要、证据文件里真的写着那个许可、`models/DENYLIST.md` 的拒绝清单，
    /// 以及夹带产物的分发影响声明。最后写出 `models.lock.json` 供 `jq` 断言。
    VerifyModels {
        /// 只核对随仓保存的许可证据，不访问网络。
        #[arg(long)]
        offline: bool,
    },

    /// 实测文言 CER 并写出 `docs/reports/asr-cer.{json,md}`，含 `scoring_mode` 裁决
    /// （只可能是 `advisory_accuracy` 或 `completeness_only`，**永远不是 `full`**）。
    /// todo 48、51、56、57 读那份报告。
    ///
    /// 不开 `voice` 特性时如实写 NOT MEASURED 并说明阻塞原因，绝不编造数字。
    CerSpike {
        /// 从锁定 revision 重建参考文本（需网络），不做测量。
        #[arg(long)]
        refresh_fixtures: bool,
        /// **仅用于验证阈值门禁是真的**：跳过测量，直接把总 CER 设成给定值，
        /// 确认裁决按 10% 阈值翻转。产出的报告会显式标注它不是测量结果。
        #[arg(long, value_name = "CER", conflicts_with = "refresh_fixtures")]
        force_cer: Option<f64>,
        /// 只测前 N 首。试运行用，报告会标注它不是完整测量。
        #[arg(long, value_name = "N", conflicts_with = "refresh_fixtures")]
        limit: Option<usize>,
        /// 把每一条的参考文本、识别结果与 CER 写成 JSONL。人工复核与排障用：
        /// 只看聚合 CER 分不出「识别得差」与「根本没识别」。
        #[arg(long, value_name = "PATH", conflicts_with = "refresh_fixtures")]
        dump_transcripts: Option<std::path::PathBuf>,
        /// 只按现有 `asr-cer.json` 重渲染 Markdown，不重跑测量。
        /// 用于调整人读报告的措辞——实测一轮约一小时。
        #[arg(long, conflicts_with_all = ["refresh_fixtures", "force_cer", "limit"])]
        render_only: bool,
    },

    /// 桌面端真机验收：在**交互式桌面会话**里安装并拉起应用，逐条判定一张
    /// **执行之前就已声明**的断言集，写出 `docs/reports/desktop-qa-<日期>.{json,md}`
    /// 与每条 UI 断言的截图。
    ///
    /// 绿色构建只证明某个缺陷没有复现，不证明产品能用；session 0 里再绿也证不了
    /// WebView 能显示。因此 UI 断言走真实 WebDriver（Linux 上 `tauri-driver` +
    /// `WebKitWebDriver`），WebDriver 到不了的操作系统级事实（原生窗口控件、输入法
    /// 组字、任务栏图标）走 `enigo` 合成输入加 X11 属性观测。
    ///
    /// **不用 mock 顶替**：做不到的一律标 `NOT EXECUTED` 并写明原因与可执行条件。
    /// `all_pass` 取最严格语义（零 FAIL 且零 NOT EXECUTED），因为终验会消费它，
    /// 而它最容易造成的误读是「三平台都过了」。
    Acceptance {
        /// 目标平台：`win` | `mac` | `linux`。
        #[arg(long)]
        platform: String,
        /// 断言集名。目前只有 `desktop`。
        #[arg(long, default_value = "desktop")]
        set: String,
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
            chinese_poetry_dir.expect("clap 应已提供 chinese-poetry 默认目录"),
            werneror_dir.expect("clap 应已提供 Werneror 默认目录"),
            rhyme_dir.expect("clap 应已提供韵书默认目录"),
            repeats,
            artifact_budget_mib * 1024 * 1024,
            keep_databases,
        ),
        Some(Commands::CorpusBuild {
            chinese_poetry_dir,
            werneror_dir,
            rhyme_dir,
            out_dir,
        }) => corpus_build::run(chinese_poetry_dir, werneror_dir, rhyme_dir, out_dir),
        Some(Commands::CorpusPackage { corpus_db, out_dir }) => {
            corpus_package::run(corpus_db, out_dir)
        }
        Some(Commands::Pregenerate {
            corpus_db,
            limit,
            out_dir,
            model,
            model_license,
            provider,
            endpoint,
        }) => pregenerate::run(
            corpus_db,
            limit,
            out_dir,
            model,
            model_license,
            provider,
            endpoint,
        ),
        Some(Commands::CleanInstallReport {
            observed,
            provider_calls,
            dataset_manifest,
            artifacts_dir,
            image,
            image_digest,
            os_release,
            kernel,
            preexisting_home_entries,
            offline_isolation,
            out_dir,
            date,
            commit_sha,
        }) => clean_install_report::run(
            observed,
            provider_calls,
            dataset_manifest,
            artifacts_dir,
            image,
            image_digest,
            os_release,
            kernel,
            preexisting_home_entries,
            offline_isolation,
            out_dir,
            date,
            commit_sha,
        ),
        Some(Commands::AssetsManifest {
            corpus_manifest,
            seed_manifest,
            seed,
            base_url,
            out,
        }) => assets_manifest::run(corpus_manifest, seed_manifest, seed, base_url, out),
        Some(Commands::ProviderCalls { corpus_db, out }) => provider_calls::run(corpus_db, out),
        Some(Commands::VerifyIcons) => verify_icons::run(),
        Some(Commands::VerifyModels { offline }) => verify_models::run(offline),
        Some(Commands::CerSpike {
            refresh_fixtures,
            force_cer,
            limit,
            dump_transcripts,
            render_only,
        }) => cer_spike::run(
            None,
            refresh_fixtures,
            force_cer,
            limit,
            dump_transcripts,
            render_only,
        ),
        Some(Commands::Acceptance { platform, set }) => acceptance::run(&platform, &set),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_measure_uses_the_shipped_fixture_directories_by_default() {
        let cli = Cli::try_parse_from(["xtask", "corpus-measure", "--scale", "10k"])
            .expect("计划中的无路径验收入口必须可解析");

        let Some(Commands::CorpusMeasure {
            chinese_poetry_dir,
            werneror_dir,
            rhyme_dir,
            ..
        }) = cli.command
        else {
            panic!("应解析为 corpus-measure");
        };

        assert_eq!(
            chinese_poetry_dir,
            Some("crates/yunjian-corpus/tests/fixtures/chinese_poetry".into())
        );
        assert_eq!(
            werneror_dir,
            Some("crates/yunjian-corpus/tests/fixtures/werneror".into())
        );
        assert_eq!(
            rhyme_dir,
            Some("crates/yunjian-corpus/tests/fixtures/rhyme_book".into())
        );
    }
}
