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
mod mobile_distribution;
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
        #[arg(long, default_value = corpus_measure::FIXTURE_CHINESE_POETRY_DIR)]
        chinese_poetry_dir: Option<std::path::PathBuf>,
        /// `Werneror/Poetry` 检出目录。默认使用随仓 fixture；实测完整语料时显式覆盖。
        #[arg(long, default_value = corpus_measure::FIXTURE_WERNEROR_DIR)]
        werneror_dir: Option<std::path::PathBuf>,
        /// `charlesix59/chinese_word_rhyme` 检出目录。默认使用随仓 fixture；完整实测时显式覆盖。
        #[arg(long, default_value = corpus_measure::FIXTURE_RHYME_DIR)]
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

    /// 扫描真实移动产物、执行 APK 上限与禁带资产守卫，并写出
    /// `docs/reports/mobile-size.{md,json}`。缺产物或设备时如实标 `NOT EXECUTED`。
    MobileDistribution {
        /// 包含 split APK、AAB、`.xcarchive` 或 IPA 的目录；递归发现产物。
        #[arg(long)]
        artifacts_dir: Option<std::path::PathBuf>,
        /// 物理设备 instrumented smoke 的 JSON 观测；不给就绝不把 smoke 写成 PASS。
        #[arg(long)]
        smoke_json: Option<std::path::PathBuf>,
    },

    /// 桌面真机验收或移动端可行性门禁。移动门禁缺少物理设备、签名或商店凭据时
    /// 如实写 `NOT EXECUTED`，不会编造测量值或框架选型。
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
        /// 目标平台：`win` | `mac` | `linux` | `android` | `ios`。
        #[arg(long)]
        platform: String,
        /// 断言集名：桌面平台用 `desktop`，移动平台用 `spike`。
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
        Some(Commands::MobileDistribution {
            artifacts_dir,
            smoke_json,
        }) => mobile_distribution::run(artifacts_dir, smoke_json),
        Some(Commands::Acceptance { platform, set }) => acceptance::run(&platform, &set),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 遍历仓库时跳过的目录：里面没有本仓库的调用点，且 `target/` 有近百 GiB。
    const SKIPPED_DIRS: [&str; 5] = ["target", "node_modules", ".git", "dist", "build"];

    /// 一行里出现这些 token 就换了一条命令，`run` 的上下文随之作废。
    const SHELL_SEPARATORS: [&str; 4] = ["&&", "||", ";", "|"];

    /// clap 真实认得的子命令名（含别名）。
    ///
    /// 刻意**问 clap 而不是解析 `Commands` 枚举的源码文本**：用户敲下的那个名字由 clap 的
    /// 改名规则（`CorpusBuild` → `corpus-build`）决定，源码里的变体名只是它的输入。
    /// 问 clap 等于问真相，且顺带免疫「注释里提到的变体名被当成数据」这类误判。
    fn known_subcommands() -> std::collections::BTreeSet<String> {
        use clap::CommandFactory;
        Cli::command()
            .get_subcommands()
            .flat_map(|sub| {
                std::iter::once(sub.get_name().to_string())
                    .chain(sub.get_all_aliases().map(str::to_string))
            })
            .collect()
    }

    /// 从一段命令文本里抽出所有被调用的 xtask 子命令名。
    ///
    /// 按 token 走状态机而不是找字符串前缀，是因为同一件事在仓库里有四种写法：
    /// `cargo run -p xtask -- x`、`$(CARGO) run -p xtask -- x`、
    /// `cargo run -p xtask --release -- x`、`cargo run --package xtask -- x`。
    /// 找前缀要为每种写法立一个常量，漏一种就是静默漏扫。
    ///
    /// **必须要求 `run` 出现过**：`cargo test -p xtask corpus_package` 里 `corpus_package`
    /// 是测试名过滤器，不是子命令。把它当子命令会让这条断言在一条完全正确的命令上变红，
    /// 而那种红没有信息量，只会训练人忽略它。
    fn invoked_subcommands(text: &str) -> Vec<String> {
        enum State {
            Idle,
            AwaitSeparator,
            AwaitName,
        }

        let mut found = Vec::new();
        let mut state = State::Idle;
        let mut saw_run = false;
        let mut previous = "";

        for token in text.split_whitespace() {
            if SHELL_SEPARATORS.contains(&token) {
                state = State::Idle;
                saw_run = false;
                previous = token;
                continue;
            }
            match state {
                State::AwaitName => {
                    // TOML / YAML 里命令是带引号的字符串，按空白切出来的 token 会粘着
                    // 结尾那个 `"`。不剥掉的话 `verify-models"` 找不到，于是一条完全正确
                    // 的配置被判成「引用了不存在的子命令」——假红同样是缺陷。
                    let name = token.trim_matches(|c| matches!(c, '"' | '\'' | '`' | ',' | ';'));
                    // `-- --help` 是没有子命令的合法调用，不是引用。
                    if !name.is_empty() && !name.starts_with('-') {
                        found.push(name.to_string());
                    }
                    state = State::Idle;
                    saw_run = false;
                }
                State::AwaitSeparator if token == "--" => state = State::AwaitName,
                State::AwaitSeparator => {}
                State::Idle => {
                    if token == "run" {
                        saw_run = true;
                    } else if token == "xtask" {
                        if previous == "cargo" {
                            // `cargo xtask <子命令>`：别名形态，下一个 token 就是子命令。
                            state = State::AwaitName;
                        } else if saw_run && (previous == "-p" || previous == "--package") {
                            state = State::AwaitSeparator;
                        }
                    }
                }
            }
            previous = token;
        }
        found
    }

    /// 去掉 `#` 起始的注释，但不动引号里的 `#`。
    ///
    /// **不剔注释这条断言就是错的，而且是两个方向都错。** `mobile/device-farm.toml` 第 50 行
    /// 的注释刻意记着一个**已被删掉**的子命令名（那是这次修复的历史记录），Makefile 与
    /// 工作流里也有注释提到 xtask 命令。扫原文会把这些历史记录判成缺陷（假红），
    /// 而 Makefile 里真正的调用一旦被注释掉又会被漏掉（假绿）。
    fn strip_hash_comments(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for line in text.lines() {
            let bytes = line.as_bytes();
            let mut cut = line.len();
            let mut quote: Option<u8> = None;
            for (index, byte) in bytes.iter().enumerate() {
                match quote {
                    Some(open) if *byte == open => quote = None,
                    Some(_) => {}
                    None if *byte == b'"' || *byte == b'\'' => quote = Some(*byte),
                    None if *byte == b'#' => {
                        cut = index;
                        break;
                    }
                    None => {}
                }
            }
            // `cut` 只落在 ASCII 的 `#` 上或行尾，因此这个切片不会切断多字节字符。
            out.push_str(&line[..cut]);
            out.push('\n');
        }
        out
    }

    /// 把行尾续行接起来，让 `-- <子命令>` 不会被换行切散。
    fn join_continuations(text: &str) -> String {
        text.replace("\\\n", " ")
    }

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("从 xtask/ 推出仓库根目录")
            .to_path_buf()
    }

    fn collect_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", dir.display()));
        for entry in entries {
            let path = entry.expect("读取目录项失败").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            if path.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_str()) {
                    collect_files(&path, found);
                }
            } else if name == "Makefile"
                || path
                    .extension()
                    .is_some_and(|ext| ext == "toml" || ext == "yml" || ext == "yaml")
            {
                found.push(path);
            }
        }
    }

    #[test]
    fn the_reference_extractor_reads_every_spelling_and_ignores_test_filters() {
        assert_eq!(
            invoked_subcommands("cargo run -p xtask -- verify-models"),
            ["verify-models"]
        );
        assert_eq!(
            invoked_subcommands("$(CARGO) run -p xtask --release -- corpus-build --scale 10k"),
            ["corpus-build"]
        );
        assert_eq!(
            invoked_subcommands("cargo run --package xtask -- verify-sources --offline"),
            ["verify-sources"]
        );
        assert_eq!(
            invoked_subcommands("cargo xtask verify-icons"),
            ["verify-icons"]
        );
        assert_eq!(
            invoked_subcommands(
                "cd mobile/android && gradle assemble && cargo run -p xtask -- verify-icons"
            ),
            ["verify-icons"]
        );
        assert!(
            invoked_subcommands("cargo test -p xtask corpus_package").is_empty(),
            "`cargo test -p xtask <过滤器>` 不是子命令调用；判成调用会在正确命令上变红"
        );
        assert!(
            invoked_subcommands("cargo build -p xtask --release").is_empty(),
            "没有 `--` 分隔符就没有子命令"
        );
        assert!(
            invoked_subcommands("cargo run -p xtask -- --help").is_empty(),
            "`-- --help` 是没有子命令的合法调用，不是一次引用"
        );
        assert_eq!(
            invoked_subcommands("build_command = \"cargo run -p xtask -- verify-models\""),
            ["verify-models"],
            "TOML 值的收尾引号必须剥掉，否则一条正确的配置会被判成引用了不存在的子命令"
        );
    }

    #[test]
    fn the_comment_stripper_keeps_history_from_impersonating_a_live_invocation() {
        // `mobile/device-farm.toml` 第 50 行的形状：注释里记着一个已被删掉的子命令。
        let history = "# 此前这里写的是 cargo run -p xtask -- mobile-package\n\
                       build_command = \"cargo run -p xtask -- verify-models\"\n";
        let live = invoked_subcommands(&strip_hash_comments(history));
        assert_eq!(
            live,
            ["verify-models"],
            "注释里的历史记录不得被当成调用，真实调用不得被漏掉：{live:?}"
        );
        let quoted = strip_hash_comments("a = \"x #1 y\" # 说明");
        assert!(
            quoted.contains("#1"),
            "引号里的 `#` 被误判成注释起点：{quoted}"
        );
        assert!(!quoted.contains("说明"), "引号外的注释未被剔除：{quoted}");
    }

    /// 配置与构建入口里引用的每个 xtask 子命令都必须真实存在。
    ///
    /// # 这条守的是哪一次真实失败
    ///
    /// `mobile/device-farm.toml` 的 `[android_full].build_command` 末段一度是
    /// `cargo run -p xtask -- mobile-package`，而 `Commands` 里从来没有过这个子命令。
    /// 配置本身看起来是完整的：`enabled = true`、设备池 ARN 真实、产物路径也都对。
    /// 于是 **Android 真机验收无法从干净检出复现**，而已落盘的十条 PASS 又都是真的——
    /// 「报告存在」与「报告可复现」被这一行悄悄拆开了，且没有任何测试会因此变红。
    ///
    /// # 为什么扫这三类文件
    ///
    /// 它们是 xtask 子命令被**调用**的全部落点：TOML 配置（Device Farm、分发）、
    /// `Makefile` 的门禁配方、工作流的 `run:` 段。文档里的命令是说明而不是调用，
    /// 说明写错不会让任何流程跑不通，不该由这条断言判红。
    #[test]
    fn every_xtask_subcommand_referenced_by_a_config_or_build_entrypoint_really_exists() {
        let known = known_subcommands();
        let mut files = Vec::new();
        collect_files(&repo_root(), &mut files);
        assert!(
            !files.is_empty(),
            "一份文件都没扫到，说明遍历本身坏了；零命中的扫描是最典型的假绿"
        );

        let mut references = 0usize;
        for file in &files {
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", file.display()));
            let commands = join_continuations(&strip_hash_comments(&text));
            for name in invoked_subcommands(&commands) {
                references += 1;
                assert!(
                    known.contains(&name),
                    "{} 引用了 xtask 子命令 `{name}`，而 clap 认得的只有 {known:?}。\
                     配置里写一个不存在的子命令不会让任何测试变红，却让这条命令所属的流程\
                     无法从干净检出复现——要么实现它，要么把配置改成真实存在的命令",
                    file.display()
                );
            }
        }

        assert!(
            references > 0,
            "没有扫到任何 xtask 子命令调用。若这是真的，这条断言应当连同理由一起删除；\
             更可能的是调用写法变了而 invoked_subcommands 没跟上，那样它会永远绿"
        );
    }

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
