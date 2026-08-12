//! 云笺命令行。
//!
//! # 两条流，各有其主
//!
//! **stdout 只放结果**：人类可读文本或 `--json` 的一行信封，且只经
//! [`present`] 这一个出口。**stderr 只放日志**：进度、警告、失败原因全部走 `tracing`。
//!
//! 这不是风格偏好。同一个二进制还要承载 `yunjian mcp` 的 stdio 服务端，而 MCP 规范要求
//! stdout 上只能有协议帧；`yunjian search x --json | jq` 之所以永远不会被日志破坏，与
//! MCP 会话之所以不会被日志毁掉，是同一条约束的两个面。工作区级的 `print_stdout = "deny"`
//! 与 `disallowed-methods` 把它变成了编译期门禁，而不是靠人记住。
//!
//! # 入口顺序
//!
//! `main` 里的前两件事固定是 [`yunjian_core::init_config`] 然后
//! [`yunjian_core::init_logger`]，顺序不可换——没有配置就不知道日志级别与目录。
//! `init_logger` 返回的 `WorkerGuard` 必须活到进程结束，否则写文件的后台线程会被提前停掉，
//! 缓冲区里的日志随之丢失。因此 [`run`] 把它绑成局部变量并在函数末尾自然析构，
//! 而**退出码由返回值交给 `main`**：`std::process::exit` 不跑析构函数，在 `run` 内部直接
//! 退出等于把日志和 stdout 缓冲一起丢掉。
//!
//! # 退出码
//!
//! 见 [`exit`]：0 成功、1 无结果、2 用法错误、3 语料不可用。

#![warn(missing_docs)]

pub mod cli;
pub mod command;
pub mod envelope;
pub mod exit;
#[cfg(feature = "mcp")]
pub mod mcp_install;
pub mod output;
pub mod present;
pub mod provision;

use clap::Parser;
use cli::{Cli, Global};
use command::{Body, Report};
use envelope::{Envelope, ErrorCode, Failure};
use exit::Exit;
use yunjian_core::{Config, LoggerConfig, init_config, init_logger, init_stdio_logger};

/// 配置发现与用户配置目录使用的应用名。
pub const APP: &str = "yunjian";

/// 解析参数、初始化运行环境、执行子命令，返回进程退出码。
///
/// 不在内部调用 `std::process::exit`：那会跳过 `WorkerGuard` 的析构，把尚未落盘的日志丢掉。
#[must_use]
pub fn run() -> i32 {
    let cli = Cli::parse();
    let command_name = cli.command.name();

    let config = match init_config(cli.global.config.clone(), APP) {
        Ok(config) => config,
        Err(error) => {
            // 配置失败时还没有日志设施，而诊断不能写 stdout。于是先用默认配置把日志装起来
            // ——默认目录不可写时 `init_logger` 自己会降级成只写 stderr，这条路径因此总能出声。
            let _guard = init_logger(&LoggerConfig::default()).ok().flatten();
            tracing::error!(error = %error, "配置初始化失败");
            return emit(
                &Report {
                    command: command_name,
                    exit: Exit::Usage,
                    warnings: Vec::new(),
                    body: Body::Failed(
                        Failure::new(ErrorCode::Usage, format!("配置初始化失败：{error}"))
                            .with_hint("检查 --config 指向的文件，或删掉它改用默认配置"),
                    ),
                    human: Vec::new(),
                },
                cli.global.json,
            );
        }
    };

    let logger = logger_config(config, &cli.global);
    let logger_result = if is_mcp(&cli.command) {
        init_stdio_logger(&logger)
    } else {
        init_logger(&logger)
    };
    let Ok(_guard) = logger_result else {
        // 走到这里意味着进程里已经有全局订阅器了，在一个刚启动的可执行文件里不可能发生。
        // 既然日志设施状态不明，只报信封、不试图记录。
        return emit(
            &Report {
                command: command_name,
                exit: Exit::Usage,
                warnings: Vec::new(),
                body: Body::Failed(Failure::new(
                    ErrorCode::Usage,
                    "日志初始化失败：进程内已存在全局订阅器",
                )),
                human: Vec::new(),
            },
            cli.global.json,
        );
    };
    tracing::debug!(
        command = command_name,
        json = cli.global.json,
        voice = %voice_backend(),
        "云笺命令行启动"
    );

    #[cfg(feature = "mcp")]
    if let cli::Command::Mcp(args) = &cli.command
        && args.action.is_none()
    {
        return run_mcp(config, cli.global.corpus.as_deref(), args);
    }

    let report = command::execute(&cli.command, config, cli.global.corpus.as_deref());
    emit(&report, cli.global.json)
}

/// 本次运行会不会占住 stdout 当协议流。
///
/// **只有起服务那一种形态算**。`yunjian mcp install` 写完文件就退出，它的 stdout 是给人
/// 或给 `jq` 看的普通结果；给它装上 stdio 专用的日志订阅器会让终端里的输出无端失去颜色，
/// 而真正需要那份订阅器的是承载协议流的那条路径。
fn is_mcp(command: &cli::Command) -> bool {
    #[cfg(feature = "mcp")]
    if matches!(command, cli::Command::Mcp(args) if args.action.is_none()) {
        return true;
    }
    false
}

#[cfg(feature = "mcp")]
fn run_mcp(config: &Config, corpus_override: Option<&std::path::Path>, args: &cli::McpArgs) -> i32 {
    use yunjian_core::{CORPUS_FILE_NAME, CorpusHandle, Yunjian};
    use yunjian_mcp::YunjianServer;

    let mut corpus = config.corpus.clone();
    if let Some(path) = corpus_override {
        corpus.path = Some(path.to_path_buf());
    }
    let path = corpus
        .path
        .clone()
        .unwrap_or_else(|| corpus.data_dir.join(CORPUS_FILE_NAME));
    let (server, corpus_version) = if path.is_file() {
        corpus.path = Some(path);
        match CorpusHandle::open(&corpus) {
            Ok(handle) => {
                let corpus_version = handle.meta().corpus_version.clone();
                (YunjianServer::new(Yunjian::new(handle)), corpus_version)
            }
            Err(error) => {
                tracing::warn!(error = %error, "语料库不可用，MCP 将以缺语料模式启动");
                (YunjianServer::without_corpus(), "unavailable".to_owned())
            }
        }
    } else {
        tracing::warn!(corpus = %path.display(), "未找到语料库，MCP 将以缺语料模式启动");
        (YunjianServer::without_corpus(), "unavailable".to_owned())
    };
    let server = configure_mcp_ai(server, config, &corpus_version);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = %error, "创建 MCP 异步运行时失败");
            return Exit::Usage.code();
        }
    };
    #[cfg(feature = "mcp-http")]
    if args.http.http {
        return runtime.block_on(serve_mcp_http(&args.http, server));
    }
    #[cfg(not(feature = "mcp-http"))]
    let _unused = args;

    match runtime.block_on(yunjian_mcp::serve_stdio(server)) {
        Ok(()) => Exit::Success.code(),
        Err(error) => {
            tracing::error!(error = %error, "MCP stdio 服务异常结束");
            Exit::Usage.code()
        }
    }
}

#[cfg(feature = "mcp")]
fn configure_mcp_ai(
    server: yunjian_mcp::YunjianServer,
    config: &Config,
    corpus_version: &str,
) -> yunjian_mcp::YunjianServer {
    use std::sync::Arc;
    use yunjian_ai::{
        AiProvider, AppreciationCache, DEFAULT_APPRECIATION_CACHE_CAPACITY, GenAiProvider,
        GenAiProviderConfig, KeyStore, KeyStoreConfig, NullProvider, ProviderKind,
    };
    use yunjian_core::config::PROVIDER_NONE;

    let cache = AppreciationCache::open(
        &config.app.data_dir,
        corpus_version,
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )
    .map(Arc::new)
    .map_err(|error| {
        tracing::warn!(error = %error, "赏析缓存不可用，MCP AI 赏析将不缓存");
    })
    .ok();

    let without_provider = |server: yunjian_mcp::YunjianServer, model: String| {
        let provider: Arc<dyn AiProvider> = Arc::new(
            NullProvider::new("unconfigured")
                .unwrap_or_else(|_| unreachable!("内置供应商标识恒为合法 ASCII")),
        );
        server.with_ai(provider, cache.clone(), model)
    };

    if config.ai.provider == PROVIDER_NONE {
        return without_provider(server, String::new());
    }
    let kind = match ProviderKind::parse(&config.ai.provider) {
        Ok(kind) => kind,
        Err(error) => {
            tracing::warn!(error = %error, "AI 服务商配置无效，MCP AI 工具将提示重新配置");
            return without_provider(server, String::new());
        }
    };
    let model = config
        .ai
        .model
        .clone()
        .unwrap_or_else(|| kind.default_model().to_owned());
    let mut provider_config = GenAiProviderConfig::new(kind);
    if let Some(endpoint) = &config.ai.endpoint {
        provider_config = provider_config.with_base_url(endpoint.clone());
    }
    if config.ai.model.is_some() {
        provider_config = provider_config.with_model_override(model.clone());
    }
    let store = match KeyStore::open(KeyStoreConfig::default()) {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(error = %error, "钥匙串不可用，MCP AI 工具将提示配置密钥");
            return without_provider(server, model);
        }
    };
    let provider = match GenAiProvider::from_keystore(provider_config, &store) {
        Ok(provider) => provider,
        Err(yunjian_core::Error::AiKeyNotConfigured { .. }) => {
            return without_provider(server, model);
        }
        Err(error) => {
            tracing::warn!(error = %error, "AI 服务商初始化失败，MCP AI 工具将提示重新配置");
            return without_provider(server, model);
        }
    };
    let provider: Arc<dyn AiProvider> = Arc::new(provider);
    server.with_ai(provider, cache, model)
}

/// 起 Streamable HTTP 传输，并把 Ctrl-C 接到关停用的 cancellation token 上。
///
/// **拒绝绑定与服务失败必须是两个不同的日志与两次不同的判断**：前者是用户要改命令（少写了
/// `--allow-remote`），后者是环境问题（端口被占、token 目录不可写）。两者都返回非零码，
/// 但把它们合成一句「启动失败」会让第一种情形的用户去查端口。
#[cfg(feature = "mcp-http")]
async fn serve_mcp_http(
    options: &yunjian_mcp::http::HttpOptions,
    server: yunjian_mcp::YunjianServer,
) -> i32 {
    use tokio_util::sync::CancellationToken;
    use yunjian_mcp::http::{EXIT_HTTP_FAILED, EXIT_REMOTE_BIND_REFUSED, HttpServer};

    if let Err(refusal) = options.check_bind() {
        tracing::error!(error = %refusal, "拒绝启动 MCP HTTP 服务");
        return EXIT_REMOTE_BIND_REFUSED;
    }

    let cancellation = CancellationToken::new();
    let bound = match HttpServer::bind(options, cancellation.clone()).await {
        Ok(bound) => bound,
        Err(error) => {
            tracing::error!(error = %error, "MCP HTTP 服务启动失败");
            return EXIT_HTTP_FAILED;
        }
    };
    tracing::info!(
        endpoint = %bound.endpoint(),
        "客户端请从 token 文件读取 bearer token；命令行与环境变量都不接受它"
    );

    let signalled = cancellation.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("收到中断信号，正在关停 MCP HTTP 服务");
            signalled.cancel();
        }
    });

    match bound.serve(server).await {
        Ok(()) => Exit::Success.code(),
        Err(error) => {
            tracing::error!(error = %error, "MCP HTTP 服务异常结束");
            EXIT_HTTP_FAILED
        }
    }
}

/// 把命令行给出的日志级别叠加到配置上。
///
/// `--log-level` 压过 `config.toml`，但**不**压过 `RUST_LOG`——那是 `init_logger` 的既定
/// 语义（`RUST_LOG` 存在时整条过滤器都由它决定），在这里改会让两个入口各说一套。
fn logger_config(config: &Config, global: &Global) -> LoggerConfig {
    let mut logger = config.logger.clone();
    if let Some(level) = global.log_level {
        logger.level = level.as_key().to_owned();
    }
    logger
}

/// 写出结果并返回退出码。
///
/// 结果走 stdout，诊断走 stderr。两者顺序无关紧要，因为它们是两个流；重要的是**没有一个
/// 字节走错流**。
fn emit(report: &Report, json: bool) -> i32 {
    for warning in &report.warnings {
        tracing::warn!(
            code = ?warning.code,
            message = %warning.message,
            "命令降级"
        );
    }
    if let Body::Failed(failure) = &report.body {
        tracing::error!(
            code = ?failure.code,
            message = %failure.render(),
            "命令失败"
        );
    }

    if json {
        present::payload(&envelope(report).to_json_line());
    } else {
        for line in &report.human {
            present::line(line);
        }
    }

    if let Err(error) = present::flush() {
        tracing::warn!(error = %error, "刷新 stdout 失败");
    }
    report.exit.code()
}

fn envelope(report: &Report) -> Envelope {
    let envelope = match &report.body {
        Body::Ok(data) => Envelope::ok(report.command, data.clone()),
        Body::Empty(data) => Envelope::empty(report.command, data.clone()),
        Body::Failed(failure) => Envelope::failed(report.command, failure.clone()),
    };
    envelope.with_warnings(report.warnings.clone())
}

/// 语音后端的诊断串。
///
/// 返回值只进日志，但这次调用必须留着：它是 `--features voice` 下 onnxruntime 真的被链接
/// 进来的证据。去掉它，链接器就会把整个原生依赖丢弃，`ldd` 断言与安装包体积测量都会变成
/// 空话。
#[must_use]
pub fn voice_backend() -> String {
    match yunjian_voice::backend_version() {
        Some(version) => format!("sherpa-onnx {version}"),
        None => "disabled".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{envelope, logger_config, voice_backend};
    use crate::cli::{Global, LogLevel};
    use crate::command::{Body, Report};
    use crate::envelope::{ErrorCode, Failure, Status, Warning, WarningCode};
    use crate::exit::Exit;
    use yunjian_core::Config;

    fn global(log_level: Option<LogLevel>) -> Global {
        Global {
            config: None,
            corpus: None,
            log_level,
            json: false,
        }
    }

    #[test]
    fn the_log_level_flag_overrides_the_configured_level() {
        let mut config = Config::default();
        config.logger.level = "info".to_owned();
        assert_eq!(
            logger_config(&config, &global(Some(LogLevel::Trace))).level,
            "trace"
        );
        assert_eq!(logger_config(&config, &global(None)).level, "info");
    }

    #[test]
    fn the_envelope_mirrors_the_report_status_and_keeps_warnings() {
        let report = Report {
            command: "search",
            exit: Exit::NoResults,
            warnings: vec![Warning::new(WarningCode::DegradedPlan, "退化了")],
            body: Body::Empty(serde_json::json!({"hits": []})),
            human: Vec::new(),
        };
        let envelope = envelope(&report);
        assert_eq!(envelope.status, Status::Empty);
        assert_eq!(envelope.warnings.len(), 1);
        assert_eq!(envelope.command, "search");
    }

    #[test]
    fn a_failed_report_becomes_a_failed_envelope_without_data() {
        let report = Report {
            command: "show",
            exit: Exit::CorpusUnavailable,
            warnings: Vec::new(),
            body: Body::Failed(Failure::new(ErrorCode::CorpusUnavailable, "语料库缺失")),
            human: Vec::new(),
        };
        let envelope = envelope(&report);
        assert_eq!(envelope.status, Status::Error);
        assert!(envelope.data.is_none());
        assert!(envelope.error.is_some());
    }

    #[test]
    fn the_voice_backend_probe_stays_callable_so_the_native_link_survives() {
        let reported = voice_backend();
        assert!(
            reported == "disabled" || reported.starts_with("sherpa-onnx "),
            "语音后端诊断串形态意外：{reported}"
        );
    }
}
