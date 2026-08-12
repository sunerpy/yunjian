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

use yunjian_core::{LoggerConfig, init_config, init_logger};

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
    // guard 必须具名。写成 `let _ = ...` 会立刻析构，写文件的后台线程随之停掉，
    // 于是日志文件恒为空——而这种失效在 stderr 上看不出来。
    let _log_guard = match init_config(None, APP) {
        Ok(config) => init_logger(&config.logger).ok().flatten(),
        Err(error) => {
            // 配置失败时还没有日志设施，而 GUI 进程里 stdout 也不是诊断出口
            //（工作区级 `print_stdout = "deny"` 正是为此）。先用默认配置把日志装起来
            // ——默认目录不可写时 `init_logger` 自己会降级成只写 stderr，这条路径因此总能出声。
            let guard = init_logger(&LoggerConfig::default()).ok().flatten();
            tracing::error!(error = %error, "配置初始化失败，改用默认配置继续启动");
            guard
        }
    };

    tracing::info!(
        app = APP,
        version = env!("CARGO_PKG_VERSION"),
        "云笺桌面端启动"
    );

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
