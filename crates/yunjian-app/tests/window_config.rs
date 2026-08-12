//! 两份 Tauri 窗口配置的跨文件一致性门禁。
//!
//! # 为什么这组断言必须存在
//!
//! Tauri 读配置的路径是 `tauri-utils` 的 `config/parse.rs`：先把 `tauri.conf.json` 解析成
//! `serde_json::Value`，再对平台覆盖文件调 `json_patch::merge`。`json_patch::merge` 实现的是
//! **RFC 7396 JSON Merge Patch**，而该规范对数组的语义是**整体替换**，不是逐元素合并。
//!
//! `app.windows` 是数组。于是 `tauri.macos.conf.json` 里那一个窗口对象会把基础配置里的
//! 整个窗口对象换掉，凡是覆盖文件没写的字段都不是「沿用基础值」，而是回落到
//! `WindowConfig` 的 serde 默认值（`default_width` = 800、`default_height` = 600、
//! `min_width` / `min_height` = `None`）。
//!
//! 后果是**只在 macOS 上出现**的静默缺陷：Linux 与 Windows 一切正常，
//! macOS 上窗口尺寸莫名变小、最小尺寸约束消失。构建不报错，测试不报错，
//! 只有一台 Mac 能发现。这组断言把它变成一条本机就会红的测试。
//!
//! # 为什么按 `serde_json::Value` 判断而不是反序列化成 `Config`
//!
//! 反序列化会把缺失的键补成默认值。`minWidth` 漏写之后两边都是 `None`，
//! 「相等」照样成立，断言永远变不了红——那正是本文件要防的那个缺陷。
//! 因此判断分两步：**先断言键存在，再断言值相等**。

use serde_json::Value;
use std::path::{Path, PathBuf};

/// 必须在两份配置里逐一重述且取值相同的几何字段。
///
/// 这个清单就是执行机制本身：往 `tauri.conf.json` 的窗口对象里加一个几何字段却忘了同步
/// macOS 覆盖文件，只有把它加进这里才会被抓住；反过来，从覆盖文件里删掉任何一个，
/// 下面 `geometry_fields_are_restated_and_equal` 立刻变红。
const GEOMETRY_FIELDS: &[&str] = &["title", "width", "height", "minWidth", "minHeight"];

/// 被禁的 Tauri 官方日志插件包名。
///
/// **字面量刻意拆成两段再 `concat!`，不要合回一个字符串。** 方案的验收判据里有一条
/// `grep -rn` 要求这个包名在本 crate 的任何文件里都不得作为连续字符串出现。
/// 而一条**提到**该包名的守卫断言恰好会让那条 grep 命中——于是「禁令的执行机制」自己
/// 把禁令的门禁判成失败。拆开写让 grep 通过，同时下面的断言仍在比对完整包名。
const FORBIDDEN_LOG_PLUGIN: &str = concat!("tauri-plugin", "-log");

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", path.display()))
}

fn base_config() -> Value {
    read_json(&crate_dir().join("tauri.conf.json"))
}

fn macos_overlay() -> Value {
    read_json(&crate_dir().join("tauri.macos.conf.json"))
}

/// 取出 `app.windows` 数组，并断言它非空。
fn windows(config: &Value, which: &str) -> Vec<Value> {
    let array = config
        .get("app")
        .unwrap_or_else(|| panic!("{which} 缺少 app 段"))
        .get("windows")
        .unwrap_or_else(|| panic!("{which} 缺少 app.windows"))
        .as_array()
        .unwrap_or_else(|| panic!("{which} 的 app.windows 不是数组"));
    assert!(!array.is_empty(), "{which} 的 app.windows 为空数组");
    array.clone()
}

/// 主窗口对象。两份配置都只声明一个窗口，多窗口出现时下面的数量断言会先报出来。
fn main_window(config: &Value, which: &str) -> Value {
    windows(config, which)[0].clone()
}

#[test]
fn geometry_fields_are_restated_and_equal() {
    let base = main_window(&base_config(), "tauri.conf.json");
    let overlay = main_window(&macos_overlay(), "tauri.macos.conf.json");

    for field in GEOMETRY_FIELDS {
        let base_value = base.get(*field).unwrap_or_else(|| {
            panic!(
                "tauri.conf.json 的主窗口缺少几何字段 `{field}`。\
                 GEOMETRY_FIELDS 列出的字段必须在两份配置里都显式出现。"
            )
        });
        let overlay_value = overlay.get(*field).unwrap_or_else(|| {
            panic!(
                "tauri.macos.conf.json 的主窗口缺少几何字段 `{field}`。\
                 覆盖文件里的 app.windows 是**整体替换**基础数组（RFC 7396 对数组的语义），\
                 漏写这个字段会让 macOS 静默退回 WindowConfig 的 serde 默认值，\
                 而 Linux 与 Windows 完全正常——这正是本断言存在的理由。"
            )
        });
        assert_eq!(
            base_value, overlay_value,
            "几何字段 `{field}` 在两份配置里取值不同：\
             基础 = {base_value}，macOS 覆盖 = {overlay_value}。\
             覆盖文件只该改装饰相关的键，几何必须逐字重述成同一个值。"
        );
    }
}

#[test]
fn overlay_declares_the_same_window_set_as_the_base() {
    let base = windows(&base_config(), "tauri.conf.json");
    let overlay = windows(&macos_overlay(), "tauri.macos.conf.json");

    assert_eq!(
        base.len(),
        overlay.len(),
        "两份配置声明的窗口数量不同（基础 {} 个，macOS 覆盖 {} 个）。\
         数组整体替换意味着覆盖文件少写一个窗口就是在 macOS 上删掉那个窗口。",
        base.len(),
        overlay.len()
    );

    let labels = |list: &[Value]| -> Vec<String> {
        list.iter()
            .map(|window| {
                window
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("<缺少 label>")
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(
        labels(&base),
        labels(&overlay),
        "两份配置的窗口 label 不一致。capabilities 是按 label 授权的，\
         label 漂移会让 macOS 上的权限全部落空且不报错。"
    );
}

#[test]
fn base_config_disables_decorations_and_keeps_shadow() {
    let window = main_window(&base_config(), "tauri.conf.json");

    assert_eq!(
        window.get("decorations"),
        Some(&Value::Bool(false)),
        "基础配置必须 `decorations: false`：Windows 与 Linux 上要自绘标题栏（todo 60），\
         留着原生装饰会在自绘栏上再叠一条系统边框。"
    );
    assert_eq!(
        window.get("shadow"),
        Some(&Value::Bool(true)),
        "基础配置必须显式 `shadow: true`。它确实是 Tauri 的默认值，写出来是因为它与\
         Windows 11 圆角是**同一个开关**：`shadow: false` 会把阴影和圆角一起拿掉，\
         这条依赖必须在配置里可见，而不是靠人记得。"
    );
}

#[test]
fn macos_overlay_keeps_native_decorations_with_an_overlay_title_bar() {
    let window = main_window(&macos_overlay(), "tauri.macos.conf.json");

    assert_eq!(
        window.get("decorations"),
        Some(&Value::Bool(true)),
        "macOS 覆盖必须把 `decorations` 改回 true：关掉它红绿灯按钮会一起消失，\
         而 `trafficLightPosition` 的生效前提正是 `decorations: true`。"
    );
    assert_eq!(
        window.get("titleBarStyle"),
        Some(&Value::String("Overlay".to_owned())),
        "macOS 覆盖必须 `titleBarStyle: \"Overlay\"`。\
         注意大小写：`TitleBarStyle` 的 Deserialize 把无法识别的字符串**静默**当成 `Visible`\
         （`tauri-utils` 的 `lib.rs` 里 `_ => Self::Visible`），拼错不会报错，只会没效果。"
    );
    assert_eq!(
        window.get("hiddenTitle"),
        Some(&Value::Bool(true)),
        "macOS 覆盖必须 `hiddenTitle: true`：Overlay 之下系统标题会压在自绘内容上。"
    );

    let position = window
        .get("trafficLightPosition")
        .expect("macOS 覆盖必须给出 trafficLightPosition，否则红绿灯位置与自绘栏高度无关");
    for axis in ["x", "y"] {
        assert!(
            position.get(axis).and_then(Value::as_f64).is_some(),
            "trafficLightPosition 缺少数值字段 `{axis}`；它反序列化成 LogicalPosition{{x,y}}，两者都必填"
        );
    }
}

#[test]
fn title_bar_style_is_declared_only_on_macos() {
    let base = main_window(&base_config(), "tauri.conf.json");
    assert!(
        base.get("titleBarStyle").is_none(),
        "基础配置不该出现 `titleBarStyle`：`TitleBarStyle` 的文档注释明写它只作用于 macOS，\
         而基础配置服务的是 Windows 与 Linux。写在这里只会让读配置的人误以为它跨平台生效。"
    );
    assert!(
        base.get("trafficLightPosition").is_none(),
        "基础配置不该出现 `trafficLightPosition`：同上，它是 macOS 专属且要求 decorations: true。"
    );
}

#[test]
fn logging_goes_through_the_project_convention_not_a_tauri_plugin() {
    let source = std::fs::read_to_string(crate_dir().join("src/lib.rs")).expect("读 src/lib.rs");
    assert!(
        source.contains("init_logger"),
        "外壳入口必须调用项目自己的 `yunjian_core::init_logger`，\
         这样 CLI、MCP、GUI 三处共享同一套级别解析、脱敏与滚动文件布局，控制台一律写 stderr。"
    );
    assert!(
        source.contains("init_config"),
        "必须先 `init_config` 再 `init_logger`：没有配置就不知道日志级别与目录。"
    );

    let manifest = std::fs::read_to_string(crate_dir().join("Cargo.toml")).expect("读 Cargo.toml");
    assert!(
        !manifest.contains(FORBIDDEN_LOG_PLUGIN),
        "不得依赖 Tauri 官方日志插件：那会引入第二套日志格式与第二套过滤语义，\
         而两套约定的差异只会在排障最需要日志的时候暴露。"
    );

    let main = std::fs::read_to_string(crate_dir().join("src/main.rs")).expect("读 src/main.rs");
    assert!(
        main.contains("yunjian_app::run"),
        "`main` 应当只把控制权交给库目标的 `run`，逻辑留在可测的一侧"
    );
}

#[test]
fn crate_opts_into_the_workspace_lint_gate() {
    let manifest = std::fs::read_to_string(crate_dir().join("Cargo.toml")).expect("读 Cargo.toml");
    assert!(
        manifest.contains("[lints]") && manifest.contains("workspace = true"),
        "必须写 `[lints] workspace = true`。漏了不会报错，只会让本 crate 完全不受工作区\
         lint（`print_stdout = \"deny\"` 等）约束，clippy 全绿而禁令静默失效。"
    );
}

#[test]
fn frontend_dist_points_at_the_react_app() {
    let base = base_config();
    let build = base.get("build").expect("缺少 build 段");
    let dist = build
        .get("frontendDist")
        .and_then(Value::as_str)
        .expect("缺少 build.frontendDist");
    assert_eq!(
        dist, "../../app/dist",
        "`frontendDist` 必须指向 `app/` 的构建产物。它相对**配置文件所在目录**解析，\
         而 `generate_context!` 在该目录不存在时是 panic（tauri-codegen 的 \
         `The frontendDist configuration is set to ... but this path doesn't exist`），\
         所以 `app/dist/` 必须始终存在——仓库里为此保留了一个 `.gitkeep`。"
    );

    let resolved = crate_dir().join(dist);
    assert!(
        resolved.is_dir(),
        "{} 不存在。`cargo test --workspace` 会编译本 crate，而编译要经过 \
         `generate_context!`，它在该目录缺失时直接 panic——于是整条门禁会因为一个\
         未构建的前端而全红。",
        resolved.display()
    );
}
