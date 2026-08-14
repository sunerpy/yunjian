//! `xtask acceptance --platform <win|mac|linux> --set desktop`：桌面端真机验收。
//!
//! # 为什么这个子命令必须存在
//!
//! 绿色构建只证明**某个缺陷没有复现**，不证明产品能用。工作区里 1131 条测试全绿，
//! 而其中一条也不能回答「安装完之后，一个人双击图标，窗口会不会出现」。窗口出现、
//! 自绘标题栏的四个控件真的操作了窗口、双击真的最大化了**一次**、中文输入法往一个
//! 已有内容的检索框里打字不会把界面冻住——这些只在一个**交互式桌面会话**里成立或不
//! 成立。session 0 里跑得再绿也证不了 WebView 能显示。
//!
//! # 断言集在执行之前声明
//!
//! [`DECLARED`] 是一张编译期常量表。报告逐条对着它写，因此：
//!
//! - **提前退出无法被误当成成功**。跑到一半崩掉，剩下的条目仍然在报告里，verdict 是
//!   `NOT EXECUTED`，而不是从报告里消失。
//! - **没有未声明的断言**。运行期只能给已声明的 id 填 verdict，填一个表里没有的 id 是
//!   一个 `bail!`，而不是往报告里多长出一行。
//! - [`tests`] 里那条解析器断言把这两件事钉死。
//!
//! # 三种通道，各自能证明什么
//!
//! | 通道 | 工具 | 能证明 |
//! | --- | --- | --- |
//! | [`Channel::WebDriver`] | `tauri-driver` + `WebKitWebDriver` | DOM 层的事实：某个 `data-testid` 在不在、里面的文字是什么 |
//! | [`Channel::OsHarness`] | `enigo` 合成输入 + X11 属性查询 | 操作系统层的事实：窗口在不在、`_NET_WM_STATE` 是什么、点下去之后窗口真的变了 |
//! | [`Channel::Process`] | 进程与产物 | 产物存在、进程起得来、退得干净 |
//!
//! 三者不可互相替代。WebDriver 读不到「窗口真的被最大化了」（它只看得见 DOM），
//! OS 通道读不到「检索框里现在是哪几个字」（它只看得见像素）。因此每条断言在声明时
//! 就绑定了它的通道，验收时**不允许**换一条更容易的通道去满足它。
//!
//! # 为什么 `all_pass` 严格要求「零 FAIL 且零 NOT EXECUTED」
//!
//! 这个字段会被终验消费。如果 Windows 与 macOS 从未执行，而 `all_pass` 是 `true`，
//! 读到它的人会得出「三平台都过了」——那是一句假话，且是这份报告最容易造成的那句假话。
//! 所以它取最严格的语义，另有 `executed_pass` / `failed` / `not_executed` 三个计数
//! 让消费方看得到细节。
//!
//! **退出码与 `all_pass` 刻意不同义**：只有 `FAIL` 才让本命令非零退出。`NOT EXECUTED`
//! 是 harness 尽了责的结果（它如实说明了自己做不到什么），不是缺陷，因此退出 0。
//! 把「未执行」也判成非零会有一个很坏的后果——它会奖励**删掉**那些跑不了的断言。

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::verify_sources::emit;

/// 报告目录。
const REPORT_DIR: &str = "docs/reports";
/// 应用产物相对仓库根的路径。debug 产物即可：真机验收要证明的是「窗口出现、控件生效」，
/// 而这些与优化级别无关；release 构建在本机要多花十几分钟。
const APP_BINARY: &str = "target/debug/yunjian-desktop";
/// Tauri 配置，报告里的应用版本从它读，不从 `CARGO_PKG_VERSION` 猜——桌面端的版本号
/// 由 `tauri.conf.json` 的 `version` 定义，两者可以不同。
const TAURI_CONF: &str = "crates/yunjian-app/tauri.conf.json";

/// 自绘标题栏的几何常量，来源是 `app/src/styles.css` 与 `app/src/chrome/titlebar.css`：
/// `--titlebar-height: 2.5rem`、`.titlebar__button { width: 2.875rem }`、
/// `.titlebar__drag { padding-inline: var(--space-3) }`（`--space-3: 0.75rem`）。
/// 根字号 16 px，故 40 / 46 / 12 px。
///
/// **这些常量漂了不会静默通过**：三个按钮的断言判的是「点下去之后窗口状态真的变了」，
/// 坐标错了就点在空处，状态不变，断言变红。所以它们是可自证的，不是需要被信任的。
mod geometry {
    /// 标题栏高度，px。
    pub const TITLEBAR_HEIGHT: i32 = 40;
    /// 单个窗口控件宽度，px。
    pub const BUTTON_WIDTH: i32 = 46;
    /// 拖动区的行内内边距，px。标题文字自此开始。
    pub const DRAG_PADDING: i32 = 12;

    /// 标题栏竖直中线。
    pub const fn mid_y() -> i32 {
        TITLEBAR_HEIGHT / 2
    }

    /// 从右往左数第 `nth`（0 起）个控件的中心 x，`width` 是窗口宽度。
    /// 顺序与 `TitleBar.tsx` 一致：最小化、最大化、关闭。
    pub const fn button_center_x(width: i32, nth_from_right: i32) -> i32 {
        width - BUTTON_WIDTH * nth_from_right - BUTTON_WIDTH / 2
    }

    /// 关闭按钮是最右一个。
    pub const CLOSE_FROM_RIGHT: i32 = 0;
    /// 最大化按钮在中间。
    pub const MAXIMIZE_FROM_RIGHT: i32 = 1;
    /// 最小化按钮最左。
    pub const MINIMIZE_FROM_RIGHT: i32 = 2;

    /// 标题文字上的一点。文字是「云笺」两个字，`--text-sm: 0.875rem` = 14 px，
    /// 起点在 `DRAG_PADDING`，故第一个字覆盖约 12..26 px。
    pub const fn title_text_x() -> i32 {
        DRAG_PADDING + 6
    }

    /// 导航条高度：`padding: var(--space-2)` 上下各 8 px 加一行按钮，加下边框 1 px。
    /// 见 `app/src/App.tsx` 里那一行 `<nav>` 的内联样式。
    const NAV_HEIGHT: i32 = 41;

    /// 内容区里一个足够靠上的 y，落在检索框那一带。
    pub const fn content_top_y() -> i32 {
        TITLEBAR_HEIGHT + NAV_HEIGHT + 40
    }
}

/// 一条断言由哪种通道执行。声明时绑定，运行期不得更换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Channel {
    /// DOM 层事实，只能由真实 WebDriver 会话回答。
    WebDriver,
    /// 操作系统层事实：合成输入加窗口属性查询。
    OsHarness,
    /// 产物与进程层事实。
    Process,
}

/// 一条断言的裁决。**没有第四种取值**——「跑了但说不清」不是允许的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum Verdict {
    /// 执行了，且断言成立。
    Pass,
    /// 执行了，断言不成立。这是唯一让本命令非零退出的取值。
    Fail,
    /// 没有执行。必须附原因与「什么条件下能跑」。
    #[serde(rename = "NOT EXECUTED")]
    NotExecuted,
}

impl Verdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotExecuted => "NOT EXECUTED",
        }
    }
}

/// 一条声明的断言。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Declared {
    /// 稳定 id，报告与代码共用。
    pub(crate) id: &'static str,
    /// 中文说明，写给读报告的人。
    pub(crate) what: &'static str,
    /// 执行通道。
    pub(crate) channel: Channel,
    /// 是否要求截图。UI 断言一律要求——「界面上确实是这样」这句话只有图能作证。
    pub(crate) needs_screenshot: bool,
}

/// **断言集。在任何一次执行之前声明。**
///
/// 方案要求的最小集全部在此，逐条对应：安装包能跑与应用能启动、自绘标题栏与四个控件、
/// 双击最大化恰好一次、从标题文字拖拽、任务栏与托盘图标、语音一轮端到端成功（与降级
/// 路径**分开**）、中文输入法往已有内容的检索框输入、两字检索、首启语料物化、无 key 时
/// 随包赏析。
///
/// 语音那两条刻意是**两条**：写成「成功或降级」会被一个语音从未工作过的构建满足。
pub(crate) const DECLARED: &[Declared] = &[
    Declared {
        id: "artifact_present",
        what: "构建产物存在且可执行，动态库依赖全部可解析",
        channel: Channel::Process,
        needs_screenshot: false,
    },
    Declared {
        id: "installer_runs",
        what: "安装包（.deb / NSIS / .dmg）能安装并从安装后的路径启动",
        channel: Channel::Process,
        needs_screenshot: false,
    },
    Declared {
        id: "app_launches",
        what: "应用在交互式桌面会话里启动并映射出一个顶层窗口",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "custom_titlebar_rendered",
        what: "窗口没有窗口管理器绘制的边框（decorations: false 生效），标题栏由应用自绘",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "control_minimize_works",
        what: "点自绘标题栏的最小化按钮，窗口真的最小化",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "control_maximize_works",
        what: "点自绘标题栏的最大化按钮，窗口真的最大化",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "control_restore_works",
        what: "最大化后再点同一个按钮，窗口真的还原",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "control_close_works",
        what: "点自绘标题栏的关闭按钮，窗口关闭且进程退出",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "double_click_maximizes_exactly_once",
        what: "双击标题栏恰好最大化一次（自己再挂一个双击处理器会双切换回原样）",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "drag_from_title_text",
        what: "按住标题文字本身拖动，窗口位置真的改变（data-tauri-drag-region=\"deep\"）",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "taskbar_icon_correct",
        what: "窗口带 _NET_WM_ICON，任务栏能取到图标",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "tray_icon_correct",
        what: "托盘图标存在且背景透明",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "ime_prefilled_search_box",
        what: "中文输入法往一个**已有内容**的检索框里输入：不冻结且字符落入框内（tauri#15436）",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "ime_prefilled_search_box_no_freeze",
        what: "承 tauri#15436：聚焦已有内容的检索框并输入后，界面仍然响应（OS 层可证的那一半）",
        channel: Channel::OsHarness,
        needs_screenshot: true,
    },
    Declared {
        id: "two_char_search_returns_results",
        what: "两字检索（明月）返回结果行",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "corpus_first_run_materialization",
        what: "首启语料物化完成并显示进度",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "shipped_appreciation_without_key",
        what: "没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "voice_round_succeeds_end_to_end",
        what: "语音背诵一轮端到端成功：采集 -> ASR -> 无偏置评分这条链跑通",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "voice_degradation_states_reason",
        what: "失败路径单独验证：语音不可用时切到打字模式并显示具体原因",
        channel: Channel::WebDriver,
        needs_screenshot: true,
    },
    Declared {
        id: "app_exits_cleanly",
        what: "应用正常退出，退出码为 0，不留孤儿进程",
        channel: Channel::Process,
        needs_screenshot: false,
    },
];

/// 一条断言的执行结果。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Outcome {
    id: &'static str,
    what: &'static str,
    channel: Channel,
    /// 这条是否要求截图。写进报告，好让读的人一眼看出「一条要求截图的 PASS 却没有图」
    /// 是可疑的——那正是 `screenshot_present_for_every_passing_ui_assertion` 守的事。
    screenshot_required: bool,
    verdict: Verdict,
    /// PASS 时是观测到的事实；FAIL 时是不符之处；NOT EXECUTED 时是阻塞原因。
    /// **永远不为空**——空 verdict 说明与解析器断言冲突。
    detail: String,
    /// NOT EXECUTED 时：什么条件下能跑。其余取值为 `None`。
    executable_when: Option<String>,
    /// 截图相对报告目录的路径。
    screenshot: Option<String>,
}

/// 报告的机器可读形态。终验消费 `all_pass`。
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    schema_version: u32,
    platform: String,
    set: String,
    /// 生成日期（本地时区，`YYYY-MM-DD`）。
    date: String,
    /// 被测应用版本，取自 `tauri.conf.json`。
    app_version: String,
    /// 被测提交。
    commit_sha: String,
    /// 操作系统构建标识。
    os_build: String,
    /// 会话形态。虚拟显示不等于物理显示器，报告必须说清。
    session: SessionFacts,
    /// **严格语义：零 FAIL 且零 NOT EXECUTED。** 见模块文档。
    all_pass: bool,
    /// 实际执行且通过的条数。
    executed_pass: usize,
    failed: usize,
    not_executed: usize,
    /// 本次未执行的平台，附原因。顶部显著标注用。
    platforms_not_executed: Vec<PlatformGap>,
    assertions: Vec<Outcome>,
}

/// 会话事实。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SessionFacts {
    /// `physical` | `virtual` | `none`。
    pub(crate) display_kind: String,
    pub(crate) display: Option<String>,
    /// 窗口管理器名字。没有 WM 就没有 `_NET_WM_STATE`，最大化断言无从判定。
    pub(crate) window_manager: Option<String>,
    /// WebDriver 握手探测结果。
    pub(crate) webdriver: WebDriverProbe,
    /// 音频输入设备数。0 意味着语音端到端那条必然 NOT EXECUTED。
    pub(crate) audio_input_devices: usize,
}

/// WebDriver 握手探测。**探测结果本身就是证据**：它是「UI 断言为何未执行」这句话的依据。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct WebDriverProbe {
    pub(crate) attempted: bool,
    pub(crate) succeeded: bool,
    pub(crate) detail: String,
}

/// 未执行的平台。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PlatformGap {
    platform: String,
    reason: String,
    executable_when: String,
}

/// 结果收集器。**只接受已声明的 id**，并保证每条声明恰好被填一次。
pub(crate) struct Collector {
    outcomes: Vec<Outcome>,
}

impl Collector {
    fn new() -> Self {
        Self {
            outcomes: Vec::with_capacity(DECLARED.len()),
        }
    }

    /// 记录一条 verdict。
    ///
    /// # Errors
    ///
    /// id 不在 [`DECLARED`] 里，或同一 id 被填了两次，或 `detail` 为空。三者都是
    /// harness 自身的缺陷，不是被测产品的缺陷，所以是错误而不是一条 FAIL。
    pub(crate) fn record(
        &mut self,
        id: &str,
        verdict: Verdict,
        detail: impl Into<String>,
        executable_when: Option<String>,
        screenshot: Option<String>,
    ) -> Result<()> {
        let declared = DECLARED
            .iter()
            .find(|d| d.id == id)
            .with_context(|| format!("断言 `{id}` 未在 DECLARED 里声明；不允许运行期新增断言"))?;
        if self.outcomes.iter().any(|o| o.id == declared.id) {
            bail!("断言 `{id}` 被记录了两次");
        }
        let detail = detail.into();
        if detail.trim().is_empty() {
            bail!("断言 `{id}` 的 detail 为空；每条 verdict 都必须说明依据");
        }
        if verdict == Verdict::NotExecuted && executable_when.is_none() {
            bail!("断言 `{id}` 标为 NOT EXECUTED 但没写「什么条件下能跑」");
        }
        emit(&format!(
            "  {:<12} {}  {}",
            verdict.as_str(),
            declared.id,
            detail
        ));
        self.outcomes.push(Outcome {
            id: declared.id,
            what: declared.what,
            channel: declared.channel,
            screenshot_required: declared.needs_screenshot,
            verdict,
            detail,
            executable_when,
            screenshot,
        });
        Ok(())
    }

    /// 把还没填过的声明一律补成 NOT EXECUTED。
    ///
    /// 这是「提前退出不会被误当成成功」的机制：无论 harness 在哪一步崩，剩下的条目都会
    /// 以未执行的形态出现在报告里。
    pub(crate) fn fill_remaining(&mut self, reason: &str, executable_when: &str) -> Result<()> {
        let missing: Vec<&'static str> = DECLARED
            .iter()
            .filter(|d| !self.outcomes.iter().any(|o| o.id == d.id))
            .map(|d| d.id)
            .collect();
        for id in missing {
            self.record(
                id,
                Verdict::NotExecuted,
                reason,
                Some(executable_when.to_owned()),
                None,
            )?;
        }
        Ok(())
    }

    /// 按 [`DECLARED`] 的顺序输出，便于人读时逐条对照。
    fn finish(self) -> Vec<Outcome> {
        let mut sorted = Vec::with_capacity(self.outcomes.len());
        for declared in DECLARED {
            if let Some(found) = self.outcomes.iter().find(|o| o.id == declared.id) {
                sorted.push(found.clone());
            }
        }
        sorted
    }
}

/// 目标平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    /// 解析 `--platform`。
    ///
    /// # Errors
    ///
    /// 取值不在 `win` | `mac` | `linux` 之内。
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        match raw {
            "win" | "windows" => Ok(Self::Windows),
            "mac" | "macos" => Ok(Self::MacOs),
            "linux" => Ok(Self::Linux),
            other => bail!("未知平台 `{other}`；只接受 win | mac | linux"),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
        }
    }
}

/// 子命令入口。
///
/// # Errors
///
/// 出现任何 `FAIL`，或 harness 自身无法写出报告时返回错误。`NOT EXECUTED` 不算错误，
/// 见模块文档「退出码与 `all_pass` 刻意不同义」。
pub(crate) fn run(platform: &str, set: &str) -> Result<()> {
    let platform = Platform::parse(platform)?;
    if set != "desktop" {
        bail!("未知断言集 `{set}`；本子命令目前只有 desktop");
    }
    let root = repo_root();

    emit("== 桌面端真机验收（断言集在执行之前声明）==");
    emit(&format!(
        "  平台 {}  断言集 {set}  声明 {} 条",
        platform.as_str(),
        DECLARED.len()
    ));

    let mut collector = Collector::new();
    let session = match platform {
        Platform::Linux => linux::execute(&root, &mut collector)?,
        Platform::Windows | Platform::MacOs => {
            let (reason, when) = foreign_platform_gap(platform);
            collector.fill_remaining(reason, when)?;
            SessionFacts {
                display_kind: "none".to_owned(),
                display: None,
                window_manager: None,
                webdriver: WebDriverProbe {
                    attempted: false,
                    succeeded: false,
                    detail: reason.to_owned(),
                },
                audio_input_devices: 0,
            }
        }
    };

    let report = build_report(&root, platform, set, session, collector)?;
    let paths = write_report(&root, &report)?;

    emit(&format!(
        "  报告 {}  与 {}",
        paths.markdown.display(),
        paths.json.display()
    ));
    emit(&format!(
        "  汇总 PASS {}  FAIL {}  NOT EXECUTED {}  all_pass {}",
        report.executed_pass, report.failed, report.not_executed, report.all_pass
    ));

    if report.failed > 0 {
        bail!(
            "桌面端验收有 {} 条 FAIL，详见 {}",
            report.failed,
            paths.markdown.display()
        );
    }
    Ok(())
}

/// 非本机平台的统一缺口说明。
const fn foreign_platform_gap(platform: Platform) -> (&'static str, &'static str) {
    match platform {
        Platform::Windows => (
            "本机是 Linux 容器，没有 Windows 宿主机，也没有 msedgedriver；\
             session 0 的服务账户跑不出交互式桌面会话",
            "一台装有 WebView2 运行时的 Windows 11 交互式登录会话，\
             加上与 Edge 主版本号匹配的 msedgedriver",
        ),
        Platform::MacOs => (
            "本机没有 macOS 宿主机；且 WKWebView 没有官方 WebDriver 工具，\
             公证构建还需要签名身份",
            "一台交互式登录的 macOS 机器、一个 Developer ID 签名身份，\
             以及 @wdio/tauri-service 的 embedded WebDriver（macOS 唯一可行的驱动方式）",
        ),
        Platform::Linux => ("", ""),
    }
}

/// 报告文件路径对。
pub(crate) struct ReportPaths {
    markdown: PathBuf,
    json: PathBuf,
}

fn build_report(
    root: &Path,
    platform: Platform,
    set: &str,
    session: SessionFacts,
    collector: Collector,
) -> Result<Report> {
    let assertions = collector.finish();
    if assertions.len() != DECLARED.len() {
        bail!(
            "报告有 {} 条断言，声明了 {} 条；两者必须一致",
            assertions.len(),
            DECLARED.len()
        );
    }
    // 「每条 UI 断言都有截图」这句话只对**执行过**的那些成立：未执行的条目没有可截的
    // 界面。所以判据落在 PASS 上——一条声称通过却拿不出图的 UI 断言是不可信的。
    for outcome in &assertions {
        if outcome.verdict == Verdict::Pass
            && outcome.screenshot_required
            && outcome.screenshot.is_none()
        {
            bail!(
                "断言 `{}` 判为 PASS 且要求截图，但没有截图；界面上确实是这样这句话只有图能作证",
                outcome.id
            );
        }
    }
    let failed = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::Fail)
        .count();
    let not_executed = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::NotExecuted)
        .count();
    let executed_pass = assertions
        .iter()
        .filter(|o| o.verdict == Verdict::Pass)
        .count();

    let mut gaps = Vec::new();
    for other in [Platform::Windows, Platform::MacOs, Platform::Linux] {
        if other == platform {
            continue;
        }
        let (reason, when) = foreign_platform_gap(other);
        if reason.is_empty() {
            gaps.push(PlatformGap {
                platform: other.as_str().to_owned(),
                reason: "本次运行只针对 --platform 指定的那一个平台".to_owned(),
                executable_when: "在该平台的交互式桌面会话里另跑一次本子命令".to_owned(),
            });
        } else {
            gaps.push(PlatformGap {
                platform: other.as_str().to_owned(),
                reason: reason.to_owned(),
                executable_when: when.to_owned(),
            });
        }
    }

    Ok(Report {
        schema_version: 1,
        platform: platform.as_str().to_owned(),
        set: set.to_owned(),
        date: today(),
        app_version: read_app_version(root)?,
        commit_sha: commit_sha(root),
        os_build: os_build(),
        session,
        all_pass: failed == 0 && not_executed == 0,
        executed_pass,
        failed,
        not_executed,
        platforms_not_executed: gaps,
        assertions,
    })
}

fn write_report(root: &Path, report: &Report) -> Result<ReportPaths> {
    let dir = root.join(REPORT_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    let stem = format!("desktop-qa-{}", report.date);
    let json = dir.join(format!("{stem}.json"));
    let markdown = dir.join(format!("{stem}.md"));

    let mut encoded = serde_json::to_string_pretty(report).context("序列化验收报告失败")?;
    encoded.push('\n');
    fs::write(&json, encoded).with_context(|| format!("写 {} 失败", json.display()))?;
    fs::write(&markdown, render_markdown(report))
        .with_context(|| format!("写 {} 失败", markdown.display()))?;
    Ok(ReportPaths { markdown, json })
}

fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 桌面端真机验收 · {}", report.date);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> [!WARNING]\n\
         > **`all_pass` = `{}`。本次只在 `{}` 上执行。**\n\
         > 本文件是**一次运行**的报告，`platform` 字段就是那一次的平台。文件名只带日期，\n\
         > 所以同一天换个平台再跑会覆盖它——要保留多平台结果，跑完一个平台先把报告归档。\n\
         > `all_pass` 的语义是**零 FAIL 且零 NOT EXECUTED**，因此只要有任何一条未执行\n\
         > 它就是 `false`。它**不能**被读成「三平台都过了」。逐平台执行状态见下方\n\
         > 「未执行的平台」一节。",
        report.all_pass, report.platform
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## 被测对象");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 项 | 值 |");
    let _ = writeln!(out, "| --- | --- |");
    let _ = writeln!(out, "| 平台 | `{}` |", report.platform);
    let _ = writeln!(out, "| 断言集 | `{}` |", report.set);
    let _ = writeln!(out, "| 应用版本 | `{}` |", report.app_version);
    let _ = writeln!(out, "| 提交 | `{}` |", report.commit_sha);
    let _ = writeln!(out, "| 操作系统构建 | `{}` |", report.os_build);
    let _ = writeln!(
        out,
        "| 会话 | `{}`{} |",
        report.session.display_kind,
        report
            .session
            .display
            .as_deref()
            .map_or_else(String::new, |d| format!("（`DISPLAY={d}`）"))
    );
    let _ = writeln!(
        out,
        "| 窗口管理器 | {} |",
        report
            .session
            .window_manager
            .as_deref()
            .map_or_else(|| "无".to_owned(), |w| format!("`{w}`"))
    );
    let _ = writeln!(
        out,
        "| 音频输入设备 | {} |",
        report.session.audio_input_devices
    );
    let _ = writeln!(out);

    if report.session.display_kind == "virtual" {
        let _ = writeln!(
            out,
            "> [!NOTE]\n\
             > 会话是 **Xvfb 虚拟显示**加一个真实窗口管理器，不是物理显示器。窗口映射、\n\
             > `_NET_WM_STATE` 迁移、合成输入这些**窗口管理器层**的事实在这里成立；\n\
             > 而依赖真实显示硬件的事实（GPU 合成路径、HiDPI 缩放、屏幕色彩）不成立，\n\
             > 本报告不对它们下结论。"
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## 汇总");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "声明 {} 条 · PASS {} · FAIL {} · NOT EXECUTED {}",
        report.assertions.len(),
        report.executed_pass,
        report.failed,
        report.not_executed
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## WebDriver 握手探测");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "尝试：{} · 成功：{}",
        yes_no(report.session.webdriver.attempted),
        yes_no(report.session.webdriver.succeeded)
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", report.session.webdriver.detail);
    let _ = writeln!(out);

    let _ = writeln!(out, "## 逐条断言");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 断言 | 通道 | 裁决 | 依据 | 截图 |");
    let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
    for outcome in &report.assertions {
        let _ = writeln!(
            out,
            "| `{}`<br>{} | {} | **{}** | {}{} | {} |",
            outcome.id,
            outcome.what,
            channel_label(outcome.channel),
            outcome.verdict.as_str(),
            outcome.detail,
            outcome
                .executable_when
                .as_deref()
                .map_or_else(String::new, |w| format!("<br>**可执行条件**：{w}")),
            outcome
                .screenshot
                .as_deref()
                .map_or_else(|| "—".to_owned(), |s| format!("[`{s}`]({s})"))
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## 未执行的平台");
    let _ = writeln!(out);
    let _ = writeln!(out, "| 平台 | 原因 | 可执行条件 |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for gap in &report.platforms_not_executed {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            gap.platform, gap.reason, gap.executable_when
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "## `all_pass` 的语义，以及退出码为什么与它不同义\n\
         \n\
         `all_pass` 取**最严格**的定义：零 FAIL 且零 NOT EXECUTED。终验会消费这个字段，\n\
         而它最容易造成的误读是「三平台都过了」——只要 Windows 或 macOS 未执行，那句话\n\
         就是假的。因此宁可让它在任何缺口下都为 `false`，另用 `executed_pass` / `failed` /\n\
         `not_executed` 三个计数表达细节。\n\
         \n\
         退出码只看 `FAIL`。`NOT EXECUTED` 是 harness 尽了责的结果——它如实说明了自己\n\
         做不到什么，那不是缺陷。把未执行也判成非零会奖励一个很坏的动作：**删掉**跑不了\n\
         的断言好让命令变绿。"
    );

    out
}

const fn yes_no(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

const fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::WebDriver => "WebDriver",
        Channel::OsHarness => "OS 输入",
        Channel::Process => "进程",
    }
}

/// 仓库根。`CARGO_MANIFEST_DIR` 是 `xtask/`，向上一级即根。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须有父目录")
        .to_path_buf()
}

/// 应用版本取自 `tauri.conf.json`，不从 `CARGO_PKG_VERSION` 猜。
fn read_app_version(root: &Path) -> Result<String> {
    let path = root.join(TAURI_CONF);
    let text = fs::read_to_string(&path).with_context(|| format!("读 {} 失败", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{} 里没有 version 字段", path.display()))
}

fn commit_sha(root: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn os_build() -> String {
    let kernel = Command::new("uname")
        .arg("-sr")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|| std::env::consts::OS.to_owned());
    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME=").map(str::to_owned))
        })
        .map(|value| value.trim_matches('"').to_owned());
    match distro {
        Some(distro) => format!("{distro} / {kernel}"),
        None => kernel,
    }
}

/// 本地日期 `YYYY-MM-DD`。刻意用 `date` 而不是引一个时间库：报告只需要一个日期，
/// 而 xtask 的依赖树里目前没有 chrono / time，为一行格式化拖进来不值得。
fn today() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

pub(crate) mod linux;
pub(crate) mod screenshot;
pub(crate) mod webdriver;

#[cfg(test)]
mod tests;

/// 供子模块与测试构造子进程的小工具：起一个后台进程并持有它。
struct Background {
    name: &'static str,
    child: Child,
}

impl Background {
    fn spawn(name: &'static str, command: &mut Command) -> Result<Self> {
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("启动 {name} 失败"))?;
        Ok(Self { name, child })
    }

    /// 轮询到 `predicate` 为真或超时。
    fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        predicate()
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        // 拆除必须尽力而为且不 panic：Drop 里 panic 会把真正的失败原因盖掉。
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.name;
    }
}
