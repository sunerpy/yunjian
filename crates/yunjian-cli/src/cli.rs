//! 命令行界面定义。
//!
//! 全局参数一律 `global = true`，因为 `yunjian search 明月 --json` 与
//! `yunjian --json search 明月` 都是用户会写的形态，只认前置位置会让前者报用法错误。

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use yunjian_core::{RhymeBook, RhymeTone, ToneFilter};

/// `yunjian` 的顶层命令。
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
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
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
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
    /// 语料库维护。
    Corpus {
        /// 具体动作。
        #[command(subcommand)]
        action: CorpusAction,
    },
    /// 以 stdio 承载 MCP 服务器。
    ///
    /// **本版本只有占位**：子命令与 `--help` 已就位，服务端实现见方案 todo 31。
    /// 保留占位而不是等实现完再加，是为了让 `yunjian mcp` 这个入口名从第一天起就固定。
    Mcp,
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
            Self::Corpus {
                action: CorpusAction::Status,
            } => "corpus.status",
            Self::Corpus {
                action: CorpusAction::Fetch,
            } => "corpus.fetch",
            Self::Mcp => "mcp",
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
    use super::{Book, Cli, Command, CorpusAction, LogLevel, Tone};
    use clap::{CommandFactory, Parser};
    use yunjian_core::{RhymeBook, RhymeTone, ToneFilter};

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
    fn the_mcp_subcommand_exists_so_the_entry_name_is_fixed_from_day_one() {
        assert!(matches!(
            Cli::try_parse_from(["yunjian", "mcp"])
                .expect("解析 mcp")
                .command,
            Command::Mcp
        ));
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
            Command::Mcp.name(),
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
