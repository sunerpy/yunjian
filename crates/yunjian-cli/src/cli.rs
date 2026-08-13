//! 命令行界面定义。
//!
//! 全局参数一律 `global = true`，因为 `yunjian search 明月 --json` 与
//! `yunjian --json search 明月` 都是用户会写的形态，只认前置位置会让前者报用法错误。

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use yunjian_core::{RhymeBook, RhymeTone, ToneFilter};
use yunjian_recite::{ClozeOptions, FsrsGrade, MaskStage, PracticeMode};

/// `yunjian` 的顶层命令。
///
/// 不派生 `Eq`：`recite --ratio` 是一个 `f32`，而 `f32` 没有 `Eq`。测试里的
/// `assert_eq!` 与 `matches!` 只需要 `PartialEq`。
#[derive(Debug, Clone, PartialEq, Parser)]
#[command(
    name = "yunjian",
    version,
    about = "云笺：离线可用的中国古典诗词检索",
    long_about = "云笺命令行。语料是一份只读 SQLite 文件，不联网、不登录。\n\
                  `--json` 在 stdout 输出稳定信封，全部日志走 stderr，因此\n\
                  `yunjian search 明月 --json | jq` 不会被日志破坏。"
)]
pub struct Cli {
    /// 全局参数。
    #[command(flatten)]
    pub global: Global,
    /// 子命令。
    #[command(subcommand)]
    pub command: Command,
}

/// 与子命令无关的全局参数。
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct Global {
    /// 配置文件路径。发现顺序：本参数 → `APP_CONFIG` → `./config.toml` → 用户配置目录。
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// 只读语料库文件路径，覆盖配置与 `YUNJIAN_CORPUS_PATH`。
    #[arg(long, global = true, value_name = "PATH")]
    pub corpus: Option<PathBuf>,
    /// 日志级别。显式给出时压过 `RUST_LOG`。
    #[arg(long, global = true, value_name = "LEVEL", value_enum)]
    pub log_level: Option<LogLevel>,
    /// 在 stdout 输出机器可读的 JSON 信封而不是人类可读文本。
    #[arg(long, global = true)]
    pub json: bool,
}

/// 子命令。
///
/// 不派生 `Eq`，理由同 [`Cli`]。
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Command {
    /// 检索正文或残句。
    Search {
        /// 查询文本。两字（如「明月」）即可，走辅助候选表而非 trigram。
        query: String,
        /// 单页命中上限，服务端硬上限 100。
        #[arg(long, default_value_t = 10, value_name = "N")]
        limit: usize,
        /// 只保留该作者的命中。**在本页内过滤**，见 `docs/CLI.zh.md`。
        #[arg(long, value_name = "NAME")]
        author: Option<String>,
        /// 只保留该朝代规范键的命中。**在本页内过滤**。
        #[arg(long, value_name = "DYNASTY")]
        dynasty: Option<String>,
        /// 为每条命中附上该韵书下的韵部归属。
        #[arg(long, value_name = "BOOK", value_enum)]
        rhyme_book: Option<Book>,
        /// 上一页返回的续页游标。
        #[arg(long, value_name = "CURSOR")]
        cursor: Option<String>,
    },
    /// 按稳定标识读取作品详情：本体、平仄、韵部、出处与历代集评。
    Show {
        /// 作品的稳定标识（`stable_id`）。
        poem_id: String,
    },
    /// 按作者名或作者名前缀读取作者详情与作品列表。
    Author {
        /// 作者名或前缀。
        name: String,
        /// 上一页返回的续页游标。
        #[arg(long, value_name = "CURSOR")]
        cursor: Option<String>,
    },
    /// 按韵部检索作品。韵书是必填项，没有隐式默认值。
    Rhyme {
        /// 韵部名，可带声部前缀（`七阳` 与 `下平七阳` 都接受）。
        group: String,
        /// 解释韵部名所依据的韵书。
        #[arg(long, value_name = "BOOK", value_enum)]
        book: Book,
        /// 按声调筛选。
        #[arg(long, value_name = "TONE", value_enum, default_value_t = Tone::Any)]
        tone: Tone,
    },
    /// 背诵练习与复习排程。作答从 stdin 读入。
    Recite(ReciteArgs),
    /// 语料库维护。
    Corpus {
        /// 具体动作。
        #[command(subcommand)]
        action: CorpusAction,
    },
    /// 语音模型维护。权重不随安装包分发，按需下载并逐个校验许可与摘要。
    Models {
        /// 具体动作。
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// AI 赏析维护。
    Ai {
        /// 具体动作。
        #[command(subcommand)]
        action: AiAction,
    },
    /// 承载 MCP 服务器。默认 stdio。
    #[cfg(feature = "mcp")]
    Mcp(McpArgs),
}

/// `recite` 的参数。
///
/// 一个位置参数与两个子命令共存，靠 `args_conflicts_with_subcommands` 与
/// `subcommand_negates_reqs` 消歧：`yunjian recite due` 走子命令，
/// `yunjian recite <poem-id>` 走位置参数。
#[derive(Debug, Clone, PartialEq, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct ReciteArgs {
    /// 排程查询动作；省略即按作品稳定标识做一轮练习。
    #[command(subcommand)]
    pub action: Option<ReciteAction>,
    /// 作品的稳定标识（`stable_id`）。
    #[arg(required = true, value_name = "POEM_ID")]
    pub poem_id: Option<String>,
    /// 练习形态。`voice` 在特性或模型缺失时退化为 `cloze`，退出码仍为 0。
    #[arg(long, value_name = "MODE", value_enum, default_value_t = Mode::Cloze)]
    pub mode: Mode,
    /// 挖空比例，取值 (0, 1]。只对 `--mode cloze` 有意义。
    #[arg(long, value_name = "RATIO", default_value_t = ClozeOptions::DEFAULT_RATIO, value_parser = parse_ratio)]
    pub ratio: f32,
    /// 挖空随机种子。省略则按当前时间取一个，并在输出里回显以便复现。
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,
    /// 遮挡的行数。只对 `--mode masked` 有意义；超出句数时按句数截断。
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub masked_lines: usize,
    /// 直接指定本次的 FSRS 等级，压过打字评分映射。
    ///
    /// 语音路径按裁决不做自动评级，退化成打字练习后若要沿用「用户自选等级」，用这个。
    #[arg(long, value_name = "GRADE", value_enum)]
    pub grade: Option<Grade>,
}

/// `recite` 的排程查询动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum ReciteAction {
    // 刻意不打开语料库：复习状态是用户自己的可写数据，按 `stable_id` 存取；为了给队列补上
    // 题目而去开一份首启派生实测 571.8 s 的语料，会把一条查询变成长任务。这条理由是给维护者
    // 的，因此用 `//` 而不是 `///`——写进文档注释就会被 clap 原样印到 `--help` 里。
    /// 列出今天到期的复习项。不读取语料库，缺语料时同样可查。
    Due {
        /// 连尚未到期的一并列出，即整份排程。
        #[arg(long)]
        all: bool,
    },
    /// 报告排程规模、等级分布与本机生效的评级阈值。
    Stats,
}

/// 练习形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum Mode {
    /// 挖空：按 `--ratio` 挖掉若干字，优先挖韵脚与实词。
    Cloze,
    /// 首字提示：每句只留第一个字。
    FirstChar,
    /// 遮挡：按 `--masked-lines` 从上往下逐句遮住。
    Masked,
    /// 语音：朗读并识别。不可用时退化为挖空。
    Voice,
}

impl Mode {
    /// 写进载荷的稳定标识，与命令行取值逐字一致。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Cloze => "cloze",
            Self::FirstChar => "first-char",
            Self::Masked => "masked",
            Self::Voice => "voice",
        }
    }

    /// 组出内核认识的练习形态。`Voice` 没有对应值，退化由调用方决定，故这里返回 `None`。
    #[must_use]
    pub fn practice(self, ratio: f32, seed: u64, masked_lines: usize) -> Option<PracticeMode> {
        match self {
            Self::Cloze => Some(PracticeMode::Cloze(ClozeOptions::new(ratio, seed))),
            Self::FirstChar => Some(PracticeMode::FirstChar),
            Self::Masked => Some(PracticeMode::Masked(MaskStage::new(masked_lines))),
            Self::Voice => None,
        }
    }
}

/// 用户直接指定的 FSRS 等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Grade {
    /// 未能回忆。
    Again,
    /// 回忆困难。
    Hard,
    /// 正常回忆。
    Good,
    /// 轻松且准确。
    Easy,
}

impl From<Grade> for FsrsGrade {
    fn from(grade: Grade) -> Self {
        match grade {
            Grade::Again => Self::Again,
            Grade::Hard => Self::Hard,
            Grade::Good => Self::Good,
            Grade::Easy => Self::Easy,
        }
    }
}

/// 解析并校验挖空比例。
///
/// 内核的 [`ClozeOptions::new`] 会把越界值静默夹到 `[0, 1]`，那是库该有的健壮性；但把
/// 用户写的 `--ratio 5` 悄悄当成「全挖」是把一次输入错误执行成了另一件事，所以命令行
/// 这一层直接拒绝。
fn parse_ratio(raw: &str) -> Result<f32, String> {
    let value: f32 = raw
        .parse()
        .map_err(|_| format!("挖空比例必须是小数，实际收到 `{raw}`"))?;
    if !value.is_finite() || value <= 0.0 || value > 1.0 {
        return Err(format!(
            "挖空比例必须落在 (0, 1] 区间内，实际收到 `{raw}`；0.3 表示挖掉三成字"
        ));
    }
    Ok(value)
}

/// `mcp` 的参数。
///
/// 启用 `mcp-http` 特性时它多出 `yunjian-mcp` 定义的那组 HTTP 开关；不启用时它是一个空的
/// 参数集，`yunjian mcp` 仍然只有 stdio 一种形态。**HTTP 的 flag 定义与守卫都在
/// `yunjian-mcp` 里**，这里只做转发：把 flag 抄一份到 CLI 会让「参数还在、守卫被删了」
/// 变成一次谁都不会发现的回归。
#[cfg(feature = "mcp")]
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct McpArgs {
    /// Streamable HTTP 传输的开关与绑定地址。
    #[cfg(feature = "mcp-http")]
    #[command(flatten)]
    pub http: yunjian_mcp::http::HttpOptions,
    /// 维护动作。缺省即承载服务器本身。
    #[command(subcommand)]
    pub action: Option<McpAction>,
}

/// `mcp` 的维护动作。
///
/// 只有一个变体也用枚举而不是布尔开关：`yunjian mcp install` 与 `yunjian mcp` 是两件
/// 完全不同的事（一个写文件后退出，一个占住 stdio 直到对端关闭），把它们编码成同一个
/// 命令的两种参数组合，会让「起服务时误带上 install 的参数」变成一个能解析通过的调用。
#[cfg(feature = "mcp")]
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum McpAction {
    /// 把 `yunjian mcp` 写进 MCP 客户端的配置文件。
    Install(crate::mcp_install::InstallArgs),
}

impl Command {
    /// 写进 JSON 信封 `command` 字段的稳定 ASCII 名。
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Search { .. } => "search",
            Self::Show { .. } => "show",
            Self::Author { .. } => "author",
            Self::Rhyme { .. } => "rhyme",
            Self::Recite(ReciteArgs { action: None, .. }) => "recite",
            Self::Recite(ReciteArgs {
                action: Some(ReciteAction::Due { .. }),
                ..
            }) => "recite.due",
            Self::Recite(ReciteArgs {
                action: Some(ReciteAction::Stats),
                ..
            }) => "recite.stats",
            Self::Corpus {
                action: CorpusAction::Status,
            } => "corpus.status",
            Self::Corpus {
                action: CorpusAction::Fetch,
            } => "corpus.fetch",
            Self::Models { action } => action.envelope_name(),
            Self::Ai { action } => action.envelope_name(),
            #[cfg(feature = "mcp")]
            Self::Mcp(McpArgs { action: None, .. }) => "mcp",
            #[cfg(feature = "mcp")]
            Self::Mcp(McpArgs {
                action: Some(McpAction::Install(_)),
                ..
            }) => "mcp.install",
        }
    }
}

/// `ai` 的动作。
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AiAction {
    /// 赏析缓存维护。
    Cache {
        /// 具体动作。
        #[command(subcommand)]
        action: AiCacheAction,
    },
}

impl AiAction {
    /// 信封里的命令名。
    #[must_use]
    pub const fn envelope_name(&self) -> &'static str {
        match self {
            Self::Cache {
                action: AiCacheAction::Purge(_),
            } => "ai.cache.purge",
        }
    }
}

/// `ai cache` 的动作。
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AiCacheAction {
    /// 清理用户自己付费生成的赏析；内置赏析不受影响。
    Purge(AiCachePurgeArgs),
}

/// 用户赏析缓存的清理范围。
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[group(id = "scope", required = true, multiple = false)]
pub struct AiCachePurgeArgs {
    /// 只清理指定模板版本。
    #[arg(long, value_name = "VERSION", group = "scope")]
    pub template: Option<String>,
    /// 只清理指定作品的稳定标识。
    #[arg(long, value_name = "ID", group = "scope")]
    pub poem: Option<String>,
    /// 清理全部用户赏析缓存。
    #[arg(long, group = "scope")]
    pub all: bool,
}

/// `models` 的动作。
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ModelsAction {
    /// 列出清单里的每个模型：用途、许可、体积与本地缓存状态。
    ///
    /// 只读本地状态，不联网。
    List,
    /// 下载、校验并解包一个模型。
    ///
    /// 已就位时直接返回，不发起任何网络请求。摘要不符时中止且不留下任何文件。
    Fetch {
        /// 模型名，取自 `models list`。
        name: String,
    },
    /// 核对本地已下载归档的摘要。
    ///
    /// 不给模型名时核对全部；本地没有归档的条目跳过而不算失败。
    Verify {
        /// 模型名；省略则核对全部。
        name: Option<String>,
    },
    /// 删掉一个模型的本地缓存（解包目录与归档）。
    Remove {
        /// 模型名。
        name: String,
    },
}

impl ModelsAction {
    /// 信封里的命令名。
    #[must_use]
    pub const fn envelope_name(&self) -> &'static str {
        match self {
            Self::List => "models.list",
            Self::Fetch { .. } => "models.fetch",
            Self::Verify { .. } => "models.verify",
            Self::Remove { .. } => "models.remove",
        }
    }
}

/// `corpus` 的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum CorpusAction {
    /// 报告语料库位置、版本、规模与派生结构状态。
    ///
    /// 语料库不存在时**不会**去落地一份，而是退出 3 并指向 `corpus fetch`：
    /// 一条查看状态的命令不该有十分钟的副作用。
    Status,
    /// 校验、解压并落地语料库，必要时派生检索结构。
    ///
    /// 首启派生实测唐宋规模 571.8 s，因此进度逐步汇报到 stderr。
    Fetch,
}

/// 日志级别。取值与 `yunjian-core` 的日志级别表逐字一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum LogLevel {
    /// 关闭日志。
    Off,
    /// 只记错误。
    Error,
    /// 记警告及以上。
    Warn,
    /// 记信息及以上。默认。
    Info,
    /// 记调试及以上。
    Debug,
    /// 全记。
    Trace,
}

impl LogLevel {
    /// `yunjian-core` 认识的级别名。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// 韻书。含未随包的中华新韵，因为「没有这本书」必须能被请求到并得到明确回答，
/// 而不是从选项里消失后让用户以为查过了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Book {
    /// 平水韵，用于诗。
    Pingshui,
    /// 词林正韵，用于词。
    Cilin,
    /// 中华新韵。**未随包分发**，请求它会得到一条许可判定说明。
    Xinyun,
}

impl From<Book> for RhymeBook {
    fn from(book: Book) -> Self {
        match book {
            Book::Pingshui => Self::Pingshui,
            Book::Cilin => Self::Cilin,
            Book::Xinyun => Self::Xinyun,
        }
    }
}

/// 声调筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Tone {
    /// 不筛选。
    Any,
    /// 平声。
    Level,
    /// 上声。
    Rising,
    /// 去声。
    Departing,
    /// 入声。
    Entering,
    /// 仄声（词林正韵不再细分的那一档）。
    Oblique,
}

impl From<Tone> for ToneFilter {
    fn from(tone: Tone) -> Self {
        match tone {
            Tone::Any => Self::Any,
            Tone::Level => Self::Only(RhymeTone::Level),
            Tone::Rising => Self::Only(RhymeTone::Rising),
            Tone::Departing => Self::Only(RhymeTone::Departing),
            Tone::Entering => Self::Only(RhymeTone::Entering),
            Tone::Oblique => Self::Only(RhymeTone::Oblique),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AiAction, AiCacheAction, Book, Cli, Command, CorpusAction, LogLevel, Tone};
    #[cfg(feature = "mcp")]
    use super::{McpAction, McpArgs};
    use clap::{CommandFactory, Parser};
    use yunjian_core::{RhymeBook, RhymeTone, ToneFilter};

    /// 解析一条命令行，取出子命令。
    fn parse_command(argv: &[&str]) -> Command {
        Cli::try_parse_from(argv)
            .unwrap_or_else(|error| panic!("解析 {argv:?} 失败：{error}"))
            .command
    }

    #[test]
    fn the_command_definition_itself_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_binary_is_named_yunjian_not_yunjian_cli() {
        // 包名是 `yunjian-cli`（裸名被占），但用户看到的名字必须是 `yunjian`。
        assert_eq!(Cli::command().get_name(), "yunjian");
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        // `yunjian search 明月 --json` 是验收用例里的确切形态。
        let cli = Cli::try_parse_from(["yunjian", "search", "明月", "--json"])
            .expect("子命令之后的全局参数必须能解析");
        assert!(cli.global.json);
        assert!(matches!(cli.command, Command::Search { .. }));
    }

    #[test]
    fn global_flags_are_accepted_before_the_subcommand() {
        let cli = Cli::try_parse_from(["yunjian", "--corpus", "/nonexistent", "search", "x"])
            .expect("子命令之前的全局参数必须能解析");
        assert_eq!(
            cli.global.corpus.as_deref(),
            Some(std::path::Path::new("/nonexistent"))
        );
    }

    #[test]
    fn search_defaults_to_ten_hits_and_no_filters() {
        let cli = Cli::try_parse_from(["yunjian", "search", "明月"]).expect("解析 search");
        let Command::Search {
            query,
            limit,
            author,
            dynasty,
            rhyme_book,
            cursor,
        } = cli.command
        else {
            panic!("应解析为 search");
        };
        assert_eq!(query, "明月");
        assert_eq!(limit, 10);
        assert!(author.is_none() && dynasty.is_none() && rhyme_book.is_none() && cursor.is_none());
    }

    #[test]
    fn rhyme_requires_an_explicit_book() {
        // 平水韵为诗韵，不适用于词牌格律；隐式默认值会让「拿词对平水韵」变成静默的错答案。
        Cli::try_parse_from(["yunjian", "rhyme", "七阳"]).expect_err("缺 --book 必须报错");
        let cli = Cli::try_parse_from(["yunjian", "rhyme", "七阳", "--book", "pingshui"])
            .expect("给出 --book 应解析成功");
        assert!(matches!(
            cli.command,
            Command::Rhyme {
                book: Book::Pingshui,
                tone: Tone::Any,
                ..
            }
        ));
    }

    #[test]
    fn corpus_has_exactly_status_and_fetch() {
        assert!(matches!(
            Cli::try_parse_from(["yunjian", "corpus", "status"])
                .expect("解析 corpus status")
                .command,
            Command::Corpus {
                action: CorpusAction::Status
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["yunjian", "corpus", "fetch"])
                .expect("解析 corpus fetch")
                .command,
            Command::Corpus {
                action: CorpusAction::Fetch
            }
        ));
        Cli::try_parse_from(["yunjian", "corpus", "purge"]).expect_err("未定义的动作必须报错");
    }

    #[test]
    fn ai_cache_purge_requires_exactly_one_scope() {
        for arguments in [["--template", "1.0.0"], ["--poem", "poem-fixture"]] {
            let cli = Cli::try_parse_from([
                "yunjian",
                "ai",
                "cache",
                "purge",
                arguments[0],
                arguments[1],
            ])
            .expect("单一清理范围应解析成功");
            assert!(matches!(
                cli.command,
                Command::Ai {
                    action: AiAction::Cache {
                        action: AiCacheAction::Purge(_)
                    }
                }
            ));
        }
        assert!(Cli::try_parse_from(["yunjian", "ai", "cache", "purge", "--all"]).is_ok());
        Cli::try_parse_from(["yunjian", "ai", "cache", "purge"]).expect_err("缺清理范围必须报错");
        Cli::try_parse_from([
            "yunjian",
            "ai",
            "cache",
            "purge",
            "--all",
            "--poem",
            "poem-fixture",
        ])
        .expect_err("多个清理范围必须报错");
    }

    #[test]
    fn the_mcp_subcommand_exists_so_the_entry_name_is_fixed_from_day_one() {
        assert!(matches!(
            Cli::try_parse_from(["yunjian", "mcp"])
                .expect("解析 mcp")
                .command,
            Command::Mcp(McpArgs { action: None, .. })
        ));
    }

    #[test]
    fn install_requires_an_explicit_client_because_the_two_shapes_are_not_interchangeable() {
        // 猜错客户端写出的是一份语法合法、语义为空的条目：客户端不报错，只是永远连不上。
        Cli::try_parse_from(["yunjian", "mcp", "install"]).expect_err("缺 --client 必须报错");
        Cli::try_parse_from(["yunjian", "mcp", "install", "--client", "vscode"])
            .expect_err("未支持的客户端必须报错");
        for (argument, expected) in [
            ("claude", crate::mcp_install::Client::Claude),
            ("opencode", crate::mcp_install::Client::OpenCode),
        ] {
            let cli = Cli::try_parse_from(["yunjian", "mcp", "install", "--client", argument])
                .expect("解析 install");
            let Command::Mcp(McpArgs {
                action: Some(McpAction::Install(args)),
                ..
            }) = cli.command
            else {
                panic!("应解析为 mcp install");
            };
            assert_eq!(args.client, expected);
            assert!(!args.global && !args.dry_run && args.path.is_none());
        }
    }

    #[test]
    fn serving_and_installing_are_two_different_envelope_names() {
        // 两者一个写文件后退出、一个占住 stdio，混成一个名字会让调用方分不清日志属于哪次运行。
        // 从 argv 解析而不是手工构造 `McpArgs`：`mcp-http` 特性会给它加一个字段，于是
        // 结构体字面量在开启时编译不过、`..default()` 在关闭时又是空更新。解析同时也走了
        // 真实路径。
        assert_eq!(parse_command(&["yunjian", "mcp"]).name(), "mcp");
        assert_eq!(
            parse_command(&["yunjian", "mcp", "install", "--client", "claude"]).name(),
            "mcp.install"
        );
    }

    #[test]
    fn every_command_maps_to_a_stable_ascii_envelope_name() {
        let names = [
            Command::Search {
                query: String::new(),
                limit: 10,
                author: None,
                dynasty: None,
                rhyme_book: None,
                cursor: None,
            }
            .name(),
            Command::Show {
                poem_id: String::new(),
            }
            .name(),
            Command::Author {
                name: String::new(),
                cursor: None,
            }
            .name(),
            Command::Rhyme {
                group: String::new(),
                book: Book::Pingshui,
                tone: Tone::Any,
            }
            .name(),
            Command::Corpus {
                action: CorpusAction::Status,
            }
            .name(),
            Command::Corpus {
                action: CorpusAction::Fetch,
            }
            .name(),
            parse_command(&["yunjian", "ai", "cache", "purge", "--all"]).name(),
            parse_command(&["yunjian", "mcp"]).name(),
            parse_command(&["yunjian", "mcp", "install", "--client", "opencode"]).name(),
        ];
        for name in names {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "信封里的命令名必须是 ASCII：{name}"
            );
        }
        let mut unique = names.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "命令名必须互不相同：{names:?}");
    }

    #[test]
    fn log_levels_match_the_core_level_table() {
        // 级别名对不上的后果不是报错而是静默：`EnvFilter` 把不认识的裸词当成 target 名，
        // `EnvFilter::new("verbose")` 会挡掉全工程的日志。真正跑起来的验证在
        // `tests/cli.rs::trace_level_floods_stderr_without_touching_stdout`。
        for (level, key) in [
            (LogLevel::Off, "off"),
            (LogLevel::Error, "error"),
            (LogLevel::Warn, "warn"),
            (LogLevel::Info, "info"),
            (LogLevel::Debug, "debug"),
            (LogLevel::Trace, "trace"),
        ] {
            assert_eq!(level.as_key(), key);
        }
    }

    #[test]
    fn books_and_tones_convert_to_core_types() {
        assert_eq!(RhymeBook::from(Book::Pingshui), RhymeBook::Pingshui);
        assert_eq!(RhymeBook::from(Book::Cilin), RhymeBook::Cilin);
        assert_eq!(RhymeBook::from(Book::Xinyun), RhymeBook::Xinyun);
        assert_eq!(ToneFilter::from(Tone::Any), ToneFilter::Any);
        assert_eq!(
            ToneFilter::from(Tone::Entering),
            ToneFilter::Only(RhymeTone::Entering)
        );
    }
}
