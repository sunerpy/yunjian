//! 日志初始化：全进程唯一入口 [`init_logger`]，控制台输出固定写 stderr。
//!
//! # 为什么控制台必须是 stderr
//!
//! `yunjian` 这一个二进制同时承载 MCP stdio 服务器，stdout 是协议流本身。MCP 规范要求
//! 服务器「绝不能向 stdout 写入任何非 MCP 消息的内容」，一行日志就会让整条会话解析失败。
//! 因此本模块**没有**、也不会有切换到 stdout 的开关：[`LoggerConfig`] 里刻意不存在这个
//! 字段，那不是遗漏。同一份 `init_logger` 也供 CLI 与桌面外壳使用，不引入 `tauri-plugin-log`。
//!
//! # 组成
//!
//! - 过滤：`RUST_LOG` 优先，缺省回落到 `cfg.level`；装在 [`reload::Layer`] 里，
//!   因此可以用 [`set_log_level`] 在不重启进程的前提下改级别。
//! - 控制台层：写 stderr，仅在连着终端时上色。
//! - 文件层：按天滚动，永不写 ANSI 转义序列。
//! - 时间戳：本地时区 RFC3339，取不到本地偏移时退回 UTC。
//!
//! # 用法
//!
//! 在每个二进制的 `main` 里尽早调用一次，并把返回的 `WorkerGuard` 绑定成活到进程结束的
//! 变量——它一被丢弃，后台写文件的线程就会停止，缓冲区里的日志随之丢失。
//!
//! ```no_run
//! use yunjian_core::{init_config, init_logger};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = init_config(None, "yunjian")?;
//! let _guard = init_logger(&config.logger)?;
//! tracing::info!("云笺启动");
//! # Ok(())
//! # }
//! ```

use crate::config::LoggerConfig;
use anyhow::{Context, Result, anyhow};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::OnceLock;
use time::UtcOffset;
use time::format_description::well_known::Rfc3339;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::layer::{Layered, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, reload};

/// 级别名与过滤器的唯一映射表，[`parse_level`] 与 [`level_name`] 双向共用，
/// 以此保证两个方向不会各自漂移。
const LEVELS: [(&str, LevelFilter); 6] = [
    ("off", LevelFilter::OFF),
    ("error", LevelFilter::ERROR),
    ("warn", LevelFilter::WARN),
    ("info", LevelFilter::INFO),
    ("debug", LevelFilter::DEBUG),
    ("trace", LevelFilter::TRACE),
];

/// 配置里写了无法识别的级别时归一到此：一个错别字不该让日志整体静默。
const FALLBACK_LEVEL: &str = "info";

/// 尚未初始化时 [`current_log_level`] 的回答——没有订阅器，任何事件都不会被记录。
const UNINITIALIZED_LEVEL: &str = "off";

/// 探测目录可写性的临时文件名，创建后立即删除。
const WRITE_PROBE: &str = ".yunjian-log-writable";

/// 装好过滤层之后的订阅器类型，输出层要按它来装箱。
type Filtered = Layered<reload::Layer<EnvFilter, Registry>, Registry>;

type BoxedLayer = Box<dyn Layer<Filtered> + Send + Sync>;

static RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// 时间戳采用的时区来源，会作为字段写进初始化日志，便于解释一份 UTC 时间戳的来历。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Local,
    UtcFallback,
}

impl Zone {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::UtcFallback => "utc-fallback",
        }
    }
}

/// 严格解析级别名；无法识别时返回 `None`，由调用方决定是归一还是报错。
fn parse_level(input: &str) -> Option<LevelFilter> {
    let lowered = input.trim().to_ascii_lowercase();
    let wanted = match lowered.as_str() {
        "warning" => "warn",
        other => other,
    };
    LEVELS
        .iter()
        .find(|(name, _)| *name == wanted)
        .map(|(_, level)| *level)
}

fn level_name(level: LevelFilter) -> &'static str {
    LEVELS
        .iter()
        .find(|(_, candidate)| *candidate == level)
        .map(|(name, _)| *name)
        .unwrap_or(FALLBACK_LEVEL)
}

/// 把配置里的级别归一成映射表里的写法，无法识别时退回 [`FALLBACK_LEVEL`]。
///
/// 不能把原始字符串直接交给 `EnvFilter`：凡是解析不成级别的裸词都被当作**目标名**，
/// `EnvFilter::new("verbose")` 得到的是 `verbose=trace`，本工程所有 target 全被挡掉；
/// 空字符串更是直接退化成 `error`。配置里一个错别字就换来一片静默。
fn normalize_level(configured: &str) -> &'static str {
    parse_level(configured)
        .map(level_name)
        .unwrap_or(FALLBACK_LEVEL)
}

fn utc_timer() -> OffsetTime<Rfc3339> {
    OffsetTime::new(UtcOffset::UTC, Rfc3339)
}

/// 取本地时区的 RFC3339 计时器，拿不到本地偏移时退回 UTC。
///
/// 失败是常态而非异常：`time` 在多线程进程里拒绝推断本地偏移，容器里也常常没有时区数据。
/// 所以这里绝不 `expect`，只是把用了哪一种记录下来。
fn rfc3339_timer() -> (OffsetTime<Rfc3339>, Zone) {
    match OffsetTime::local_rfc_3339() {
        Ok(timer) => (timer, Zone::Local),
        Err(_) => (utc_timer(), Zone::UtcFallback),
    }
}

/// 建出日志目录并确认它真的可写。
///
/// `tracing_appender::rolling::daily` 内部是 `.expect("initializing rolling file appender
/// failed")`，目录存在但不可写时会直接 panic。这里用一个立即删除的探针文件把那条 panik
/// 换成可降级的 `Err`：日志目录不可写不该让用户连诗都读不了。
fn probe_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(WRITE_PROBE);
    std::fs::File::create(&probe)?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn console_layer(json: bool, timer: OffsetTime<Rfc3339>) -> BoxedLayer {
    // stdout 属于 MCP 协议流，控制台层永远只能写 stderr。
    let layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_timer(timer)
        .with_target(true)
        // 只有连着终端才上色：MCP 客户端会把 stderr 收进日志文件，转义序列在那里是噪声。
        .with_ansi(std::io::stderr().is_terminal());
    if json {
        layer.json().with_current_span(true).boxed()
    } else {
        layer.boxed()
    }
}

fn file_layer(cfg: &LoggerConfig, timer: OffsetTime<Rfc3339>) -> (BoxedLayer, WorkerGuard) {
    let appender = tracing_appender::rolling::daily(&cfg.dir, &cfg.file_prefix);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let layer = fmt::layer()
        .with_writer(writer)
        // 文件里绝不写 ANSI 转义序列：`grep`、`less` 与日志采集器都会被它污染。
        .with_ansi(false)
        .with_timer(timer)
        .with_target(true);
    let layer = if cfg.json {
        layer.json().with_current_span(true).boxed()
    } else {
        layer.boxed()
    };
    (layer, guard)
}

/// 安装全局订阅器。整个进程只能调用一次，重复调用返回 `Err`。
///
/// 返回的 `WorkerGuard` 必须在 `main` 里绑定成 `let _guard` 活到进程结束；提前丢弃会停掉
/// 写文件的后台线程，缓冲中的日志随之丢失。返回 `None` 表示日志目录不可用，本次运行只有
/// stderr 输出——这一降级会以 `warn` 记录下来，而不是静默发生。
///
/// # Errors
///
/// 已经存在全局订阅器（含重复调用本函数）时返回 `Err`，绝不静默覆盖。
pub fn init_logger(cfg: &LoggerConfig) -> Result<Option<WorkerGuard>, Box<dyn std::error::Error>> {
    // 先解析时区偏移再创建 appender：`time` 在多线程进程里拒绝推断本地偏移，而下面的
    // 非阻塞 writer 会起后台线程，顺序颠倒就永远只能拿到 UTC。
    let (timer, zone) = rfc3339_timer();

    let configured = normalize_level(&cfg.level);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(configured));
    let (filter_layer, handle) = reload::Layer::<EnvFilter, Registry>::new(filter);

    let mut layers: Vec<BoxedLayer> = vec![console_layer(cfg.json, timer.clone())];
    let (guard, dir_error) = match probe_dir(&cfg.dir) {
        Ok(()) => {
            let (layer, guard) = file_layer(cfg, timer);
            layers.push(layer);
            (Some(guard), None)
        }
        Err(err) => (None, Some(err)),
    };

    Registry::default()
        .with(filter_layer)
        .with(layers)
        .try_init()?;
    RELOAD_HANDLE
        .set(handle)
        .map_err(|_| "日志已初始化，init_logger() 只能调用一次")?;

    if let Some(err) = dir_error {
        tracing::warn!(
            dir = %cfg.dir.display(),
            error = %err,
            "日志目录不可写，本次运行仅输出到 stderr"
        );
    }
    tracing::info!(
        level = current_log_level(),
        json = cfg.json,
        timezone = zone.as_str(),
        to_file = guard.is_some(),
        dir = %cfg.dir.display(),
        "日志已初始化"
    );
    Ok(guard)
}

/// 在不重启进程的前提下调整日志级别。
///
/// 会**替换**整个过滤器，因此启动时来自 `RUST_LOG` 的分目标指令（如 `tower=warn`）在调用后
/// 失效——这是显式覆盖的应有语义。
///
/// # Errors
///
/// 级别名无法识别，或 [`init_logger`] 尚未调用时返回 `Err`。级别名一律严格校验：运行时接口
/// 的入参来自用户，把 `verbose` 悄悄当成 `info` 只会让人以为设置生效了。
pub fn set_log_level(level: &str) -> Result<()> {
    let parsed = parse_level(level).ok_or_else(|| {
        let names: Vec<&str> = LEVELS.iter().map(|(name, _)| *name).collect();
        anyhow!("未知日志级别 `{level}`，可选值：{}", names.join(" / "))
    })?;
    let handle = RELOAD_HANDLE
        .get()
        .context("日志未初始化，请先调用 init_logger()")?;
    let name = level_name(parsed);
    handle.modify(|filter| *filter = EnvFilter::new(name))?;
    tracing::info!(level = name, "已在运行时调整日志级别");
    Ok(())
}

/// 当前放行的最详细级别。
///
/// [`init_logger`] 之前返回 `"off"`：那时没有任何订阅器，事件确实不会被记录。
#[must_use]
pub fn current_log_level() -> String {
    let Some(handle) = RELOAD_HANDLE.get() else {
        return UNINITIALIZED_LEVEL.to_owned();
    };
    match handle.with_current(EnvFilter::max_level_hint) {
        // `None` 意味着过滤器不设上限，等价于放行到 trace。
        Ok(hint) => level_name(hint.unwrap_or(LevelFilter::TRACE)).to_owned(),
        Err(_) => UNINITIALIZED_LEVEL.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tracing_subscriber::fmt::format::Writer;
    use tracing_subscriber::fmt::time::FormatTime;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("系统时间晚于 UNIX 纪元")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "yunjian-logger-{tag}-{}-{nanos}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parse_level_accepts_the_documented_names_only() {
        assert_eq!(parse_level("off"), Some(LevelFilter::OFF));
        assert_eq!(parse_level("error"), Some(LevelFilter::ERROR));
        assert_eq!(parse_level("warn"), Some(LevelFilter::WARN));
        assert_eq!(parse_level("warning"), Some(LevelFilter::WARN));
        assert_eq!(parse_level("info"), Some(LevelFilter::INFO));
        assert_eq!(parse_level("debug"), Some(LevelFilter::DEBUG));
        assert_eq!(parse_level("trace"), Some(LevelFilter::TRACE));
        assert_eq!(parse_level("  DEBUG \n"), Some(LevelFilter::DEBUG));

        for rejected in ["", "verbose", "warn=debug", "информация", "infos"] {
            assert_eq!(parse_level(rejected), None, "不该识别 `{rejected}`");
        }
    }

    #[test]
    fn level_name_round_trips_through_parse_level() {
        for (name, level) in LEVELS {
            assert_eq!(level_name(level), name);
            assert_eq!(parse_level(name), Some(level));
        }
    }

    #[test]
    fn unknown_configured_level_falls_back_to_info_instead_of_silence() {
        assert_eq!(normalize_level("verbose"), FALLBACK_LEVEL);
        assert_eq!(normalize_level(""), FALLBACK_LEVEL);
        assert_eq!(normalize_level("TRACE"), "trace");
        assert_eq!(normalize_level("warning"), "warn");
    }

    /// [`normalize_level`] 存在的唯一理由，用可执行断言钉住而不是写在注释里：`EnvFilter`
    /// 把解析不成级别的裸词当作**目标名**，直接透传配置里的错别字会让本工程所有 target
    /// 静默。上游哪天改了这个行为，本测试会红，届时才可以考虑去掉归一化。
    #[test]
    fn passing_a_misspelled_level_straight_through_would_silence_everything() {
        assert_eq!(EnvFilter::new("verbose").to_string(), "verbose=trace");
        assert_eq!(EnvFilter::new("").to_string(), "error");

        assert_eq!(
            EnvFilter::new(normalize_level("verbose")).to_string(),
            "info"
        );
        assert_eq!(EnvFilter::new(normalize_level("")).to_string(), "info");
    }

    /// UTC 兜底必须真的能用：这条断言直接调用兜底分支并检查它输出零偏移的 RFC3339。
    #[test]
    fn utc_fallback_formats_a_zero_offset_timestamp() {
        let mut rendered = String::new();
        utc_timer()
            .format_time(&mut Writer::new(&mut rendered))
            .expect("UTC 兜底计时器必须能格式化当前时间");

        assert!(
            rendered.ends_with('Z'),
            "UTC 兜底应当输出零偏移时间戳: {rendered}"
        );
        assert!(
            rendered.contains('T') && rendered.len() >= 20,
            "应当是 RFC3339 形状: {rendered}"
        );
    }

    /// 本地偏移取不到时不得 panic，只是换成 UTC。
    #[test]
    fn timer_resolution_never_panics() {
        let (_, zone) = rfc3339_timer();
        assert!(matches!(zone, Zone::Local | Zone::UtcFallback));
        assert_eq!(Zone::UtcFallback.as_str(), "utc-fallback");
    }

    #[test]
    fn probe_dir_creates_missing_directories_and_leaves_nothing_behind() {
        let dir = TempDir::new("probe-ok");
        let nested = dir.path().join("a/b/c");

        probe_dir(&nested).expect("可写目录应当探测成功");

        assert!(nested.is_dir(), "缺失的目录应当被建出来");
        assert!(!nested.join(WRITE_PROBE).exists(), "探针文件必须被删掉");
    }

    /// 目录位置被一个普通文件占着时必须报错而不是 panic——这是 `init_logger`
    /// 得以降级为「仅 stderr」的前提。
    #[test]
    fn probe_dir_reports_an_unusable_location() {
        let dir = TempDir::new("probe-bad");
        let occupied = dir.path().join("not-a-dir");
        fs::write(&occupied, b"x").expect("写占位文件");

        assert!(probe_dir(&occupied).is_err());
    }

    /// 本 crate 的测试二进制里只有这一条用例可以安装全局订阅器。
    ///
    /// 它同时覆盖三件事：`init_logger` 成功、级别可在运行时改、非法级别被拒且不破坏现状。
    #[test]
    fn runtime_level_change_takes_effect() {
        let dir = TempDir::new("reload");
        let cfg = LoggerConfig {
            level: "info".to_owned(),
            json: false,
            dir: dir.path().to_path_buf(),
            file_prefix: "reload".to_owned(),
        };

        let guard = init_logger(&cfg).expect("首次初始化必须成功");
        assert!(guard.is_some(), "可写目录下必须挂上文件层");
        assert!(
            LEVELS.iter().any(|(name, _)| *name == current_log_level()),
            "初始级别必须是映射表里的名字: {}",
            current_log_level()
        );

        set_log_level("debug").expect("调到 debug");
        assert_eq!(current_log_level(), "debug");

        set_log_level("warn").expect("调回 warn");
        assert_eq!(current_log_level(), "warn");

        let err = set_log_level("verbose").expect_err("非法级别必须报错");
        assert!(
            format!("{err}").contains("verbose"),
            "报错要指出是哪个值: {err}"
        );
        assert_eq!(current_log_level(), "warn", "失败的调用不得改动现状");

        assert!(
            init_logger(&cfg).is_err(),
            "重复初始化必须报错，而不是静默安装第二个订阅器"
        );
    }
}
