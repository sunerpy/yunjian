//! 云笺桌面端外壳。
//!
//! # 这个 crate 只做外壳的事
//!
//! 它持有 Tauri 的窗口配置、日志引导与（后续 todo 接入的）IPC 命令表。
//! 检索、背诵、赏析的逻辑一律留在 `yunjian-core` 等领域 crate 里，
//! **`yunjian-core` 永远不知道 Tauri 的存在**——那是移动端逃生通道的前提。
//!
//! # 日志：与 CLI、MCP 共用同一套约定
//!
//! 这里用 [`yunjian_core::init_logger`] 而**不是** Tauri 官方的日志插件。
//! 三个入口（命令行、MCP stdio 服务端、桌面 GUI）因此共享同一份级别解析、同一份
//! 凭据脱敏、同一份滚动文件布局，控制台一律写 **stderr**。
//! 换成插件会得到第二套格式与第二套过滤语义，而两套日志约定的差异只会在
//! 排障最需要日志的时候暴露。
//!
//! 顺序与 CLI 一致且不可换：先 [`yunjian_core::init_config`]（没有配置就不知道级别与目录），
//! 再 `init_logger`。返回的 guard 必须活到进程结束，否则写文件的后台线程被提前停掉，
//! 缓冲区里的日志随之丢失——所以它在 [`run`] 里被绑成具名局部变量，
//! 而不是丢给 `_`（那会立刻析构）。
//!
//! # 窗口装饰按平台分叉
//!
//! 基础 `tauri.conf.json` 对 Windows 与 Linux 关掉原生装饰（`decorations: false`）以便自绘
//! 标题栏（todo 60）；macOS 由 `tauri.macos.conf.json` 覆盖回 `decorations: true` +
//! `titleBarStyle: "Overlay"`，否则红绿灯按钮会一起消失。
//!
//! **覆盖文件里的 `app.windows` 是整体替换而非逐字段合并**（`tauri-utils` 的
//! `config/parse.rs` 用 `json_patch::merge`，即 RFC 7396 JSON Merge Patch，数组被整体替换），
//! 所以它必须重述每一个几何字段。漏写一个就会在 macOS 上静默退回 Tauri 的默认值，
//! 而 Linux 与 Windows 上一切正常。`tests/window_config.rs` 把这条钉死。
//!
//! # 已记录在案的降级：Windows 11 Snap Layouts
//!
//! `decorations: false` 在 Windows 上唯一的代价是悬停最大化按钮时的 Snap Layouts 浮层
//! 失效（Aero Snap、缩放边框、阴影、Windows 11 圆角都**保留**）。这是 WebView2 侧的上游
//! 限制，没有干净的绕法；社区插件的「恢复」实现是合成一次 `Win`+`Z` 按键，在 Tauri
//! 自己注册的缩放边框子窗口存在时浮层会错位。**因此这条作为已接受的降级记录在此，
//! 不引入插件去解决它。**

#![warn(missing_docs)]

use yunjian_core::{Config, LoggerConfig, init_config, init_logger};

mod ipc;
pub mod voice_ipc;

/// 配置发现与用户配置目录使用的应用名。与命令行共用同一个名字，
/// 两个入口因此读同一份 `config.toml`。
pub const APP: &str = "yunjian";

/// 启动桌面端：先配置、再日志、最后拉起 Tauri。
///
/// # Panics
///
/// Tauri 构建失败（WebView 运行时缺失、系统缺少 GTK/WebKit 运行库）时 panic。
/// 这里刻意不吞：一个起不来的窗口没有可降级的形态，静默返回只会得到一个既没有窗口
/// 也没有解释的进程。
pub fn run() {
    let (config, config_error) = match init_config(None, APP) {
        Ok(config) => (config.clone(), None),
        Err(error) => (Config::default(), Some(error)),
    };
    let logger = config_error
        .as_ref()
        .map_or_else(|| config.logger.clone(), |_| LoggerConfig::default());
    let _log_guard = init_logger(&logger).ok().flatten();
    if let Some(error) = config_error {
        tracing::error!(error = %error, "配置初始化失败，改用默认配置继续启动");
    }

    tracing::info!(
        app = APP,
        version = env!("CARGO_PKG_VERSION"),
        "云笺桌面端启动"
    );

    let startup_config = config.clone();
    ipc::configure_builder(tauri::Builder::default(), config)
        .setup(move |_| {
            start_asset_sync(startup_config.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

fn start_asset_sync(config: Config) {
    let spawn = std::thread::Builder::new()
        .name("yunjian-assets".to_owned())
        .spawn(
            move || match yunjian_ai::sync_shipped_assets(config.corpus, config.app.data_dir) {
                Ok(assets) => tracing::info!(
                    corpus_version = %assets.corpus.meta().corpus_version,
                    seed_template_version = %assets.seed.template_version,
                    record_count = assets.seed.record_count,
                    stale_count = assets.seed.stale_count,
                    "语料与随包赏析种子已就绪"
                ),
                Err(error) => tracing::warn!(
                    error = %error,
                    "首启资产同步失败；已有语料与旧赏析种子保持不变"
                ),
            },
        );
    if let Err(error) = spawn {
        tracing::warn!(error = %error, "无法启动首启资产同步线程");
    }
}
