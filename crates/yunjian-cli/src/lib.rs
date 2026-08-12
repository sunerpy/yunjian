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
    if matches!(cli.command, cli::Command::Mcp) {
        return run_mcp(config, cli.global.corpus.as_deref());
    }

    let report = command::execute(&cli.command, config, cli.global.corpus.as_deref());
    emit(&report, cli.global.json)
}

fn is_mcp(command: &cli::Command) -> bool {
    #[cfg(feature = "mcp")]
    if matches!(command, cli::Command::Mcp) {
        return true;
    }
    false
}

#[cfg(feature = "mcp")]
fn run_mcp(config: &Config, corpus_override: Option<&std::path::Path>) -> i32 {
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
    let server = if path.is_file() {
        corpus.path = Some(path);
        match CorpusHandle::open(&corpus) {
            Ok(handle) => YunjianServer::new(Yunjian::new(handle)),
            Err(error) => {
                tracing::warn!(error = %error, "语料库不可用，MCP 将以缺语料模式启动");
                YunjianServer::without_corpus()
            }
        }
    } else {
        tracing::warn!(corpus = %path.display(), "未找到语料库，MCP 将以缺语料模式启动");
        YunjianServer::without_corpus()
    };

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
    match runtime.block_on(yunjian_mcp::serve_stdio(server)) {
        Ok(()) => Exit::Success.code(),
        Err(error) => {
            tracing::error!(error = %error, "MCP stdio 服务异常结束");
            Exit::Usage.code()
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
