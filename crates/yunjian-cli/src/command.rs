//! 子命令的执行与结果组装。
//!
//! 全部检索一律经 [`Yunjian`] 门面，不直接调 `search::*` 的内部函数：门面是 todo 29 固化
//! 的稳定表面，绕过它就等于让 CLI 与核心内部结构耦合，而桌面端、MCP 与 FFI 三个外壳都
//! 依赖这层稳定性。

use crate::cli::{
    AiAction, AiCacheAction, AiCachePurgeArgs, Book, Command, CorpusAction, ModelsAction,
};
use crate::envelope::{ErrorCode, Failure, Status, Warning, WarningCode};
use crate::exit::{Exit, corpus_failure, describe, describe_model};
use crate::output::{
    AiCachePurgeOut, CorpusOut, ModelFetchOut, ModelListOut, ModelRemoveOut, ModelRow, NotFound,
    Renderable, SearchFilters, SearchHit, SearchOut,
};
use crate::provision::{Provisioned, degradation, provision};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use yunjian_core::{
    AuthorDetailRequest, Config, CorpusConfig, Error, PoemDetailRequest, Result,
    RhymeGroupMembership, RhymeGroupSearchRequest, TEXT_SEARCH_HARD_CAP, TextSearchRequest,
    Yunjian,
};
use yunjian_voice::models::{FetchProgress, ModelCache, ModelError};

/// 一次子命令执行的全部产出。
#[derive(Debug)]
pub struct Report {
    /// 信封里的命令名。
    pub command: &'static str,
    /// 进程退出码。
    pub exit: Exit,
    /// 降级与退化提示。
    pub warnings: Vec<Warning>,
    /// 结果本体。
    pub body: Body,
    /// 人类可读输出；失败时为空，因为失败文案走 stderr 日志。
    pub human: Vec<String>,
}

/// 结果本体。
///
/// 三个变体而不是「可选载荷 + 可选错误」：后者能表达出「既没有载荷也没有错误」这种
/// 不存在的状态，于是每个消费方都得为它写一条永远走不到的分支。
#[derive(Debug)]
pub enum Body {
    /// 成功且有结果。
    Ok(Value),
    /// 成功但结果为空。
    Empty(Value),
    /// 未能完成。
    Failed(Failure),
}

impl Body {
    /// 对应的信封状态。
    #[must_use]
    pub const fn status(&self) -> Status {
        match self {
            Self::Ok(_) => Status::Ok,
            Self::Empty(_) => Status::Empty,
            Self::Failed(_) => Status::Error,
        }
    }
}

/// 一次成功执行的产出。
struct Produced {
    data: Value,
    human: Vec<String>,
    empty: bool,
    warnings: Vec<Warning>,
}

impl Produced {
    fn new<T>(value: &T, empty: bool) -> Result<Self>
    where
        T: Serialize + Renderable,
    {
        Ok(Self {
            data: to_value(value)?,
            human: value.render(),
            empty,
            warnings: Vec::new(),
        })
    }

    fn warn(mut self, warnings: Vec<Warning>) -> Self {
        self.warnings.extend(warnings);
        self
    }
}

/// 执行子命令。
///
/// 失败一律翻成 [`Report`] 而不是向上抛：退出码与信封是这一层的产物，把错误抛给调用方
/// 只会让分类逻辑散落到两处。
#[must_use]
pub fn execute(command: &Command, config: &Config, corpus_override: Option<&Path>) -> Report {
    let name = command.name();
    match run(command, config, corpus_override) {
        Ok(produced) => Report {
            command: name,
            exit: if produced.empty {
                Exit::NoResults
            } else {
                Exit::Success
            },
            warnings: produced.warnings,
            body: if produced.empty {
                Body::Empty(produced.data)
            } else {
                Body::Ok(produced.data)
            },
            human: produced.human,
        },
        Err(failed) => Report {
            command: name,
            exit: failed.exit,
            warnings: failed.warnings,
            body: Body::Failed(failed.failure),
            human: Vec::new(),
        },
    }
}

/// 一次失败执行的产出。警告要留着：语料退化的提示在失败时同样值得转达。
struct Failed {
    exit: Exit,
    failure: Failure,
    warnings: Vec<Warning>,
}

impl Failed {
    fn from_core(error: &Error) -> Self {
        let (exit, failure) = describe(error);
        Self {
            exit,
            failure,
            warnings: Vec::new(),
        }
    }

    fn from_model(error: ModelError) -> Self {
        let (exit, failure) = describe_model(&error);
        Self {
            exit,
            failure,
            warnings: Vec::new(),
        }
    }
}

impl From<Error> for Failed {
    fn from(error: Error) -> Self {
        Self::from_core(&error)
    }
}

fn run(
    command: &Command,
    config: &Config,
    corpus_override: Option<&Path>,
) -> std::result::Result<Produced, Failed> {
    let corpus = corpus_config(config, corpus_override);
    match command {
        #[cfg(feature = "mcp")]
        Command::Mcp(crate::cli::McpArgs {
            action: Some(crate::cli::McpAction::Install(args)),
            ..
        }) => mcp_install(args),
        #[cfg(feature = "mcp")]
        Command::Mcp(_) => unreachable!("起 MCP 服务由 MCP 专用入口执行"),
        Command::Corpus {
            action: CorpusAction::Status,
        } => corpus_status(&corpus),
        Command::Corpus {
            action: CorpusAction::Fetch,
        } => corpus_fetch(&corpus),
        // 模型命令刻意**不打开语料库**：它维护的是语音权重，与诗库无关，而打开语料在
        // 首启时是十分钟级的副作用。一条查模型许可的命令不该触发那件事。
        Command::Models { action } => models(action),
        Command::Ai {
            action:
                AiAction::Cache {
                    action: AiCacheAction::Purge(args),
                },
        } => ai_cache_purge(config, args),
        Command::Search {
            query,
            limit,
            author,
            dynasty,
            rhyme_book,
            cursor,
        } => {
            let (client, warnings) = open(&corpus)?;
            search(
                &client,
                query,
                *limit,
                author.as_deref(),
                dynasty.as_deref(),
                *rhyme_book,
                cursor.as_deref(),
            )
            .map(|produced| produced.warn(warnings))
            .map_err(Failed::from)
        }
        Command::Show { poem_id } => {
            let (client, warnings) = open(&corpus)?;
            show(&client, poem_id).map(|produced| produced.warn(warnings))
        }
        Command::Author { name, cursor } => {
            let (client, warnings) = open(&corpus)?;
            author_detail(&client, name, cursor.as_deref())
                .map(|produced| produced.warn(warnings))
                .map_err(Failed::from)
        }
        Command::Rhyme { group, book, tone } => {
            let (client, warnings) = open(&corpus)?;
            rhyme(&client, group, *book, (*tone).into())
                .map(|produced| produced.warn(warnings))
                .map_err(Failed::from)
        }
    }
}

fn ai_cache_purge(
    config: &Config,
    args: &AiCachePurgeArgs,
) -> std::result::Result<Produced, Failed> {
    use yunjian_ai::{AppreciationCache, DEFAULT_APPRECIATION_CACHE_CAPACITY, PurgeScope};

    let (scope, label) = match (&args.template, &args.poem, args.all) {
        (Some(version), None, false) => (
            PurgeScope::Template(version.clone()),
            format!("template:{version}"),
        ),
        (None, Some(poem_id), false) => {
            (PurgeScope::Poem(poem_id.clone()), format!("poem:{poem_id}"))
        }
        (None, None, true) => (PurgeScope::All, "all".to_owned()),
        _ => unreachable!("clap 保证清理范围恰好一个"),
    };
    let cache = AppreciationCache::open(
        &config.app.data_dir,
        "",
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )
    .map_err(Failed::from)?;
    let removed = cache.purge(scope).map_err(Failed::from)?;
    Produced::new(
        &AiCachePurgeOut {
            scope: label,
            removed,
            database: cache.path().display().to_string(),
        },
        false,
    )
    .map_err(Failed::from)
}

/// 组装本次运行实际使用的语料配置。
///
/// `--corpus` 压过配置文件与 `YUNJIAN_CORPUS_PATH`：命令行是用户最直接的表达，而
/// 显式指定的路径不存在时会报错而不是静默回落，这一点由核心保证。
fn corpus_config(config: &Config, corpus_override: Option<&Path>) -> CorpusConfig {
    let mut corpus = config.corpus.clone();
    if let Some(path) = corpus_override {
        corpus.path = Some(PathBuf::from(path));
    }
    corpus
}

/// 取语料并把派生退化转成警告。
fn open(corpus: &CorpusConfig) -> std::result::Result<(Yunjian, Vec<Warning>), Failed> {
    let provisioned = provision(corpus).map_err(|reason| Failed {
        exit: Exit::CorpusUnavailable,
        failure: corpus_failure(reason),
        warnings: Vec::new(),
    })?;
    let warnings = derived_warnings(&provisioned);
    Ok((Yunjian::new(provisioned.handle), warnings))
}

fn derived_warnings(provisioned: &Provisioned) -> Vec<Warning> {
    degradation(provisioned.handle.derived())
        .map(|message| vec![Warning::new(WarningCode::DerivedUnavailable, message)])
        .unwrap_or_default()
}

fn search(
    client: &Yunjian,
    query: &str,
    limit: usize,
    author: Option<&str>,
    dynasty: Option<&str>,
    rhyme_book: Option<Book>,
    cursor: Option<&str>,
) -> Result<Produced> {
    if limit == 0 {
        return Err(Error::Search("--limit 必须大于 0".to_owned()));
    }
    let book = rhyme_book.map(yunjian_core::RhymeBook::from);
    // 韵书未随包时立刻报错，而不是给出一串空标注：空标注会被读成「这些诗没有韵部」。
    if let Some(book) = book {
        book.ensure_available()?;
    }

    let page = client.search_text(TextSearchRequest {
        query: query.to_owned(),
        limit: limit.min(TEXT_SEARCH_HARD_CAP),
        cursor: cursor.map(str::to_owned),
    })?;

    let mut warnings = Vec::new();
    if let Some(reason) = page.plan_used.warning() {
        warnings.push(Warning::new(
            WarningCode::DegradedPlan,
            format!("本次查询没有走索引约束：{reason}"),
        ));
    }

    let total_before_filter = page.hits.len();
    let mut hits = Vec::with_capacity(total_before_filter);
    for hit in page.hits.iter().cloned() {
        if author.is_some_and(|author| hit.author != author)
            || dynasty.is_some_and(|dynasty| hit.dynasty != dynasty)
        {
            continue;
        }
        let groups = match book {
            Some(book) => Some(memberships(client, &hit.poem_id, book)?),
            None => None,
        };
        hits.push(SearchHit::new(hit, groups));
    }

    let filters = SearchFilters {
        author: author.map(str::to_owned),
        dynasty: dynasty.map(str::to_owned),
        rhyme_book: book.map(|book| book.as_key().to_owned()),
    };
    if hits.is_empty() && total_before_filter > 0 && filters.filters_hits() {
        warnings.push(Warning::new(
            WarningCode::FilteredPageEmpty,
            format!(
                "本页 {total_before_filter} 条命中被过滤条件清空；过滤只作用于本页，请翻页后重试"
            ),
        ));
    }

    let empty = hits.is_empty();
    let out = SearchOut::new(query.to_owned(), limit, &page, filters, hits);
    Ok(Produced::new(&out, empty)?.warn(warnings))
}

/// 取一首作品在指定韵书下的韵部归属。
fn memberships(
    client: &Yunjian,
    poem_id: &str,
    book: yunjian_core::RhymeBook,
) -> Result<Vec<RhymeGroupMembership>> {
    let detail = client.poem_detail(PoemDetailRequest {
        poem_id: poem_id.to_owned(),
    })?;
    Ok(detail
        .rhyme_groups
        .into_iter()
        .filter(|membership| membership.book == book)
        .collect())
}

/// 读取作品详情。
///
/// **查不到是退出 1，不是退出 3。** 空 id 在这里就地拦下（那是用法错误），因此
/// `poem_detail` 之后返回的 `Error::Search` 只可能来自「这个 `stable_id` 不在语料里」
/// ——该函数的其余失败都是 `Corpus` / `Db` / `CommentaryCitationMissing`。这条推理是结构性的，
/// 不依赖错误文案，端到端由 `tests/cli.rs` 的
/// `show_on_an_unknown_id_exits_one_because_it_is_a_miss_not_a_broken_corpus` 钉住。
fn show(client: &Yunjian, poem_id: &str) -> std::result::Result<Produced, Failed> {
    let trimmed = poem_id.trim();
    if trimmed.is_empty() {
        return Err(Failed {
            exit: Exit::Usage,
            failure: Failure::new(ErrorCode::Usage, "`show` 需要一个非空的 stable_id"),
            warnings: Vec::new(),
        });
    }
    match client.poem_detail(PoemDetailRequest {
        poem_id: trimmed.to_owned(),
    }) {
        Ok(detail) => Produced::new(&detail, false).map_err(Failed::from),
        Err(Error::Search(_)) => Produced::new(&NotFound::new(trimmed), true).map_err(Failed::from),
        Err(other) => Err(Failed::from(other)),
    }
}

fn author_detail(client: &Yunjian, name: &str, cursor: Option<&str>) -> Result<Produced> {
    let detail = client.author_detail(AuthorDetailRequest {
        query: name.to_owned(),
        cursor: cursor.map(str::to_owned),
    })?;
    let empty = detail.page.hits.is_empty();
    Produced::new(&detail, empty)
}

fn rhyme(
    client: &Yunjian,
    group: &str,
    book: Book,
    tone: yunjian_core::ToneFilter,
) -> Result<Produced> {
    let matches = client.find_by_rhyme_group(RhymeGroupSearchRequest {
        book: book.into(),
        rhyme_group: group.to_owned(),
        tone,
    })?;
    // 未消歧的候选不算命中：`hits` 空而 `unresolved` 非空时结果依然是「没有肯定的命中」。
    let empty = matches.hits.is_empty();
    Produced::new(&matches, empty)
}

/// 把 `yunjian mcp` 写进客户端配置。
///
/// 不碰语料库：一台还没取语料的机器同样应该能先把服务器注册好，让客户端在语料到位后
/// 直接可用。把注册与语料耦在一起只会多出一条「先取 211 MiB 才能改一行 JSON」的要求。
#[cfg(feature = "mcp")]
fn mcp_install(args: &crate::mcp_install::InstallArgs) -> std::result::Result<Produced, Failed> {
    use crate::mcp_install::{Dirs, InstallOut, install};

    let dirs = Dirs::discover().map_err(|error| Failed {
        exit: Exit::Usage,
        failure: Failure::new(
            ErrorCode::ClientConfigInvalid,
            format!("取不到当前目录：{error}"),
        )
        .with_hint("用 `--path` 显式指定配置文件"),
        warnings: Vec::new(),
    })?;
    let outcome = install(args, &dirs).map_err(|refusal| {
        let (exit, failure) = refusal.describe();
        Failed {
            exit,
            failure,
            warnings: Vec::new(),
        }
    })?;

    let mut warnings = Vec::new();
    // `--global` 对没有项目级配置的客户端无意义。静默接受会让用户以为自己控制了作用域。
    if args.global && !args.client.has_project_scope() && args.path.is_none() {
        warnings.push(Warning::new(
            WarningCode::ClientScopeIgnored,
            format!(
                "{} 只有用户级配置，`--global` 未改变目标文件",
                args.client.as_key()
            ),
        ));
    }
    Ok(Produced::new(&InstallOut::new(&outcome), false)?.warn(warnings))
}

fn corpus_status(corpus: &CorpusConfig) -> std::result::Result<Produced, Failed> {
    // 刻意先做一次纯文件系统检查：`provision` 会在语料缺失时去校验并解压归档，那是十分钟
    // 级别的副作用，而 `corpus status` 是一条查看状态的命令。缺语料时它必须只报告并指向
    // `corpus fetch`，把落地这个决定留给用户。
    let target = resolved_corpus_file(corpus);
    if !target.is_file() {
        return Err(Failed {
            exit: Exit::CorpusUnavailable,
            failure: corpus_failure(format!("尚无可用语料库：{} 不存在", target.display())),
            warnings: Vec::new(),
        });
    }
    let provisioned = provision(corpus).map_err(|reason| Failed {
        exit: Exit::CorpusUnavailable,
        failure: corpus_failure(reason),
        warnings: Vec::new(),
    })?;
    let warnings = derived_warnings(&provisioned);
    let out = CorpusOut::new(&provisioned.handle, provisioned.materialized());
    Produced::new(&out, false)
        .map(|produced| produced.warn(warnings))
        .map_err(Failed::from)
}

fn corpus_fetch(corpus: &CorpusConfig) -> std::result::Result<Produced, Failed> {
    let provisioned = provision(corpus).map_err(|reason| Failed {
        exit: Exit::CorpusUnavailable,
        // 这条失败不能再建议「运行 corpus fetch」——用户刚刚运行的就是它。
        failure: Failure::new(ErrorCode::CorpusUnavailable, reason)
            .with_hint("检查 `corpus.archive` 与 `corpus.data_dir` 指向的归档是否存在且摘要一致"),
        warnings: Vec::new(),
    })?;
    let warnings = derived_warnings(&provisioned);
    let out = CorpusOut::new(&provisioned.handle, provisioned.materialized());
    Produced::new(&out, false)
        .map(|produced| produced.warn(warnings))
        .map_err(Failed::from)
}

/// 语料解析顺序里前两级的目标文件。第三级（从归档落地）刻意不在这里，
/// 因为它是 `corpus status` 要避免触发的那件事。
fn resolved_corpus_file(corpus: &CorpusConfig) -> PathBuf {
    corpus
        .path
        .clone()
        .unwrap_or_else(|| corpus.data_dir.join(yunjian_core::CORPUS_FILE_NAME))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| Error::Search(format!("结果序列化失败：{error}")))
}

fn models(action: &ModelsAction) -> std::result::Result<Produced, Failed> {
    let cache = ModelCache::discover();
    match action {
        ModelsAction::List => model_rows("list", &cache, None),
        ModelsAction::Verify { name } => model_rows("verify", &cache, Some(name.as_deref())),
        ModelsAction::Fetch { name } => model_fetch(&cache, name),
        ModelsAction::Remove { name } => model_remove(&cache, name),
    }
}

/// `list` 与 `verify` 共用一条实现，差别只在要不要真的核对摘要。
///
/// `scope` 为 `None` 是 `list`；`Some(None)` 是核对全部；`Some(Some(name))` 是核对一个。
fn model_rows(
    action: &'static str,
    cache: &ModelCache,
    scope: Option<Option<&str>>,
) -> std::result::Result<Produced, Failed> {
    let statuses = cache.statuses().map_err(Failed::from_model)?;
    let mut rows = Vec::with_capacity(statuses.len());
    for status in statuses {
        if let Some(Some(wanted)) = scope
            && status.name != wanted
        {
            continue;
        }
        let verified_sha256 = match scope {
            None => None,
            Some(_) => cache
                .verify_archive(&status.name)
                .map_err(Failed::from_model)?,
        };
        rows.push(ModelRow {
            name: status.name,
            kind: status.kind.as_str(),
            role: status.role.as_str(),
            license: status.license,
            size_bytes: status.size_bytes,
            unpacked: status.unpacked,
            archived: status.archived,
            attribution: status.attribution,
            refused: status.refused,
            verified_sha256,
        });
    }

    if let Some(Some(wanted)) = scope
        && rows.is_empty()
    {
        // 名字打错时报 `Unknown`，而不是「核对了 0 个，成功」——后者会让脚本以为校验过了。
        return Err(Failed::from_model(
            yunjian_voice::models::Registry::shipped()
                .and_then(|registry| registry.find(wanted).map(|_| ()))
                .expect_err("名字能在清单里找到就不会走到这里"),
        ));
    }

    let empty = rows.is_empty();
    let out = ModelListOut {
        action,
        cache_root: cache.root().display().to_string(),
        models: rows,
    };
    Produced::new(&out, empty).map_err(Failed::from)
}

fn model_fetch(cache: &ModelCache, name: &str) -> std::result::Result<Produced, Failed> {
    let entry_license_and_attribution = yunjian_voice::models::Registry::shipped()
        .and_then(|registry| {
            registry
                .admit(name)
                .map(|entry| (entry.license.clone(), entry.attribution_file()))
        })
        .map_err(Failed::from_model)?;

    // 下载是长任务，进度按工作区约定走 `tracing` 到 stderr——stdout 属于结果与 MCP
    // 协议流，进度条写到那里会毁掉 `--json | jq`。
    let path = cache
        .ensure(name, &mut log_fetch_progress)
        .map_err(Failed::from_model)?;

    let (license, attribution) = entry_license_and_attribution;
    let out = ModelFetchOut {
        name: name.to_owned(),
        path: path.display().to_string(),
        license,
        attribution,
    };
    Produced::new(&out, false).map_err(Failed::from)
}

fn model_remove(cache: &ModelCache, name: &str) -> std::result::Result<Produced, Failed> {
    let removed = cache.remove(name).map_err(Failed::from_model)?;
    let out = ModelRemoveOut {
        name: name.to_owned(),
        removed_dir: removed.dir,
        removed_archive: removed.archive,
    };
    // 什么都没删不是失败，但也不是「有结果」——退出 1 让脚本能区分。
    Produced::new(&out, removed.is_empty()).map_err(Failed::from)
}

fn log_fetch_progress(event: FetchProgress) {
    match event {
        FetchProgress::Downloading {
            bytes_done,
            bytes_total,
        } => tracing::info!(bytes_done, bytes_total, "正在下载模型"),
        FetchProgress::Verifying { bytes } => tracing::info!(bytes, "正在核对模型归档摘要"),
        FetchProgress::Verified => tracing::info!("模型归档摘要已核对"),
        FetchProgress::Unpacking => tracing::info!("正在解包模型"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Body, corpus_config, resolved_corpus_file, run};
    use crate::cli::{AiAction, AiCacheAction, AiCachePurgeArgs, Command, CorpusAction};
    use crate::exit::Exit;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use yunjian_core::{Config, CorpusConfig};

    fn config_with(data_dir: &str) -> Config {
        Config {
            corpus: CorpusConfig {
                path: None,
                data_dir: PathBuf::from(data_dir),
                archive: None,
            },
            ..Config::default()
        }
    }

    #[test]
    fn the_corpus_flag_overrides_the_configured_path() {
        let config = config_with("/tmp/yunjian-data");
        let corpus = corpus_config(&config, Some(Path::new("/tmp/explicit.db")));
        assert_eq!(corpus.path.as_deref(), Some(Path::new("/tmp/explicit.db")));
        assert_eq!(corpus.data_dir, PathBuf::from("/tmp/yunjian-data"));
    }

    #[test]
    fn without_the_flag_the_configured_corpus_survives() {
        let mut config = config_with("/tmp/yunjian-data");
        config.corpus.path = Some(PathBuf::from("/tmp/from-config.db"));
        let corpus = corpus_config(&config, None);
        assert_eq!(
            corpus.path.as_deref(),
            Some(Path::new("/tmp/from-config.db"))
        );
    }

    #[test]
    fn status_resolves_the_first_two_levels_only() {
        let explicit = resolved_corpus_file(&CorpusConfig {
            path: Some(PathBuf::from("/tmp/explicit.db")),
            data_dir: PathBuf::from("/tmp/data"),
            archive: None,
        });
        assert_eq!(explicit, PathBuf::from("/tmp/explicit.db"));
        let materialized = resolved_corpus_file(&CorpusConfig {
            path: None,
            data_dir: PathBuf::from("/tmp/data"),
            archive: None,
        });
        assert_eq!(materialized, PathBuf::from("/tmp/data/corpus.db"));
    }

    #[test]
    fn status_reports_a_missing_corpus_without_materializing_anything() {
        let directory = std::env::temp_dir().join(format!(
            "yunjian-cli-status-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let failed = run(
            &Command::Corpus {
                action: CorpusAction::Status,
            },
            &config_with(&directory.display().to_string()),
            None,
        )
        .err()
        .expect("缺语料时 status 必须失败");
        assert_eq!(failed.exit, Exit::CorpusUnavailable);
        assert!(
            failed.failure.render().contains("corpus fetch"),
            "必须点名取语料的命令：{}",
            failed.failure.render()
        );
        // 没有副作用：一条查看状态的命令不该建目录、也不该去解压归档。
        assert!(
            !directory.exists(),
            "status 不该创建 {}",
            directory.display()
        );
    }

    #[test]
    fn ai_cache_purge_uses_app_data_and_preserves_shipped_rows() {
        let directory = std::env::temp_dir().join(format!(
            "yunjian-cli-ai-cache-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let mut config = Config::default();
        config.app.data_dir = directory.clone();
        let cache = yunjian_ai::AppreciationCache::open(&directory, "corpus-v1", 8)
            .expect("初始化赏析缓存");
        let connection = Connection::open(cache.path()).expect("打开缓存数据库");
        connection
            .execute(
                "INSERT INTO appreciation_shipped (stable_id, template_version, model, model_license, grounding_digest, text, generated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params!["poem-shipped", "1.0.0", "open-model", "MIT", "digest", "内置", 1],
            )
            .expect("写随包测试行");
        connection
            .execute(
                "INSERT INTO appreciation_cache (key, stable_id, provider, model, template_version, corpus_version, grounding_digest, text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![&[1_u8; 32], "poem-local", "openai", "user-model", "1.0.0", "corpus-v1", "digest", "本地", 1],
            )
            .expect("写本地测试行");

        let produced = match run(
            &Command::Ai {
                action: AiAction::Cache {
                    action: AiCacheAction::Purge(AiCachePurgeArgs {
                        template: None,
                        poem: None,
                        all: true,
                    }),
                },
            },
            &config,
            None,
        ) {
            Ok(produced) => produced,
            Err(_) => panic!("执行缓存清理失败"),
        };

        let Body::Ok(_) = super::execute(
            &Command::Ai {
                action: AiAction::Cache {
                    action: AiCacheAction::Purge(AiCachePurgeArgs {
                        template: None,
                        poem: None,
                        all: true,
                    }),
                },
            },
            &config,
            None,
        )
        .body
        else {
            panic!("CLI 清理应返回成功载荷");
        };
        assert_eq!(produced.data["removed"], serde_json::json!(1));
        let counts = cache.counts().expect("读取清理后统计");
        assert_eq!(counts.local, 0);
        assert_eq!(counts.shipped, 1);
        drop(connection);
        drop(cache);
        let _ = std::fs::remove_dir_all(directory);
    }
}
