//! Linux 上的执行体：拉起交互式会话，驱动真实窗口，逐条判定。
//!
//! # 会话怎么来
//!
//! 有 `DISPLAY` 就用现成的（那是一个物理桌面会话，报告标 `physical`）；没有就自己起
//! 一个 **Xvfb + 真实窗口管理器**（报告标 `virtual`）。虚拟显示不等于物理显示器，两者
//! 能证明的事情不同，因此报告里必须区分——把 Xvfb 说成真机是这份报告最容易造的假。
//!
//! 窗口管理器是必需的，不是可选装饰：`_NET_WM_STATE_MAXIMIZED_*` 由窗口管理器维护，
//! 没有 WM 就没有这些属性，于是「双击真的最大化了」这条断言无从判定——而它会看起来
//! 像是「应用没有最大化」，即一个假的 FAIL。
//!
//! # 为什么必须自带一条 session D-Bus
//!
//! 容器里 `DBUS_SESSION_BUS_ADDRESS` 常常指着一个已经死掉的 socket。应用侧因此连不上
//! 总线（本机实测日志：`Failed to connect to address unix:path=/tmp/dbus-…`），
//! 而 WebKitGTK 的自动化会话要经总线通告自己。`dbus-run-session` 起一条新的。

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use enigo::{Button, Coordinate, Direction, Enigo, Keyboard as _, Mouse as _, Settings};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::webdriver::{self, Session};
use super::{
    APP_BINARY, Background, Collector, SessionFacts, Verdict, WebDriverProbe, geometry, screenshot,
};
use crate::verify_sources::emit;

/// 自建虚拟显示用的编号。取一个不太可能被占的号。
const VIRTUAL_DISPLAY: &str = ":99";
/// 虚拟屏幕尺寸。要比窗口的 1200×800 大，否则「最大化」与「原状」量不出差别。
const VIRTUAL_SCREEN: &str = "1600x1100x24";
/// 截图子目录，相对 `docs/reports/`。
const SHOT_DIR: &str = "desktop-qa";
/// 等窗口出现的上限。debug 构建首帧要几秒。
const WINDOW_TIMEOUT: Duration = Duration::from_secs(30);
/// 等窗口状态迁移的上限。窗口管理器处理一次请求是毫秒级，给足余量。
///
/// **不是这个值的问题**：本轮把它加到 20 秒，`control_maximize_works` 与
/// `control_restore_works` 的判词完全不变，因此那两条不是等得不够久。
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// 等 WebView 画出第一帧的上限。debug 构建要加载并执行整个前端包。
const PAINT_TIMEOUT: Duration = Duration::from_secs(20);
/// 等 React 把首屏挂到 DOM 上的上限。实测约 2 秒，取 20 秒余量。
const MOUNT_TIMEOUT: Duration = Duration::from_secs(20);

/// 执行 Linux 断言集。
///
/// # Errors
///
/// harness 自身无法工作时（连不上 X、记录了未声明的断言）返回错误。被测产品的问题一律
/// 记成 `FAIL`，不是错误。
pub(crate) fn execute(root: &Path, collector: &mut Collector) -> Result<SessionFacts> {
    let app = root.join(APP_BINARY);
    let shots = root.join(super::REPORT_DIR).join(SHOT_DIR);
    std::fs::create_dir_all(&shots).with_context(|| format!("创建 {} 失败", shots.display()))?;

    // --- 产物：先判定，因为后面每一条都依赖它 ---
    let artifact_ok = check_artifact(root, &app, collector)?;
    check_installer(root, collector)?;

    if !artifact_ok {
        collector.fill_remaining(
            "构建产物不可用，后续断言无从执行",
            "先跑 `cargo build -p yunjian-app`（Linux 上还需 libwebkit2gtk-4.1-dev）",
        )?;
        return Ok(SessionFacts {
            display_kind: "none".to_owned(),
            display: None,
            window_manager: None,
            webdriver: WebDriverProbe {
                attempted: false,
                succeeded: false,
                detail: "产物不可用，未尝试握手。".to_owned(),
            },
            audio_input_devices: 0,
        });
    }

    // --- 会话 ---
    let session = Session0::acquire()?;
    emit(&format!(
        "  会话 {} DISPLAY={}",
        session.kind, session.display
    ));

    let conn = session.connect_x11()?;
    let root_window = conn.setup().roots[session.screen].root;
    let wm = window_manager_name(&conn, root_window);
    let audio = audio_input_devices();

    // --- WebDriver 握手探测。结果本身就是「UI 断言为何未执行」的依据 ---
    let env = vec![
        ("DISPLAY", session.display.clone()),
        ("TAURI_WEBVIEW_AUTOMATION", "true".to_owned()),
        ("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK", "1".to_owned()),
    ];
    emit("  探测 WebDriver 握手（真实 tauri-driver + WebKitWebDriver，无 mock）");
    let probe = webdriver::connect(&app, &env)?;
    let (driver, probe_facts) = match probe {
        Ok(session) => (
            Some(session),
            WebDriverProbe {
                attempted: true,
                succeeded: true,
                detail: "会话建立成功，DOM 层断言由真实 WebDriver 执行。".to_owned(),
            },
        ),
        Err(failure) => (
            None,
            WebDriverProbe {
                attempted: true,
                succeeded: false,
                detail: failure.detail,
            },
        ),
    };

    match driver {
        Some(driver) => webdriver_assertions(&driver, &shots, collector)?,
        None => {
            let when = "`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；\
                 也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service`";
            for declared in super::DECLARED {
                if declared.channel == super::Channel::WebDriver {
                    let (detail, executable_when) = if declared.id
                        == "voice_round_succeeds_end_to_end"
                        && audio == 0
                    {
                        (
                            format!(
                                "WebDriver 会话未能建立，且本次探测到的音频输入设备数为 {audio}；\
                                 DOM 与真实采集链均无法执行。握手探测结果：{}",
                                probe_facts.detail
                            ),
                            format!(
                                "{when}；另需至少一个非 monitor 的可采集音频输入设备及可用语音模型"
                            ),
                        )
                    } else {
                        (
                            format!(
                                "WebDriver 会话未能建立，DOM 层事实无法观测；\
                                 刻意不用 mock 或 stub 顶替。握手探测结果：{}",
                                probe_facts.detail
                            ),
                            when.to_owned(),
                        )
                    };
                    collector.record(
                        declared.id,
                        Verdict::NotExecuted,
                        detail,
                        Some(executable_when),
                        None,
                    )?;
                }
            }
        }
    }

    // --- OS 层断言：真实窗口 + 合成输入 ---
    os_assertions(&conn, root_window, &session, &app, &shots, collector)?;

    // 兜底：任何没走到的条目都以未执行入账，而不是从报告里消失。
    collector.fill_remaining(
        "harness 在到达这条之前结束",
        "重跑本子命令并检查上面失败的那一步",
    )?;

    Ok(SessionFacts {
        display_kind: session.kind.to_owned(),
        display: Some(session.display.clone()),
        window_manager: wm,
        webdriver: probe_facts,
        audio_input_devices: audio,
    })
}

fn check_artifact(root: &Path, app: &Path, collector: &mut Collector) -> Result<bool> {
    if !app.is_file() {
        collector.record(
            "artifact_present",
            Verdict::Fail,
            format!("{} 不存在", app.display()),
            None,
            None,
        )?;
        return Ok(false);
    }
    let binary_modified = app
        .metadata()
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("读取 {} 修改时间失败", app.display()))?;
    let (newest_input, newest_path) = newest_desktop_input(root)?;
    if !artifact_is_current(binary_modified, newest_input) {
        collector.record(
            "artifact_present",
            Verdict::Fail,
            format!(
                "{} 早于桌面源码输入 {}，拒绝用陈旧二进制裁决当前源码",
                app.display(),
                newest_path.display()
            ),
            None,
            None,
        )?;
        return Ok(false);
    }
    let missing = Command::new("ldd")
        .arg(app)
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|line| line.contains("not found"))
                .map(|line| line.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if missing.is_empty() {
        collector.record(
            "artifact_present",
            Verdict::Pass,
            format!("{} 存在，ldd 未报缺失动态库", app.display()),
            None,
            None,
        )?;
        Ok(true)
    } else {
        collector.record(
            "artifact_present",
            Verdict::Fail,
            format!("动态库缺失：{}", missing.join("；")),
            None,
            None,
        )?;
        Ok(false)
    }
}

fn artifact_is_current(binary_modified: SystemTime, newest_input: SystemTime) -> bool {
    binary_modified >= newest_input
}

fn newest_desktop_input(root: &Path) -> Result<(SystemTime, std::path::PathBuf)> {
    let mut newest = (SystemTime::UNIX_EPOCH, root.to_path_buf());
    for relative in ["Cargo.toml", "Cargo.lock", "app", "crates"] {
        newest_modified(&root.join(relative), &mut newest)?;
    }
    Ok(newest)
}

fn newest_modified(path: &Path, newest: &mut (SystemTime, std::path::PathBuf)) -> Result<()> {
    let metadata = path
        .metadata()
        .with_context(|| format!("读取桌面构建输入 {} 失败", path.display()))?;
    if metadata.is_dir() {
        if path
            .file_name()
            .is_some_and(|name| name == "node_modules" || name == "__tests__" || name == "tests")
        {
            return Ok(());
        }
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("遍历桌面构建输入 {} 失败", path.display()))?
        {
            newest_modified(&entry?.path(), newest)?;
        }
    } else if !path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.contains(".test.") || name.contains(".spec.")
    }) {
        let modified = metadata
            .modified()
            .with_context(|| format!("读取桌面构建输入 {} 修改时间失败", path.display()))?;
        if modified > newest.0 {
            *newest = (modified, path.to_path_buf());
        }
    }
    Ok(())
}

fn check_installer(root: &Path, collector: &mut Collector) -> Result<()> {
    let bundle = root.join("target/debug/bundle");
    let found: Vec<String> = std::fs::read_dir(&bundle)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if found.is_empty() {
        collector.record(
            "installer_runs",
            Verdict::NotExecuted,
            format!(
                "{} 下没有安装包产物；本次验收跑的是未打包的构建产物，\
                 因此「安装后从安装路径启动」这条链没有被执行",
                bundle.display()
            ),
            Some(
                "先 `cargo tauri build`（Linux 上还需 `dpkg-deb` 与 `librsvg2-dev`），\
                 再由 harness 安装到一个临时 root 并从该路径启动。\
                 NSIS 静默安装时**只等安装器进程本身**，不要等整棵进程树——\
                 完成页会把应用作为子进程拉起来，等进程树会永远等不完。"
                    .to_owned(),
            ),
            None,
        )?;
    } else {
        collector.record(
            "installer_runs",
            Verdict::NotExecuted,
            format!(
                "发现产物 {}，但本 harness 尚未实现安装后启动",
                found.join("、")
            ),
            Some("实现安装到临时 root 并从该路径启动的步骤".to_owned()),
            None,
        )?;
    }
    Ok(())
}

/// 一个交互式桌面会话。
struct Session0 {
    display: String,
    kind: &'static str,
    screen: usize,
    /// 持有自建的 Xvfb 与窗口管理器；用现成会话时为空。
    _owned: Vec<Background>,
}

impl Session0 {
    fn acquire() -> Result<Self> {
        if let Ok(display) = std::env::var("DISPLAY")
            && !display.is_empty()
        {
            return Ok(Self {
                display,
                kind: "physical",
                screen: 0,
                _owned: Vec::new(),
            });
        }

        // 自建：Xvfb + 窗口管理器。
        //
        // **不带 `-auth`**。带上它就要把 cookie 交给每一个客户端，而 `enigo` 的 x11rb
        // 后端只从进程环境读 `XAUTHORITY`——往环境里写要 `std::env::set_var`，它在 2024
        // 版次是 `unsafe`，本工作区禁用。于是合成输入会连不上自己刚建的显示。
        //
        // 取舍是清楚的：这块显示 `-nolisten tcp`，只存在于一个本地 socket 上，
        // 生命周期就是本次验收，且它是一块 QA 显示而不是任何人的登录会话。
        //
        // 另记一个真实的排障陷阱：残留的 X server 会让新起的 Xvfb 以
        // `server already running` 死掉，而**客户端**那侧看到的是
        // `Authorization required, but no authorization protocol specified`——
        // 一条把人引向授权问题的错误信息，真因却是端口已被占用。
        let stale = Path::new("/tmp/.X11-unix")
            .join(format!("X{}", VIRTUAL_DISPLAY.trim_start_matches(':')));
        if stale.exists() {
            anyhow::bail!(
                "{} 已存在，说明 {VIRTUAL_DISPLAY} 上已有一个 X server；\
                 先把它停掉再跑（残留 server 会让本次 Xvfb 以 `server already running` 退出，\
                 而客户端看到的却是一条误导性的授权错误）",
                stale.display()
            );
        }

        let mut xvfb = Command::new("Xvfb");
        xvfb.args([
            VIRTUAL_DISPLAY,
            "-screen",
            "0",
            VIRTUAL_SCREEN,
            "-nolisten",
            "tcp",
        ]);
        let xvfb = Background::spawn("Xvfb", &mut xvfb)?;

        let ready = Background::wait_until(Duration::from_secs(15), || {
            Command::new("xdpyinfo")
                .env("DISPLAY", VIRTUAL_DISPLAY)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        });
        if !ready {
            anyhow::bail!("Xvfb 起来后 15 秒内 {VIRTUAL_DISPLAY} 仍不可连接");
        }

        // 窗口管理器。没有它就没有 `_NET_WM_STATE`，最大化断言会得出一个假的 FAIL。
        let mut wm = Command::new("openbox");
        wm.env("DISPLAY", VIRTUAL_DISPLAY);
        let wm = Background::spawn("openbox", &mut wm)?;
        // 等到窗口管理器真的接管：`_NET_SUPPORTING_WM_CHECK` 出现才算。
        // 抢在它之前拉起应用，窗口会以无 WM 的形态映射，之后所有状态断言都失真。
        Background::wait_until(Duration::from_secs(10), || {
            Command::new("xprop")
                .env("DISPLAY", VIRTUAL_DISPLAY)
                .args(["-root", "_NET_SUPPORTING_WM_CHECK"])
                .output()
                .is_ok_and(|out| {
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).contains("window id")
                })
        });

        Ok(Self {
            display: VIRTUAL_DISPLAY.to_owned(),
            kind: "virtual",
            screen: 0,
            _owned: vec![xvfb, wm],
        })
    }

    /// 连上这个会话的 X 服务器。
    ///
    /// 显示名显式传入而**不依赖进程环境**：自建会话的 `DISPLAY` 只存在于本结构体里，
    /// 往环境里写要 `std::env::set_var`，它在 2024 版次是 `unsafe`，本工作区禁用。
    ///
    /// # Errors
    ///
    /// 连不上或屏幕号不存在。
    fn connect_x11(&self) -> Result<RustConnection> {
        let (conn, screen) = x11rb::connect(Some(&self.display))
            .with_context(|| format!("连接 X 显示 {} 失败", self.display))?;
        debug_assert_eq!(screen, self.screen, "本 harness 只用 0 号屏幕");
        Ok(conn)
    }
}

fn window_manager_name(conn: &impl Connection, root: Window) -> Option<String> {
    let check = intern(conn, b"_NET_SUPPORTING_WM_CHECK")?;
    let owner = conn
        .get_property(false, root, check, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()?;
    let name_atom = intern(conn, b"_NET_WM_NAME")?;
    let utf8 = intern(conn, b"UTF8_STRING")?;
    let reply = conn
        .get_property(false, owner, name_atom, utf8, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn intern(conn: &impl Connection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(true, name)
        .ok()?
        .reply()
        .ok()
        .and_then(|r| (r.atom != 0).then_some(r.atom))
}

/// PulseAudio 的输入源计数。0 意味着语音端到端那条必然未执行。
fn audio_input_devices() -> usize {
    let Some(pactl) = webdriver::which("pactl") else {
        return 0;
    };
    Command::new(pactl)
        .args(["list", "short", "sources"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                // `.monitor` 是输出的回环监听，不是麦克风。把它算成输入设备就会
                // 得出「有麦克风」这个假结论，而随后的采集必然拿到静音。
                .filter(|line| !line.contains(".monitor"))
                .count()
        })
        .unwrap_or(0)
}

/// DOM 层断言。只在真实会话建立后调用。
fn webdriver_assertions(driver: &Session, shots: &Path, collector: &mut Collector) -> Result<()> {
    // 两字检索。`search-input` / `search-submit` / `result-row` 是 app/src/search 里
    // 真实存在的 data-testid，不是猜的。
    let two_char = (|| -> Result<String> {
        if !driver.wait_for("[data-testid='search-input']", MOUNT_TIMEOUT) {
            bail!(
                "等了 {} 秒，`[data-testid='search-input']` 仍未出现在 DOM 里",
                MOUNT_TIMEOUT.as_secs()
            );
        }
        driver.send_keys("[data-testid='search-input']", "明月")?;
        driver.click("[data-testid='search-submit']")?;
        std::thread::sleep(Duration::from_secs(2));
        driver.text("[data-testid='search-summary']")
    })();
    let shot = shots.join("two-char-search.png");
    match two_char {
        Ok(summary) if !summary.trim().is_empty() => collector.record(
            "two_char_search_returns_results",
            Verdict::Pass,
            format!("检索「明月」后摘要为「{summary}」"),
            None,
            Some(relative_shot(&shot)),
        )?,
        Ok(_) => collector.record(
            "two_char_search_returns_results",
            Verdict::Fail,
            "检索「明月」后 search-summary 为空",
            None,
            Some(relative_shot(&shot)),
        )?,
        Err(error) => collector.record(
            "two_char_search_returns_results",
            Verdict::Fail,
            format!("驱动检索失败：{error}"),
            None,
            None,
        )?,
    }

    // 其余 DOM 断言尚未实现驱动步骤。**如实标未执行**，而不是让它们从报告里消失。
    for id in [
        "ime_prefilled_search_box",
        "corpus_first_run_materialization",
        "shipped_appreciation_without_key",
        "voice_round_succeeds_end_to_end",
        "voice_degradation_states_reason",
    ] {
        collector.record(
            id,
            Verdict::NotExecuted,
            "会话已建立，但本条的驱动步骤尚未实现",
            Some("实现该条的 WebDriver 驱动步骤".to_owned()),
            None,
        )?;
    }
    Ok(())
}

/// 相对报告目录的截图路径，写进报告用。
fn relative_shot(path: &Path) -> String {
    path.file_name()
        .map(|name| format!("{SHOT_DIR}/{}", name.to_string_lossy()))
        .unwrap_or_default()
}

/// OS 层断言：真实窗口 + 合成输入 + 属性观测。
fn os_assertions(
    conn: &RustConnection,
    root_window: Window,
    session: &Session0,
    app: &Path,
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let mut command = Command::new(app);
    command
        .env("DISPLAY", &session.display)
        .env("TAURI_WEBVIEW_AUTOMATION", "false")
        .env("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK", "1");
    let mut app_process = Background::spawn("yunjian-desktop", &mut command)?;

    let window = wait_for_window(conn, root_window);
    let Some(window) = window else {
        collector.record(
            "app_launches",
            Verdict::Fail,
            format!("启动后 {} 秒内没有映射出顶层窗口", WINDOW_TIMEOUT.as_secs()),
            None,
            None,
        )?;
        return Ok(());
    };

    let title = window_title(conn, window).unwrap_or_default();
    let shot = shots.join("app-launched.png");
    let _ = screenshot::capture(conn, window, &shot);
    collector.record(
        "app_launches",
        Verdict::Pass,
        format!("窗口 0x{window:x} 已映射，`_NET_WM_NAME` = 「{title}」"),
        None,
        Some(relative_shot(&shot)),
    )?;

    // --- 可观测门：后面每一条点击类断言都以「界面内容读得到」为前提 ---
    //
    // 顺序很重要：这一步必须在任何点击之前。点在一块纯色窗口上什么都不会发生，
    // 而把那记成 FAIL 是在报告一个假故障——按钮从来没被画出来，何谈「点了没反应」。
    let shot = shots.join("webview-paint.png");
    let mut paint = None;
    Background::wait_until(PAINT_TIMEOUT, || {
        paint = screenshot::capture_and_measure(conn, window, &shot).ok();
        paint.is_some_and(screenshot::Paint::painted)
    });
    let painted = paint.is_some_and(screenshot::Paint::painted);
    let measured = paint.map_or_else(
        || "像素读取失败".to_owned(),
        |p| {
            format!(
                "主色 #{:02x}{:02x}{:02x} 占 {:.4}%，显著颜色区 {} 个",
                p.dominant[0],
                p.dominant[1],
                p.dominant[2],
                p.dominant_share * 100.0,
                p.significant_colors
            )
        },
    );
    emit(&format!(
        "  绘制门 {measured}（主色占比 < 95% 才算画了东西）"
    ));

    if !painted {
        let reason = format!(
            "窗口内容不可观测：等了 {} 秒，{measured}。窗口本身是真的\
             （已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的\
             是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容\
             合成到了一块 X 读不到的 GL 表面（日志里有 \
             `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。\
             两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何\
             关于产品的事——记成 FAIL 会是一个假故障，故记未执行",
            PAINT_TIMEOUT.as_secs()
        );
        let when = "一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）\
             实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、\
             `LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。\
             一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可"
            .to_owned();
        for id in [
            "custom_titlebar_rendered",
            "control_minimize_works",
            "control_maximize_works",
            "control_restore_works",
            "control_close_works",
            "double_click_maximizes_exactly_once",
            "drag_from_title_text",
            "ime_prefilled_search_box_no_freeze",
        ] {
            collector.record(
                id,
                Verdict::NotExecuted,
                reason.clone(),
                Some(when.clone()),
                Some(relative_shot(&shot)),
            )?;
        }
        // 任务栏图标不依赖 WebView 绘制：它是窗口管理器读的一条属性，照判。
        taskbar_icon(conn, window, shots, collector)?;
        tray_icon(collector)?;
        collector.record(
            "app_exits_cleanly",
            Verdict::NotExecuted,
            "应用的正常退出入口是托盘菜单「退出」；本次 Xvfb + Openbox 会话没有托盘宿主，\
             harness 无法显示并点击该菜单，未用 kill 信号冒充正常退出",
            Some(
                "带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，\
                 并由 harness 观测托盘项后点击「退出」"
                    .to_owned(),
            ),
            None,
        )?;
        return Ok(());
    }

    // --- 自绘标题栏：窗口管理器没画边框 ---
    let extents = frame_extents(conn, window);
    let shot = shots.join("custom-titlebar.png");
    let _ = screenshot::capture(conn, window, &shot);
    match extents {
        Some(values) if values.iter().all(|v| *v == 0) => collector.record(
            "custom_titlebar_rendered",
            Verdict::Pass,
            format!("`_NET_FRAME_EXTENTS` = {values:?}，窗口管理器没有绘制边框；标题栏由应用自绘"),
            None,
            Some(relative_shot(&shot)),
        )?,
        None => collector.record(
            "custom_titlebar_rendered",
            Verdict::Pass,
            "窗口没有 `_NET_FRAME_EXTENTS`，即窗口管理器未加装饰框；标题栏由应用自绘",
            None,
            Some(relative_shot(&shot)),
        )?,
        Some(values) => collector.record(
            "custom_titlebar_rendered",
            Verdict::Fail,
            format!("`_NET_FRAME_EXTENTS` = {values:?}，窗口管理器仍在绘制边框，与 decorations: false 不符"),
            None,
            Some(relative_shot(&shot)),
        )?,
    }

    taskbar_icon(conn, window, shots, collector)?;
    tray_icon(collector)?;

    let mut enigo = Enigo::new(&Settings {
        x11_display: Some(session.display.clone()),
        ..Settings::default()
    })
    .context("初始化合成输入失败")?;

    // --- 双击标题栏恰好最大化一次 ---
    //
    // 这一条直接对着 todo 60 的规则：自己再挂一个双击处理器会让状态被切换两次，
    // 净效果回到原样。所以判据不是「最大化了」，而是「一次双击之后处于最大化」。
    let geom = window_geometry(conn, window);
    let drag_point = (
        geom.0 + geometry::title_text_x(),
        geom.1 + geometry::mid_y(),
    );
    let shot = shots.join("double-click-maximize.png");
    let before = is_maximized(conn, window);
    click_at(&mut enigo, drag_point, true)?;
    let became = Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window) != before);
    let after = is_maximized(conn, window);
    let _ = screenshot::capture(conn, root_window, &shot);
    if became && after {
        collector.record(
            "double_click_maximizes_exactly_once",
            Verdict::Pass,
            format!("双击前 maximized={before}，一次双击后 maximized={after}（恰好切换一次）"),
            None,
            Some(relative_shot(&shot)),
        )?;
    } else {
        collector.record(
            "double_click_maximizes_exactly_once",
            Verdict::Fail,
            format!(
                "双击前 maximized={before}，一次双击后 maximized={after}；\
                 净效果回到原样正是「自己又挂了一个双击处理器」的症状"
            ),
            None,
            Some(relative_shot(&shot)),
        )?;
    }
    // 还原，后面的按钮断言从非最大化态开始。
    //
    // 落点必须**重新**从当前几何算：最大化把窗口移到 (0,0) 并加宽，用最大化之前那份
    // 几何算出来的点已经落在内容区而不是标题栏上，于是这一下什么也不做、窗口留在最大化态。
    // 那个残留状态会让紧随其后的拖拽断言必然失败——最大化的窗口按定义拖不动——
    // 而报告里会写成「标题文字拖不动」，指着一处没坏的前端。
    if after {
        click_at(&mut enigo, title_point(conn, window), true)?;
        Background::wait_until(STATE_TIMEOUT, || !is_maximized(conn, window));
    }

    // --- 从标题文字拖拽 ---
    let shot = shots.join("drag-from-title-text.png");
    let still_maximized = is_maximized(conn, window);
    let origin = window_geometry(conn, window);
    let from = (
        origin.0 + geometry::title_text_x(),
        origin.1 + geometry::mid_y(),
    );
    drag(&mut enigo, from, (120, 90))?;
    let moved = Background::wait_until(STATE_TIMEOUT, || {
        let now = window_geometry(conn, window);
        now.0 != origin.0 || now.1 != origin.1
    });
    let after_drag = window_geometry(conn, window);
    let _ = screenshot::capture(conn, root_window, &shot);
    if still_maximized {
        collector.record(
            "drag_from_title_text",
            Verdict::NotExecuted,
            "拖拽开始前窗口仍处于最大化态，而最大化的窗口按定义不可拖动；\
             此时「位置没变」证不了标题栏拖动坏没坏，故不记 FAIL",
            Some(
                "一个能把窗口从最大化态还原下来的会话；还原本身由 \
                 `control_restore_works` 单独判定"
                    .to_owned(),
            ),
            Some(relative_shot(&shot)),
        )?;
    } else if moved {
        collector.record(
            "drag_from_title_text",
            Verdict::Pass,
            format!(
                "按住标题文字（窗口内 {},{}）拖动后，窗口从 {:?} 移到 {:?}",
                geometry::title_text_x(),
                geometry::mid_y(),
                (origin.0, origin.1),
                (after_drag.0, after_drag.1)
            ),
            None,
            Some(relative_shot(&shot)),
        )?;
    } else {
        collector.record(
            "drag_from_title_text",
            Verdict::Fail,
            format!(
                "按住标题文字（窗口内 {},{}）拖动后窗口位置未变（仍在 {:?}）；\
                 窗口当时不是最大化态，落点在自绘标题栏的文字上，\
                 因此这是标题栏拖动本身没有生效",
                geometry::title_text_x(),
                geometry::mid_y(),
                (origin.0, origin.1)
            ),
            None,
            Some(relative_shot(&shot)),
        )?;
    }

    // --- 中文输入：往一个已有内容的检索框里再输入 ---
    //
    // 这条只判 OS 层能证的那一半：界面在输入之后仍然响应。
    // 「字符确实落进了框里」需要读 DOM，属于 WebDriver 通道那条独立断言。
    let shot = shots.join("ime-prefilled-no-freeze.png");
    let responsive = chinese_input_keeps_ui_responsive(conn, window, &mut enigo)?;
    let _ = screenshot::capture(conn, root_window, &shot);
    if responsive {
        collector.record(
            "ime_prefilled_search_box_no_freeze",
            Verdict::Pass,
            "往检索框输入中文后再次聚焦并继续输入，窗口仍响应双击最大化，未冻结",
            None,
            Some(relative_shot(&shot)),
        )?;
    } else {
        collector.record(
            "ime_prefilled_search_box_no_freeze",
            Verdict::Fail,
            "聚焦已有内容的检索框并继续输入后，窗口不再响应窗口管理器请求（疑似冻结）",
            None,
            Some(relative_shot(&shot)),
        )?;
    }

    // --- 三个窗口控件 ---
    //
    // 每次点击**前**重新读几何，不复用一份快照。最大化会同时改变窗口的位置与宽度
    // （实测 1200 宽居中的窗口最大化成 1280 宽、原点移到 0,0），而按钮 x 是从窗口
    // 右缘往左数出来的。用最大化之前那份几何去算「还原」那一下，落点会偏 40 px ——
    // 恰好落进最小化按钮，于是「还原」把窗口最小化了、随后的「最小化」又点在按钮区
    // 之外。两条都变红，而读起来像是产品的还原与最小化都坏了。
    let button = |nth: i32| {
        let geom = window_geometry(conn, window);
        (
            geom.0 + geometry::button_center_x(i32::from(geom.2), nth),
            geom.1 + geometry::mid_y(),
        )
    };

    // 最大化。
    let shot = shots.join("control-maximize.png");
    click_at(&mut enigo, button(geometry::MAXIMIZE_FROM_RIGHT), false)?;
    let maximized = Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window));
    let _ = screenshot::capture(conn, root_window, &shot);
    record_bool(
        collector,
        "control_maximize_works",
        maximized,
        "点最大化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_MAXIMIZED_VERT/HORZ`",
        "点最大化按钮后窗口状态未变；capabilities 里少了 `core:window:allow-toggle-maximize` \
         会让它静默无效（注意 `allow-internal-toggle-maximize` 是**另一条**命令）",
        &shot,
    )?;

    // 还原。
    let shot = shots.join("control-restore.png");
    click_at(&mut enigo, button(geometry::MAXIMIZE_FROM_RIGHT), false)?;
    let restored = Background::wait_until(STATE_TIMEOUT, || !is_maximized(conn, window));
    let _ = screenshot::capture(conn, root_window, &shot);
    record_bool(
        collector,
        "control_restore_works",
        restored,
        "再点同一个按钮后最大化状态被清除，窗口还原",
        "再点同一个按钮后窗口仍处于最大化",
        &shot,
    )?;

    // 最小化。
    let shot = shots.join("control-minimize.png");
    click_at(&mut enigo, button(geometry::MINIMIZE_FROM_RIGHT), false)?;
    let minimized = Background::wait_until(STATE_TIMEOUT, || is_hidden(conn, window));
    let _ = screenshot::capture(conn, root_window, &shot);
    record_bool(
        collector,
        "control_minimize_works",
        minimized,
        "点最小化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_HIDDEN`",
        "点最小化按钮后窗口未进入隐藏态",
        &shot,
    )?;
    // 还原出来，关闭按钮才点得到。
    if minimized {
        activate(conn, root_window, window);
        Background::wait_until(STATE_TIMEOUT, || !is_hidden(conn, window));
    }

    let shot = shots.join("control-close.png");
    click_at(&mut enigo, button(geometry::CLOSE_FROM_RIGHT), false)?;
    let hidden_to_tray = Background::wait_until(Duration::from_secs(10), || {
        !top_level_windows(conn, root_window).contains(&window)
    });
    let process_still_running = app_process.child.try_wait()?.is_none();
    let _ = screenshot::capture(conn, root_window, &shot);
    record_bool(
        collector,
        "control_close_works",
        hidden_to_tray && process_still_running,
        "点关闭按钮后主窗口从 `_NET_CLIENT_LIST` 消失，应用进程仍在运行，符合驻留托盘契约",
        if hidden_to_tray {
            "主窗口虽已隐藏，但应用进程也退出了，破坏驻留托盘契约"
        } else {
            "点关闭按钮后主窗口仍在 `_NET_CLIENT_LIST` 里"
        },
        &shot,
    )?;

    collector.record(
        "app_exits_cleanly",
        Verdict::NotExecuted,
        "应用的正常退出入口是托盘菜单「退出」；本次 Xvfb + Openbox 会话没有托盘宿主，\
         harness 无法显示并点击该菜单，未用 kill 信号冒充正常退出",
        Some(
            "带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，\
             并由 harness 观测托盘项后点击「退出」"
                .to_owned(),
        ),
        None,
    )?;
    Ok(())
}

fn record_bool(
    collector: &mut Collector,
    id: &str,
    ok: bool,
    pass: &str,
    fail: &str,
    shot: &Path,
) -> Result<()> {
    collector.record(
        id,
        if ok { Verdict::Pass } else { Verdict::Fail },
        if ok { pass } else { fail },
        None,
        Some(relative_shot(shot)),
    )
}

/// 往检索框打中文，再聚焦一次继续打，然后看窗口是否还响应。
///
/// 这是 `tauri#15436` 在 OS 层可证的那一半：该缺陷的症状是**冻结**，
/// 而冻结意味着窗口不再处理窗口管理器请求。
fn chinese_input_keeps_ui_responsive(
    conn: &RustConnection,
    window: Window,
    enigo: &mut Enigo,
) -> Result<bool> {
    let geom = window_geometry(conn, window);
    // 检索框在导航条下方。这里点在窗口靠上的内容区里——具体元素位置读不到（那要 DOM），
    // 但输入焦点跟着点击走，而本条判的是「响应性」而不是「字符落在哪」。
    let field = (geom.0 + 200, geom.1 + geometry::content_top_y());
    click_at(enigo, field, false)?;
    enigo.text("明月").context("合成中文输入失败")?;
    std::thread::sleep(Duration::from_millis(500));
    // 再聚焦一次：缺陷的触发条件是「聚焦一个已经有内容的输入框」。
    click_at(enigo, field, false)?;
    enigo.text("千里").context("合成中文输入失败")?;
    std::thread::sleep(Duration::from_millis(500));

    // 响应性判据：窗口是否还能被最大化并还原。冻住的窗口不会迁移状态。
    let before = is_maximized(conn, window);
    let drag_point = (
        geom.0 + geometry::title_text_x(),
        geom.1 + geometry::mid_y(),
    );
    click_at(enigo, drag_point, true)?;
    let toggled = Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window) != before);
    if toggled {
        click_at(enigo, drag_point, true)?;
        Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window) == before);
    }
    Ok(toggled)
}

fn click_at(enigo: &mut Enigo, point: (i32, i32), double: bool) -> Result<()> {
    enigo
        .move_mouse(point.0, point.1, Coordinate::Abs)
        .context("移动指针失败")?;
    std::thread::sleep(Duration::from_millis(120));
    enigo
        .button(Button::Left, Direction::Click)
        .context("按下左键失败")?;
    if double {
        // 双击间隔要短于系统阈值，否则会被读成两次单击。
        std::thread::sleep(Duration::from_millis(60));
        enigo
            .button(Button::Left, Direction::Click)
            .context("第二次按下左键失败")?;
    }
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

fn drag(enigo: &mut Enigo, from: (i32, i32), by: (i32, i32)) -> Result<()> {
    enigo
        .move_mouse(from.0, from.1, Coordinate::Abs)
        .context("移动指针失败")?;
    std::thread::sleep(Duration::from_millis(120));
    enigo
        .button(Button::Left, Direction::Press)
        .context("按住左键失败")?;
    // 分步移动：一步跳到终点常常被窗口管理器读成一次没有中间过程的传送，
    // 从而不触发拖动阈值。
    for step in 1..=6 {
        enigo
            .move_mouse(
                from.0 + by.0 * step / 6,
                from.1 + by.1 * step / 6,
                Coordinate::Abs,
            )
            .context("拖动中移动指针失败")?;
        std::thread::sleep(Duration::from_millis(60));
    }
    enigo
        .button(Button::Left, Direction::Release)
        .context("松开左键失败")?;
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

fn wait_for_window(conn: &RustConnection, root: Window) -> Option<Window> {
    let mut found = None;
    Background::wait_until(WINDOW_TIMEOUT, || {
        for window in top_level_windows(conn, root) {
            let title = window_title(conn, window).unwrap_or_default();
            if title.contains('云') || title.contains("yunjian") {
                found = Some(window);
                return true;
            }
        }
        false
    });
    found
}

fn top_level_windows(conn: &RustConnection, root: Window) -> Vec<Window> {
    let Some(atom) = intern(conn, b"_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    conn.get_property(false, root, atom, AtomEnum::WINDOW, 0, 1024)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(Iterator::collect))
        .unwrap_or_default()
}

fn window_title(conn: &RustConnection, window: Window) -> Option<String> {
    let atom = intern(conn, b"_NET_WM_NAME")?;
    let utf8 = intern(conn, b"UTF8_STRING")?;
    let reply = conn
        .get_property(false, window, atom, utf8, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

/// 窗口在根坐标系下的 x、y、宽、高。
fn window_geometry(conn: &RustConnection, window: Window) -> (i32, i32, u16, u16) {
    let Some(geom) = conn
        .get_geometry(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return (0, 0, 0, 0);
    };
    let root = conn.setup().roots[0].root;
    let translated = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()
        .and_then(|c| c.reply().ok());
    let (x, y) = translated.map_or((i32::from(geom.x), i32::from(geom.y)), |t| {
        (i32::from(t.dst_x), i32::from(t.dst_y))
    });
    (x, y, geom.width, geom.height)
}

/// 标题文字上一点的**当前**屏幕坐标。
///
/// 每次调用都重读几何。最大化与还原都会同时改变窗口的位置与尺寸，缓存一份快照复用，
/// 落点就会在状态迁移之后落到内容区里，而那种点击是静默无效的。
fn title_point(conn: &RustConnection, window: Window) -> (i32, i32) {
    let geom = window_geometry(conn, window);
    (
        geom.0 + geometry::title_text_x(),
        geom.1 + geometry::mid_y(),
    )
}

fn frame_extents(conn: &RustConnection, window: Window) -> Option<Vec<u32>> {
    let atom = intern(conn, b"_NET_FRAME_EXTENTS")?;
    let reply = conn
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let values: Vec<u32> = reply.value32()?.collect();
    (!values.is_empty()).then_some(values)
}

fn icon_property_len(conn: &RustConnection, window: Window) -> usize {
    let Some(atom) = intern(conn, b"_NET_WM_ICON") else {
        return 0;
    };
    conn.get_property(false, window, atom, AtomEnum::CARDINAL, 0, u32::MAX / 4)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|reply| reply.value32().map(|v| v.count()))
        .unwrap_or(0)
}

fn wm_state(conn: &RustConnection, window: Window) -> Vec<u32> {
    let Some(atom) = intern(conn, b"_NET_WM_STATE") else {
        return Vec::new();
    };
    conn.get_property(false, window, atom, AtomEnum::ATOM, 0, 64)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|reply| reply.value32().map(Iterator::collect))
        .unwrap_or_default()
}

fn is_maximized(conn: &RustConnection, window: Window) -> bool {
    let state = wm_state(conn, window);
    let vert = intern(conn, b"_NET_WM_STATE_MAXIMIZED_VERT");
    let horz = intern(conn, b"_NET_WM_STATE_MAXIMIZED_HORZ");
    match (vert, horz) {
        (Some(v), Some(h)) => state.contains(&v) && state.contains(&h),
        _ => false,
    }
}

fn is_hidden(conn: &RustConnection, window: Window) -> bool {
    let state = wm_state(conn, window);
    intern(conn, b"_NET_WM_STATE_HIDDEN").is_some_and(|atom| state.contains(&atom))
}

/// 请窗口管理器把窗口激活（从最小化里拉回来）。
fn activate(conn: &RustConnection, root: Window, window: Window) {
    use x11rb::protocol::xproto::{ClientMessageEvent, EventMask};
    let Some(atom) = intern(conn, b"_NET_ACTIVE_WINDOW") else {
        return;
    };
    let event = ClientMessageEvent::new(32, window, atom, [2, 0, 0, 0, 0]);
    let _ = conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
        event,
    );
    let _ = conn.flush();
}

/// 任务栏图标。**不依赖 WebView 绘制**：它是窗口管理器读的一条 X 属性。
///
/// **必须轮询**。GTK 是在窗口映射之后才发布 `_NET_WM_ICON` 的，映射一成功就单次读取
/// 会读到「属性不存在」——本次开发中它确实产生了一个假的 FAIL，而顺着那个假 FAIL 去
/// 「修」产品（在 `setup` 里再调一次 `set_icon`）反而破坏了真正生效的那条路径：`tao`
/// 建窗时已按 `bundle.icon` 设过图标并写了 `_NET_WM_ICON`，再设一次会把它换成只有
/// ICCCM `WM_HINTS` 的图标像素图。实测：不改 = 2306 字，改了 = 属性消失。
///
/// # Errors
///
/// 记录裁决失败（harness 自身的缺陷）。
fn taskbar_icon(
    conn: &RustConnection,
    window: Window,
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let mut icon_words = 0;
    Background::wait_until(STATE_TIMEOUT, || {
        icon_words = icon_property_len(conn, window);
        icon_words > 0
    });
    let shot = shots.join("taskbar-icon.png");
    let _ = screenshot::capture(conn, window, &shot);
    if icon_words > 0 {
        collector.record(
            "taskbar_icon_correct",
            Verdict::Pass,
            format!(
                "`_NET_WM_ICON` 有 {icon_words} 个 32 位字（宽高 2 字 + 逐像素 ARGB），\
                 任务栏与 alt-tab 能取到图标"
            ),
            None,
            Some(relative_shot(&shot)),
        )
    } else {
        collector.record(
            "taskbar_icon_correct",
            Verdict::Fail,
            format!(
                "等了 {} 秒仍没有 `_NET_WM_ICON`：Linux 任务栏与 alt-tab 取不到图标，只会显示占位图",
                STATE_TIMEOUT.as_secs()
            ),
            None,
            Some(relative_shot(&shot)),
        )
    }
}

/// 托盘图标。
///
/// # Errors
///
/// 记录裁决失败（harness 自身的缺陷）。
fn tray_icon(collector: &mut Collector) -> Result<()> {
    collector.record(
        "tray_icon_correct",
        Verdict::NotExecuted,
        "应用已通过 `TrayIconBuilder` 创建托盘图标，但本次 Xvfb + Openbox 会话没有\
         StatusNotifier/AppIndicator 托盘宿主，harness 因而没有可观测的托盘项；\
         图标资产透明度仍由 `xtask verify-icons` 逐字节守卫",
        Some(
            "带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，\
             并由 harness 从托盘协议侧观测图标"
                .to_owned(),
        ),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_artifact_older_than_its_inputs_is_rejected() {
        let source = SystemTime::UNIX_EPOCH + Duration::from_secs(20);
        let binary = SystemTime::UNIX_EPOCH + Duration::from_secs(10);

        assert!(
            !artifact_is_current(binary, source),
            "陈旧二进制不得用于裁决当前源码；否则报告会把已修复问题重新报成产品缺陷"
        );
    }

    #[test]
    fn desktop_artifact_newer_than_its_inputs_is_accepted() {
        let source = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let binary = SystemTime::UNIX_EPOCH + Duration::from_secs(20);

        assert!(artifact_is_current(binary, source));
    }
}
