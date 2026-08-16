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

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use enigo::{Button, Coordinate, Direction, Enigo, Keyboard as _, Mouse as _, Settings};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::{
    APP_BINARY, Background, Collector, SessionFacts, Verdict, WebDriverProbe, dom, geometry,
    screenshot, tray, webdriver,
};
use crate::verify_sources::emit;

/// 自建虚拟显示用的编号。取一个不太可能被占的号。
const VIRTUAL_DISPLAY: &str = ":99";
/// 虚拟屏幕尺寸。要比窗口的 1200×800 大，否则「最大化」与「原状」量不出差别。
const VIRTUAL_SCREEN: &str = "1600x1100x24";
/// 截图子目录，相对 `docs/reports/`。
pub(crate) const SHOT_DIR: &str = "desktop-qa";
/// 等窗口出现的上限。debug 构建首帧要几秒。
const WINDOW_TIMEOUT: Duration = Duration::from_secs(30);
/// 等窗口状态迁移的上限。窗口管理器处理一次请求是毫秒级，给足余量。
///
/// **不是这个值的问题**：本轮把它加到 20 秒，`control_maximize_works` 与
/// `control_restore_works` 的判词完全不变，因此那两条不是等得不够久。
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// 等 WebView 画出第一帧的上限。debug 构建要加载并执行整个前端包。
const PAINT_TIMEOUT: Duration = Duration::from_secs(20);
/// 打断连击链的等待时长。GTK 默认双击时间是 400 毫秒，取两倍余量。
const CHAIN_ESCAPE_WAIT: Duration = Duration::from_millis(800);
/// 打断连击链时把指针移开的距离，px。GTK 默认双击距离是 5 px，取远超它的值。
const CHAIN_ESCAPE_PX: i32 = 200;

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

    if !artifact_ok {
        check_installer(root, None, collector)?;
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

    // 一条自建的 session 总线，供本次验收里所有子进程共用。
    //
    // 容器里 `DBUS_SESSION_BUS_ADDRESS` 常常指着一个已经死掉的 socket，而 WebKitGTK 的
    // 自动化会话与托盘协议都要经总线。这条总线同时是托盘断言可执行的前提。
    let bus = tray::SessionBus::start();
    if let Err(error) = &bus {
        emit(&format!("  会话总线起不来：{error}"));
    }
    let bus = bus.ok();
    let bus_env: Vec<(String, String)> = bus
        .as_ref()
        .map(|bus| {
            let (key, value) = bus.env();
            vec![(key.to_owned(), value)]
        })
        .unwrap_or_default();

    // 安装包那条要一个可用的显示（它的判据是「从安装路径启动的那个二进制映射出了窗口」），
    // 所以排在会话拿到之后。
    check_installer(root, Some(&session.display), collector)?;

    // --- DOM 层断言：三条会话，各自带着自己那条断言要求的前置环境 ---
    //
    // 不是一条会话跑六条：首启物化要一个空数据目录、语音降级要一个取不到模型的环境，
    // 而这两件事与其余四条要的「真实数据目录」互斥，且环境在进程启动时就定死了。
    emit("  探测 WebDriver 握手（真实 tauri-driver + WebKitWebDriver，无 mock）");
    let primary_env =
        dom::SessionEnv::new(&session.display).with_extra(merged_env(library_env(&app), &bus_env));
    let mut probe_facts = None;
    let primary_ok = dom::with_session(
        &app,
        &primary_env,
        &[
            "two_char_search_returns_results",
            "ime_prefilled_search_box",
            "shipped_appreciation_without_key",
            "voice_round_succeeds_end_to_end",
        ],
        collector,
        |driver, collector| dom::drive_primary(driver, &shots, audio, collector),
    );
    let primary_ok = primary_ok?;
    probe_facts.get_or_insert(WebDriverProbe {
        attempted: true,
        succeeded: primary_ok,
        detail: if primary_ok {
            "会话建立成功，DOM 层断言由真实 WebDriver 执行。".to_owned()
        } else {
            "主会话未能建立，DOM 层断言未执行；逐条依据写在各断言的判词里。".to_owned()
        },
    });

    first_run_session(root, &app, &session.display, &bus_env, &shots, collector)?;
    degradation_session(root, &app, &session.display, &bus_env, &shots, collector)?;

    // --- OS 层断言：真实窗口 + 合成输入 ---
    os_assertions(
        &conn,
        root_window,
        OsContext {
            session: &session,
            app: &app,
            bus,
            bus_env: &bus_env,
            shots: &shots,
        },
        collector,
    )?;

    // 兜底：任何没走到的条目都以未执行入账，而不是从报告里消失。
    collector.fill_remaining(
        "harness 在到达这条之前结束",
        "重跑本子命令并检查上面失败的那一步",
    )?;

    Ok(SessionFacts {
        display_kind: session.kind.to_owned(),
        display: Some(session.display.clone()),
        window_manager: wm,
        webdriver: probe_facts.unwrap_or(WebDriverProbe {
            attempted: true,
            succeeded: false,
            detail: "未记录握手结果。".to_owned(),
        }),
        audio_input_devices: audio,
    })
}

/// 产物自带的那些 `.so` 所在目录。
///
/// 开了 `voice` 特性的产物要链 `libsherpa-onnx-c-api.so`，而它被放在**产物旁边**，
/// 不在系统库路径上；Linux 的加载器默认不看可执行文件所在目录（没有 rpath），
/// 于是一次裸启动会以 `error while loading shared libraries` 立刻失败。
///
/// 本机实测过那个后果：`tauri-driver` 拉起应用失败，而 `POST /session` 那侧看到的是
/// **请求挂到超时**，判词会写成「`WebKitWebDriver` 从不建立连接」——一条把人引向
/// WebKit 的错误信息，真因却是少一个库路径。
///
/// 这里让 harness 按产物自己的布局去启动它：`ldd` 检查与真正启动用**同一份**环境，
/// 两者口径一致。安装包那条断言刻意不加这个环境——它要证明的正是安装后的布局自足。
/// 合并两组环境变量。
fn merged_env(
    mut first: Vec<(String, String)>,
    second: &[(String, String)],
) -> Vec<(String, String)> {
    first.extend(second.iter().cloned());
    first
}

fn library_env(app: &Path) -> Vec<(String, String)> {
    let Some(dir) = app.parent() else {
        return Vec::new();
    };
    let has_local_libs = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("libsherpa-onnx")
        })
    });
    if !has_local_libs {
        return Vec::new();
    }
    let mut value = dir.to_string_lossy().into_owned();
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH")
        && !existing.is_empty()
    {
        value.push(':');
        value.push_str(&existing.to_string_lossy());
    }
    vec![("LD_LIBRARY_PATH".to_owned(), value)]
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
        .envs(library_env(app))
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

/// 安装包：解到一个临时 root，再**从安装后的那个路径**启动。
///
/// 判据刻意是「从安装路径启动的那个二进制映射出了窗口」，而不是「产物文件存在」：
/// 后者已经由 `artifact_present` 判了，而安装包能坏在别处——漏打 sidecar、
/// `.desktop` 指到不存在的路径、动态库依赖在安装后解不开。这些只有真的从安装路径
/// 跑一次才看得见。
///
/// 用 `dpkg-deb -x` 解到临时目录而**不是** `dpkg -i`：后者要 root、会动宿主机的包数据库，
/// 而本条要证明的是包里的内容能跑，不是宿主机的 dpkg 能装。
fn check_installer(root: &Path, display: Option<&str>, collector: &mut Collector) -> Result<()> {
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
        return Ok(());
    }

    let Some(deb) = newest_file(&bundle.join("deb"), "deb") else {
        collector.record(
            "installer_runs",
            Verdict::NotExecuted,
            format!(
                "发现产物 {}，但其中没有 `.deb`；本 harness 的安装路径实现基于 `dpkg-deb -x`，\
                 AppImage 与 rpm 各自要另一套解包与启动步骤",
                found.join("、")
            ),
            Some("`cargo tauri build --bundles deb` 产出一个 `.deb`".to_owned()),
            None,
        )?;
        return Ok(());
    };
    let Some(dpkg_deb) = webdriver::which("dpkg-deb") else {
        collector.record(
            "installer_runs",
            Verdict::NotExecuted,
            "`dpkg-deb` 不在 PATH 上，无法把安装包解到临时 root".to_owned(),
            Some("安装 `dpkg` 后重跑".to_owned()),
            None,
        )?;
        return Ok(());
    };
    let Some(display) = display else {
        collector.record(
            "installer_runs",
            Verdict::NotExecuted,
            "没有可用的桌面会话，装好之后也无从证明它启动得起来".to_owned(),
            Some("一个可用的 X 显示".to_owned()),
            None,
        )?;
        return Ok(());
    };

    let install_root = root.join("target/acceptance/install-root");
    if install_root.exists() {
        std::fs::remove_dir_all(&install_root)
            .with_context(|| format!("清理上一次的 {} 失败", install_root.display()))?;
    }
    std::fs::create_dir_all(&install_root)
        .with_context(|| format!("创建 {} 失败", install_root.display()))?;

    let extract = Command::new(dpkg_deb)
        .arg("-x")
        .arg(&deb)
        .arg(&install_root)
        .output()
        .context("运行 dpkg-deb -x 失败")?;
    if !extract.status.success() {
        collector.record(
            "installer_runs",
            Verdict::Fail,
            format!(
                "`dpkg-deb -x {}` 失败：{}",
                deb.display(),
                String::from_utf8_lossy(&extract.stderr).trim()
            ),
            None,
            None,
        )?;
        return Ok(());
    }

    let installed = install_root.join("usr/bin/yunjian-desktop");
    if !installed.is_file() {
        collector.record(
            "installer_runs",
            Verdict::Fail,
            format!(
                "安装包解开后 {} 不存在；包里没有把可执行文件放到预期路径",
                installed.display()
            ),
            None,
            None,
        )?;
        return Ok(());
    }

    let launched = launch_from_install_root(&installed, display)?;
    match launched {
        Some(title) => collector.record(
            "installer_runs",
            Verdict::Pass,
            format!(
                "把 {} 解到 {} 后，从安装路径 {} 启动，映射出顶层窗口且 `_NET_WM_NAME` = 「{title}」",
                deb.file_name().unwrap_or_default().to_string_lossy(),
                install_root.display(),
                installed.display()
            ),
            None,
            None,
        )?,
        None => collector.record(
            "installer_runs",
            Verdict::Fail,
            format!(
                "把 {} 解到 {} 后，从安装路径 {} 启动，但 {} 秒内没有映射出顶层窗口",
                deb.file_name().unwrap_or_default().to_string_lossy(),
                install_root.display(),
                installed.display(),
                WINDOW_TIMEOUT.as_secs()
            ),
            None,
            None,
        )?,
    }
    Ok(())
}

/// 从安装路径启动一次，返回窗口标题。
fn launch_from_install_root(binary: &Path, display: &str) -> Result<Option<String>> {
    let (conn, screen) =
        x11rb::connect(Some(display)).with_context(|| format!("连接 X 显示 {display} 失败"))?;
    let root_window = conn.setup().roots[screen].root;
    let before = top_level_windows(&conn, root_window);

    let mut command = Command::new(binary);
    command
        .env("DISPLAY", display)
        .env("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK", "1");
    let _process = Background::spawn("installed-yunjian-desktop", &mut command)?;

    let mut title = None;
    Background::wait_until(WINDOW_TIMEOUT, || {
        let fresh: Vec<Window> = top_level_windows(&conn, root_window)
            .into_iter()
            .filter(|window| !before.contains(window))
            .collect();
        for window in fresh {
            if let Some(name) = window_title(&conn, window)
                && !name.is_empty()
            {
                title = Some(name);
                return true;
            }
        }
        false
    });
    Ok(title)
}

/// 目录下最新的一个指定扩展名文件。
fn newest_file(dir: &Path, extension: &str) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(extension) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(best, _)| modified > *best) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
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

/// 首启语料物化那一条会话：把 `HOME` 换成一个空目录，只把生产路径要读的 `.db.gz`
/// 与它的校验和放进去，于是物化真的从零发生一次。
fn first_run_session(
    root: &Path,
    app: &Path,
    display: &str,
    bus_env: &[(String, String)],
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let Some(real_data_root) = dirs::data_dir().map(|dir| dir.join("yunjian")) else {
        collector.record(
            "corpus_first_run_materialization",
            Verdict::NotExecuted,
            "本机定位不到用户数据目录，因而找不到那份待物化的 `.db.gz`",
            Some("一个 `dirs::data_dir()` 能解析出结果的会话".to_owned()),
            None,
        )?;
        return Ok(());
    };
    let fresh = dom::temp_home(root, "first-run-root")?;
    let config = match dom::seed_first_run(&real_data_root, &fresh) {
        Ok(config) => config,
        Err(error) => {
            collector.record(
                "corpus_first_run_materialization",
                Verdict::NotExecuted,
                format!("无法为首启物化准备一个干净数据目录：{error}"),
                Some(
                    "本机 `~/.local/share/yunjian/corpus/` 下存在 `yunjian-corpus.db.gz` \
                     与它的 `.sha256`（先跑一次 `yunjian corpus fetch` 或从语料 release 取）"
                        .to_owned(),
                ),
                None,
            )?;
            return Ok(());
        }
    };
    emit("  首启物化会话（空数据目录，走生产解析路径）");
    let env = dom::SessionEnv::new(display)
        .with_config(config)
        .with_extra(merged_env(library_env(app), bus_env));
    dom::with_session(
        app,
        &env,
        &["corpus_first_run_materialization"],
        collector,
        |driver, collector| dom::drive_first_run(driver, shots, collector),
    )?;
    Ok(())
}

/// 语音降级那一条会话：把模型目录指到一个空目录，于是「模型未就绪」是一个被造出来的
/// 真实条件，而不是等它碰巧发生。
fn degradation_session(
    root: &Path,
    app: &Path,
    display: &str,
    bus_env: &[(String, String)],
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let empty = dom::empty_model_dir(root)?;
    emit("  语音降级会话（空模型目录）");
    let env = dom::SessionEnv::new(display)
        .with_model_dir(empty)
        .with_extra(merged_env(library_env(app), bus_env));
    dom::with_session(
        app,
        &env,
        &["voice_degradation_states_reason"],
        collector,
        |driver, collector| dom::drive_degradation(driver, shots, collector),
    )?;
    Ok(())
}

/// 相对报告目录的截图路径，写进报告用。
fn relative_shot(path: &Path) -> String {
    path.file_name()
        .map(|name| format!("{SHOT_DIR}/{}", name.to_string_lossy()))
        .unwrap_or_default()
}

/// OS 层断言：真实窗口 + 合成输入 + 属性观测。
/// OS 层断言要的那一组上下文。凑成一个结构体而不是继续加参数：八个位置参数里
/// 相邻两个都是 `&Path`，调用处传错顺序不会有编译错误。
struct OsContext<'a> {
    session: &'a Session0,
    app: &'a Path,
    bus: Option<tray::SessionBus>,
    bus_env: &'a [(String, String)],
    shots: &'a Path,
}

fn os_assertions(
    conn: &RustConnection,
    root_window: Window,
    context: OsContext<'_>,
    collector: &mut Collector,
) -> Result<()> {
    let OsContext {
        session,
        app,
        bus,
        bus_env,
        shots,
    } = context;
    // 托盘宿主先起来，再拉应用：`libayatana-appindicator` 只在**启动时**查一次
    // `org.kde.StatusNotifierWatcher` 在不在，晚一步声明就等于没有宿主。
    let tray_host = match bus.as_ref().map(tray::TrayHost::claim) {
        Some(Ok(host)) => Some(host),
        Some(Err(error)) => {
            emit(&format!("  托盘宿主声明不下来：{error}"));
            None
        }
        None => None,
    };

    let mut command = Command::new(app);
    command
        .env("DISPLAY", &session.display)
        .env("TAURI_WEBVIEW_AUTOMATION", "false")
        .env("YUNJIAN_DISABLE_STARTUP_UPDATE_CHECK", "1")
        .envs(merged_env(library_env(app), bus_env));
    let mut app_process = Background::spawn("yunjian-desktop", &mut command)?;

    let tray_item = tray_host.as_ref().and_then(tray::TrayHost::wait_for_item);
    let tray_host = tray_host.as_ref();
    let tray_item = tray_item.as_ref();

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
        // 托盘与正常退出都不经过 WebView 绘制：一条在 D-Bus 上，一条是进程生命周期。
        tray_icon(tray_host, tray_item, shots, collector)?;
        app_exits_cleanly(tray_host, tray_item, &mut app_process, collector)?;
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
    tray_icon(tray_host, tray_item, shots, collector)?;

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
    //
    // 起始态由 harness 在 X 层直接摆好，**不借用被测的那个按钮**去摆：用产品路径准备前置态，
    // 一旦产品坏了就会连带把本条的起始态弄错，于是判词说的是另一回事。
    let shot = shots.join("control-maximize.png");
    let normalized = ensure_not_maximized(conn, root_window, window);
    click_at(&mut enigo, button(geometry::MAXIMIZE_FROM_RIGHT), false)?;
    let maximized = Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window));
    let _ = screenshot::capture(conn, root_window, &shot);
    if normalized {
        record_bool(
            collector,
            "control_maximize_works",
            maximized,
            "起始态已归一为非最大化，点最大化按钮后 `_NET_WM_STATE` 带上了 \
             `_NET_WM_STATE_MAXIMIZED_VERT/HORZ`",
            "起始态已归一为非最大化，点最大化按钮后窗口状态仍未变；capabilities 里少了 \
             `core:window:allow-toggle-maximize` 会让它静默无效（注意 \
             `allow-internal-toggle-maximize` 是**另一条**命令）",
            &shot,
        )?;
    } else {
        collector.record(
            "control_maximize_works",
            Verdict::NotExecuted,
            "起始态归一失败：harness 请求窗口管理器清除最大化态之后，窗口仍处于最大化。\
             此时点「最大化」按钮不会有可观测的状态迁移，而那证不了按钮坏没坏",
            Some("一个会响应 `_NET_WM_STATE` 客户端消息的窗口管理器".to_owned()),
            Some(relative_shot(&shot)),
        )?;
    }

    // 还原。前置条件是「窗口此刻确实是最大化的」，而那正是上一条要判的事。
    // 上一条没成立时本条无从判定——把它也记成 FAIL 会把**一个**缺陷报成两个。
    let shot = shots.join("control-restore.png");
    if maximized {
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
    } else {
        let _ = screenshot::capture(conn, root_window, &shot);
        collector.record(
            "control_restore_works",
            Verdict::NotExecuted,
            "窗口此刻不是最大化态，「再点同一个按钮把它还原」这个动作没有前置条件；\
             最大化本身由 `control_maximize_works` 判定，这里不重复报告同一个缺陷",
            Some("`control_maximize_works` 先成立".to_owned()),
            Some(relative_shot(&shot)),
        )?;
    }

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

    app_exits_cleanly(tray_host, tray_item, &mut app_process, collector)?;
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
    //
    // 两次双击的落点都必须**当场**从 `title_point` 重读，不能复用上面那份 `geom`：
    // 第一次双击把窗口移到 (0,0) 并铺满屏幕，用旧几何算出来的点落进内容区，
    // 于是还原那一下静默无效、窗口留在最大化态。本机实测过那个后果——它不会让本条变红
    // （本条只判「状态迁移过」），而是把最大化态泄漏给后面的 `control_maximize_works`
    // 与 `control_restore_works`，让那两条各自看到相反的起始态、判词整体错位一格：
    // 前者说「状态未变」、后者说「仍处于最大化」，两句互相矛盾，却都指着没坏的产品。
    let before = is_maximized(conn, window);
    click_at(enigo, title_point(conn, window), true)?;
    let toggled = Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window) != before);
    if toggled {
        click_at(enigo, title_point(conn, window), true)?;
        Background::wait_until(STATE_TIMEOUT, || is_maximized(conn, window) == before);
    }
    Ok(toggled)
}

/// 让紧接着的那次按下成为一条**新**连击链的第一次，即 `e.detail == 1`。
///
/// GTK 按「时间够近 + 位置够近」把连续的按下并进同一条连击链，`e.detail` 逐次累加。
/// 而 Tauri 的拖动区处理只认两个值——`detail == 1` 发起拖动、`detail == 2` 切换最大化，
/// 见 `tauri` 的 `scripts/drag.js` 里那句 `(e.detail === 1 || e.detail === 2)`。
/// **`detail >= 3` 被这个守卫整体排除，于是既不拖动也不最大化，静默什么都不做。**
///
/// 本机实测（tauri 2.10.3 + WebKitGTK + Openbox）：在标题文字上双击一次再**在同一像素**
/// 按下拖动，第三次按下的 `e.detail` 是 3，X 上一条 `_NET_WM_MOVERESIZE` 都不会发出，
/// 窗口一动不动；把指针先移开再等过连击时间窗、同一动作重做一次，窗口立刻跟随移动。
/// 所以「点了没反应」在合成输入下往往是**上一条断言留下的连击链**，而不是产品坏了——
/// 真人不会在半秒内、同一像素上先双击再按下拖动。
///
/// 时间与距离都取远超 GTK 默认值（400 毫秒 / 5 像素）的余量，因为这两个值可被主题与
/// 用户设置改写，harness 不该依赖某个具体数字。
fn break_click_chain(enigo: &mut Enigo, target: (i32, i32)) -> Result<()> {
    let park = chain_escape_point(target);
    enigo
        .move_mouse(park.0, park.1, Coordinate::Abs)
        .context("移开指针以打断连击链失败")?;
    std::thread::sleep(CHAIN_ESCAPE_WAIT);
    Ok(())
}

/// 打断连击链时把指针停到哪。朝屏幕内侧让开，避免算出负坐标（X 会拒绝那种移动，
/// 于是指针留在原地、链没被打断，而调用方看不出区别）。
const fn chain_escape_point(target: (i32, i32)) -> (i32, i32) {
    const fn away(v: i32) -> i32 {
        if v > CHAIN_ESCAPE_PX {
            v - CHAIN_ESCAPE_PX
        } else {
            v.saturating_add(CHAIN_ESCAPE_PX)
        }
    }
    (away(target.0), away(target.1))
}

fn click_at(enigo: &mut Enigo, point: (i32, i32), double: bool) -> Result<()> {
    break_click_chain(enigo, point)?;
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
    // 见 `break_click_chain`：紧跟在别处点击之后的这一下若并进同一条连击链，
    // `e.detail` 会到 3，Tauri 的拖动区处理直接忽略，窗口一动不动。
    break_click_chain(enigo, from)?;
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

/// 请求窗口管理器清除最大化态，并等到它真的清除。已经是非最大化时是空操作。
///
/// 走 EWMH 的 `_NET_WM_STATE` 客户端消息，即窗口管理器自己那条路，刻意不碰产品的任何按钮。
fn ensure_not_maximized(conn: &RustConnection, root: Window, window: Window) -> bool {
    use x11rb::protocol::xproto::{ClientMessageEvent, EventMask};

    if !is_maximized(conn, window) {
        return true;
    }
    let Some((state, vert, horz)) = intern(conn, b"_NET_WM_STATE").and_then(|state| {
        let vert = intern(conn, b"_NET_WM_STATE_MAXIMIZED_VERT")?;
        let horz = intern(conn, b"_NET_WM_STATE_MAXIMIZED_HORZ")?;
        Some((state, vert, horz))
    }) else {
        return false;
    };
    // data: action(0 = remove), 第一个属性, 第二个属性, source(1 = 普通应用)
    let event = ClientMessageEvent::new(32, window, state, [0, vert, horz, 1, 0]);
    if conn
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )
        .is_err()
        || conn.flush().is_err()
    {
        return false;
    }
    Background::wait_until(STATE_TIMEOUT, || !is_maximized(conn, window));
    !is_maximized(conn, window)
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
/// 托盘图标：从**托盘协议侧**观测应用真的发布了什么。
///
/// 不判像素：Linux 上托盘项的图标由宿主按 `IconThemePath` + `IconName` 去取，
/// 屏幕上那几个像素是宿主画的，判它等于在判宿主。这里判的是应用这一侧可被观测到的
/// 三件事——托盘项注册了、图标文件存在且解得开、背景真的是透明的。
///
/// 透明度**在这里也真的解码一遍**，不只依赖 `xtask verify-icons`：那条守的是仓库里
/// 的资产，而本条要证明的是**运行期那个进程实际交出去的**那份图标。两者可以不同
/// （`tray-icon` 会把内嵌图片重新写到 `$XDG_RUNTIME_DIR` 下的一个临时 PNG）。
fn tray_icon(
    host: Option<&tray::TrayHost>,
    item: Option<&tray::TrayItem>,
    shots: &Path,
    collector: &mut Collector,
) -> Result<()> {
    let (Some(host), Some(item)) = (host, item) else {
        collector.record(
            "tray_icon_correct",
            Verdict::NotExecuted,
            "本次会话没有可观测的托盘项：harness 已在会话总线上声明 \
             `org.kde.StatusNotifierWatcher` 并等待注册，但应用在等待窗口内没有发起 \
             `RegisterStatusNotifierItem`",
            Some(
                "一个 `libayatana-appindicator` 能连上的 session 总线，\
                 且宿主名在应用启动**之前**已被声明"
                    .to_owned(),
            ),
            None,
        )?;
        return Ok(());
    };

    let icon_name = host.property(item, "IconName").unwrap_or_default();
    let theme_path = host.property(item, "IconThemePath").unwrap_or_default();
    let status = host.property(item, "Status").unwrap_or_default();
    let path = resolve_tray_icon(&icon_name, &theme_path);

    let Some(path) = path else {
        collector.record(
            "tray_icon_correct",
            Verdict::Fail,
            format!(
                "托盘项已注册，但它公开的图标定位不到文件：`IconName`=「{icon_name}」、\
                 `IconThemePath`=「{theme_path}」"
            ),
            None,
            None,
        )?;
        return Ok(());
    };

    // 证据就是那张图本身：托盘图标在这块 Xvfb 上没有面板去画它，而**应用运行期实际
    // 交出去的那个 PNG** 恰好是这条断言的被观测对象，所以把它原样收进报告目录。
    // 截一张空桌面的图会是一张不能证明任何事的图。
    let shot = shots.join("tray-icon.png");
    let copied = std::fs::copy(&path, &shot).is_ok();
    let shot_ref = copied.then(|| relative_shot(&shot));

    match transparent_pixel_ratio(&path) {
        Ok((width, height, ratio)) if ratio > 0.0 => collector.record(
            "tray_icon_correct",
            Verdict::Pass,
            format!(
                "托盘项已在会话总线上注册（`Status`=「{status}」），\
                 运行期图标为 {}（{width}×{height}），\
                 其中 {:.1}% 的像素 alpha 为 0，背景确为透明",
                path.display(),
                ratio * 100.0
            ),
            None,
            shot_ref.clone(),
        )?,
        Ok((width, height, _)) => collector.record(
            "tray_icon_correct",
            Verdict::Fail,
            format!(
                "托盘项已注册，但运行期图标 {}（{width}×{height}）没有任何 alpha 为 0 的像素，\
                 即背景不透明——在深色面板上会显示成一个色块",
                path.display()
            ),
            None,
            shot_ref.clone(),
        )?,
        Err(error) => collector.record(
            "tray_icon_correct",
            Verdict::Fail,
            format!(
                "托盘项已注册，但运行期图标 {} 解码失败：{error}",
                path.display()
            ),
            None,
            shot_ref.clone(),
        )?,
    }
    Ok(())
}

/// `IconName` 可能是一个绝对路径（`tray-icon` 就是这么给的），也可能是主题里的一个名字。
fn resolve_tray_icon(icon_name: &str, theme_path: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(icon_name);
    if direct.is_file() {
        return Some(direct);
    }
    let in_theme = PathBuf::from(theme_path).join(format!("{icon_name}.png"));
    in_theme.is_file().then_some(in_theme)
}

/// 解码 PNG 并数出 alpha 为 0 的像素占比。
fn transparent_pixel_ratio(path: &Path) -> Result<(u32, u32, f64)> {
    let file =
        std::fs::File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().context("读 PNG 头失败")?;
    let size = reader
        .output_buffer_size()
        .context("PNG 声明的输出缓冲区大小超出本机可表示范围")?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).context("解 PNG 帧失败")?;
    if info.color_type != png::ColorType::Rgba {
        bail!(
            "托盘图标不是 RGBA，无法判断透明度（实际 {:?}）",
            info.color_type
        );
    }
    let channels = 4;
    let total = (info.width as usize) * (info.height as usize);
    let frame = info.buffer_size();
    let transparent = buffer[..frame]
        .chunks_exact(channels)
        .filter(|pixel| pixel[3] == 0)
        .count();
    #[allow(clippy::cast_precision_loss)]
    let ratio = transparent as f64 / total as f64;
    Ok((info.width, info.height, ratio))
}

/// 正常退出：从托盘菜单点「退出」，等进程自己结束。
///
/// **不用 kill 信号冒充**。产品的正常退出入口就是这一项（`tray.rs` 里
/// `TrayAction::Quit` → `app.exit(0)`），而关窗按契约是隐藏到托盘。发一个信号能让进程
/// 消失，但那证明的是内核会杀进程，不是产品能正常退出。
fn app_exits_cleanly(
    host: Option<&tray::TrayHost>,
    item: Option<&tray::TrayItem>,
    app_process: &mut Background,
    collector: &mut Collector,
) -> Result<()> {
    let (Some(host), Some(item)) = (host, item) else {
        collector.record(
            "app_exits_cleanly",
            Verdict::NotExecuted,
            "应用的正常退出入口是托盘菜单「退出」；本次会话没有可观测的托盘项，\
             harness 因而无法点到那一项，未用 kill 信号冒充正常退出",
            Some(
                "一个应用会向其注册托盘项的 session 总线（harness 自带 \
                 `org.kde.StatusNotifierWatcher`，需在应用启动前声明）"
                    .to_owned(),
            ),
            None,
        )?;
        return Ok(());
    };

    let entries = match host.menu_entries(item) {
        Ok(entries) => entries,
        Err(error) => {
            collector.record(
                "app_exits_cleanly",
                Verdict::Fail,
                format!("托盘项已注册，但读不出它的菜单布局：{error}"),
                None,
                None,
            )?;
            return Ok(());
        }
    };
    let labels: Vec<&str> = entries.iter().map(|entry| entry.label.as_str()).collect();
    let Some(quit) = entries.iter().find(|entry| entry.label == "退出") else {
        collector.record(
            "app_exits_cleanly",
            Verdict::Fail,
            format!(
                "托盘菜单里没有「退出」这一项，实际有：{}",
                labels.join("、")
            ),
            None,
            None,
        )?;
        return Ok(());
    };

    host.activate(item, quit.id)?;
    match tray::wait_for_exit(&mut app_process.child)? {
        Some(0) => {
            let orphans = orphan_processes();
            if orphans.is_empty() {
                collector.record(
                    "app_exits_cleanly",
                    Verdict::Pass,
                    format!(
                        "从托盘菜单点「退出」（菜单项 id {}，实际菜单为 {}）后，\
                         应用以退出码 0 结束，且没有留下同名孤儿进程",
                        quit.id,
                        labels.join("、")
                    ),
                    None,
                    None,
                )?;
            } else {
                collector.record(
                    "app_exits_cleanly",
                    Verdict::Fail,
                    format!(
                        "从托盘菜单点「退出」后主进程以 0 结束，但还留着 {} 个同名进程：{}",
                        orphans.len(),
                        orphans.join("、")
                    ),
                    None,
                    None,
                )?;
            }
        }
        Some(code) => collector.record(
            "app_exits_cleanly",
            Verdict::Fail,
            format!("从托盘菜单点「退出」后应用以退出码 {code} 结束，不是 0"),
            None,
            None,
        )?,
        None => collector.record(
            "app_exits_cleanly",
            Verdict::Fail,
            "从托盘菜单点「退出」后应用在等待窗口内没有结束".to_owned(),
            None,
            None,
        )?,
    }
    Ok(())
}

fn orphan_processes() -> Vec<String> {
    super::running_app_pids()
        .into_iter()
        .map(|pid| format!("pid {pid}"))
        .collect()
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

    /// 本文件自己的源码。下面几条守卫扫它，因为它们要守的事实在**有副作用的函数体里**，
    /// 而那些函数要真实的 X 会话才跑得起来——真机验收只在有会话时跑，`make ci` 每次都跑。
    const SELF_SOURCE: &str = include_str!("linux.rs");

    /// 取一个函数从签名到下一个顶层 `fn` 之间的正文。
    fn body_of(name: &str) -> &'static str {
        let signature = format!("\nfn {name}(");
        let start = SELF_SOURCE
            .find(&signature)
            .unwrap_or_else(|| panic!("源码里找不到 `fn {name}`；改名了就同步改这条守卫"))
            + signature.len();
        let rest = &SELF_SOURCE[start..];
        rest.find("\nfn ").map_or(rest, |end| &rest[..end])
    }

    #[test]
    fn escape_point_is_farther_than_gtk_double_click_distance() {
        // GTK 默认 `gtk-double-click-distance` 是 5 px：只有落点足够近才算同一条连击链。
        const GTK_DEFAULT_DISTANCE: i32 = 5;

        for target in [(0, 0), (3, 4), (218, 170), (1531, 20), (i32::MAX, i32::MAX)] {
            let park = chain_escape_point(target);
            assert!(
                (park.0 - target.0).abs() > GTK_DEFAULT_DISTANCE
                    && (park.1 - target.1).abs() > GTK_DEFAULT_DISTANCE,
                "{target:?} 的让开点 {park:?} 离得不够远，连击链不会被打断"
            );
            assert!(
                park.0 >= 0 && park.1 >= 0,
                "{target:?} 的让开点 {park:?} 有负坐标；X 会拒绝这次移动，\
                 于是指针留在原地、链根本没断，而调用方看不出区别"
            );
        }
    }

    #[test]
    fn escape_wait_outlasts_gtk_double_click_time() {
        // GTK 默认 `gtk-double-click-time` 是 400 毫秒。
        assert!(
            CHAIN_ESCAPE_WAIT > Duration::from_millis(400),
            "等待短于 GTK 的双击时间窗，连击链不会断"
        );
    }

    #[test]
    fn synthetic_clicks_break_the_click_chain_first() {
        // 这两行看起来像可删的多余等待，删掉的后果却是静默的：紧跟在别处点击之后的那一下
        // `e.detail` 会累加到 3，而 Tauri 的拖动区处理只认 1 与 2（见 `break_click_chain`
        // 的文档），于是既不拖动也不最大化，报告把它写成「标题栏拖不动」，指着没坏的产品。
        // 匹配**调用**形态而不是名字：注释里也会提到这个函数，只查名字的守卫会被自己的
        // 解释文字满足，于是删掉那行调用它照样绿。这一点是注入失败实测出来的。
        for name in ["click_at", "drag"] {
            assert!(
                body_of(name).contains("break_click_chain(enigo"),
                "`{name}` 没有先打断连击链"
            );
        }
    }

    #[test]
    fn responsiveness_probe_rereads_geometry_for_each_double_click() {
        let body = body_of("chinese_input_keeps_ui_responsive");
        let rereads = body.matches("title_point(conn, window)").count();

        assert!(
            rereads >= 2,
            "两次双击都必须当场重读几何（当前只有 {rereads} 处）：第一次双击把窗口移到 (0,0) \
             并铺满，复用旧几何算出的点落进内容区，还原那一下静默无效，于是最大化态泄漏给 \
             `control_maximize_works` 与 `control_restore_works`，让那两条判词整体错位一格"
        );
    }

    #[test]
    fn restore_assertion_requires_maximize_to_have_succeeded() {
        let body = body_of("os_assertions");

        assert!(
            body.contains("ensure_not_maximized("),
            "最大化断言必须先把起始态归一，否则它继承的是上一条断言的残留态"
        );
        assert!(
            !body_of("ensure_not_maximized").contains("click_at"),
            "归一不能借用被测按钮：产品一坏就会连带把起始态弄错，于是判词说的是另一回事"
        );
        assert!(
            body.contains("if maximized {"),
            "还原断言必须以「最大化确实成立」为前置；否则一个缺陷会被报成两个 FAIL"
        );
    }
}
