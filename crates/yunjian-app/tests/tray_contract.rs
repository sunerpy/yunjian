use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source(relative: &str) -> String {
    let path = crate_dir().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()))
}

#[test]
fn tray_module_is_wired_into_setup_and_window_events() {
    let lib = source("src/lib.rs");
    assert!(
        lib.contains("mod tray;"),
        "桌面外壳必须声明独立的 tray 模块"
    );
    assert!(
        lib.contains("tray::setup(app)"),
        "Tauri setup 必须创建菜单和托盘图标"
    );
    assert!(
        lib.contains(".on_window_event(tray::handle_window_event)"),
        "Builder 必须把 CloseRequested 交给托盘模块，关闭按钮才能驻留后台"
    );
}

#[test]
fn tray_source_keeps_the_four_required_actions_and_distinct_handlers() {
    let tray = source("src/tray.rs");
    for (id, label) in [
        ("toggle-window", "显示/隐藏主窗"),
        ("review-today", "今日复习"),
        ("settings", "设置"),
        ("quit", "退出"),
    ] {
        assert!(
            tray.contains(id) && tray.contains(label),
            "托盘菜单缺少 `{label}`（稳定 id `{id}`）"
        );
    }
    assert!(
        tray.contains(".show_menu_on_left_click(false)"),
        "左键用于切换窗口，必须关闭左键弹菜单"
    );
    assert!(
        tray.contains(".on_menu_event(") && tray.contains(".on_tray_icon_event("),
        "菜单事件与托盘鼠标事件必须分别注册，不能用其中一个冒充另一个"
    );
    assert!(
        tray.contains("MouseButtonState::Down"),
        "左键只在 Down 阶段切换一次，不能同时响应 Up/双击"
    );
    assert!(
        tray.contains("api.prevent_close()"),
        "主窗口 CloseRequested 必须 prevent_close 后隐藏"
    );
}
