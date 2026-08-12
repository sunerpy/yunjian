//! 自绘标题栏的 capability 门禁：**「按钮需要的权限确实不在默认集里」这条正向对照**。
//!
//! # 为什么这条断言在 Rust 侧而不是前端
//!
//! 判据是 `crates/yunjian-app/gen/schemas/acl-manifests.json`——`tauri-build` 的产物，
//! 且在 `.gitignore` 里。一个没跑过 cargo 的检出里它根本不存在，所以从 `app/` 的 vitest
//! 读它只会得到 ENOENT。而编译**本文件**必然先跑 `build.rs`，产物必然已生成。
//! 前端那侧（`app/src/chrome/__tests__/contracts.test.ts`）负责另一半：capability 文件
//! 恰好列出那五项、不多不少。
//!
//! # 这条对照防的是什么
//!
//! 单看 `capabilities/default.json` 里多出四条权限，无法判断它们是必需的还是抄多的。
//! 而权限授多了**没有任何症状**。这里把「默认集不含它们」变成机器判定：
//! 若哪天 Tauri 把某一条收进 `core:window:default`，本文件会变红并提示那条可以删掉，
//! 而不是让一份已经无意义的清单继续躺着。
//!
//! # 两条 toggle-maximize 命令
//!
//! 已实测 `core:window:default` **含** `allow-internal-toggle-maximize`——那是 Tauri 注入的
//! `drag.js` 双击标题栏时发的命令（`plugin:window|internal_toggle_maximize`）。
//! 最大化**按钮**发的是另一条：`plugin:window|toggle_maximize`，要
//! `core:window:allow-toggle-maximize`。两条各自需要各自的权限，缺哪条都只是 IPC promise
//! 被拒、点了没反应，不报错。命令名那一半由前端的 `windowControls.test.ts` 实测钉住。

use serde_json::Value;
use std::path::PathBuf;

/// 自绘标题栏在 `core:default` 之外额外需要的窗口权限，去掉 `core:window:` 前缀。
///
/// 每一条对着 `app/src/chrome/windowControls.ts` 里 `WindowControls` 的一个方法。
/// 接口加方法必须同步加权限，反之亦然。
const EXTRA_WINDOW_PERMISSIONS: &[&str] = &[
    "allow-minimize",
    "allow-toggle-maximize",
    "allow-close",
    "allow-start-dragging",
];

/// 已实测由 `core:window:default` 提供、因此**不该**再显式授一遍的权限。
const PROVIDED_BY_WINDOW_DEFAULT: &[&str] = &[
    // 最大化状态查询。状态层每次 resize 都要问它。
    "allow-is-maximized",
    // 注入脚本双击时用的那条命令，与按钮用的 `allow-toggle-maximize` 不是一条。
    "allow-internal-toggle-maximize",
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(relative: &str) -> Value {
    let path = crate_dir().join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", path.display()))
}

/// `core:window` 的默认权限集。
fn window_default_permissions() -> Vec<String> {
    let manifests = read_json("gen/schemas/acl-manifests.json");
    let list = manifests
        .get("core:window")
        .expect("acl-manifests.json 缺少 core:window 段")
        .get("default_permission")
        .expect("core:window 缺少 default_permission")
        .get("permissions")
        .and_then(Value::as_array)
        .expect("default_permission.permissions 不是数组");
    assert!(
        !list.is_empty(),
        "core:window 的默认权限集为空。这多半意味着 gen/schemas 是过期或残缺的产物，\
         而不是 Tauri 真的什么都不授——按空集判断会让下面每条断言都平凡通过。"
    );
    list.iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn declared_permissions() -> Vec<String> {
    read_json("capabilities/default.json")
        .get("permissions")
        .and_then(Value::as_array)
        .expect("capabilities/default.json 缺少 permissions 数组")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[test]
fn extra_window_permissions_are_genuinely_absent_from_the_default_set() {
    let defaults = window_default_permissions();

    for permission in EXTRA_WINDOW_PERMISSIONS {
        assert!(
            !defaults.contains(&(*permission).to_owned()),
            "`{permission}` 已经在 core:window 的默认集里了，`capabilities/default.json` \
             里那条显式授权成了冗余。删掉它，并把它从本文件的 EXTRA_WINDOW_PERMISSIONS \
             移到 PROVIDED_BY_WINDOW_DEFAULT。默认集实际为 {defaults:?}"
        );
    }
}

#[test]
fn permissions_already_in_the_default_set_are_not_granted_twice() {
    let defaults = window_default_permissions();
    let declared = declared_permissions();

    for permission in PROVIDED_BY_WINDOW_DEFAULT {
        assert!(
            defaults.contains(&(*permission).to_owned()),
            "`{permission}` 不在 core:window 的默认集里了（默认集实际为 {defaults:?}）。\
             状态层依赖 `allow-is-maximized`，注入脚本的双击依赖 \
             `allow-internal-toggle-maximize`；上游收回其中任何一条都必须在 \
             capabilities 里补授，否则相应功能会静默失效。"
        );
        let explicit = format!("core:window:{permission}");
        assert!(
            !declared.contains(&explicit),
            "`{explicit}` 被显式授了一遍，而它已由 core:default 提供。\
             重复授权不会报错也不会有症状，只会让「最小权限集」这句话失真。"
        );
    }
}

#[test]
fn capability_grants_exactly_the_minimal_set() {
    let mut declared = declared_permissions();
    declared.sort();

    let mut expected = vec!["core:default".to_owned()];
    expected.extend(
        EXTRA_WINDOW_PERMISSIONS
            .iter()
            .map(|permission| format!("core:window:{permission}")),
    );
    expected.sort();

    assert_eq!(
        declared, expected,
        "capabilities/default.json 授的权限与最小集不符。\
         权限是攻击面，多授一条不会有任何症状——只能靠这条断言把它变成一次可见的决定。"
    );
}

#[test]
fn capability_targets_the_same_window_label_as_both_configs() {
    let capability = read_json("capabilities/default.json");
    let windows = capability
        .get("windows")
        .and_then(Value::as_array)
        .expect("capabilities/default.json 缺少 windows 数组")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();

    // capabilities 是按 label 授权的。label 漂移会让权限全部落空且**不报错**，
    // 症状是「窗口按钮在某个平台上全无反应」。
    assert_eq!(
        windows,
        vec!["main"],
        "capability 的授权窗口应当恰好是 main，与两份 tauri 配置里的 label 一致"
    );

    for config in ["tauri.conf.json", "tauri.macos.conf.json"] {
        let label = read_json(config)
            .get("app")
            .and_then(|app| app.get("windows"))
            .and_then(Value::as_array)
            .and_then(|list| list.first().cloned())
            .and_then(|window| window.get("label").cloned())
            .and_then(|label| label.as_str().map(str::to_owned))
            .unwrap_or_else(|| panic!("{config} 的主窗口缺少 label"));
        assert_eq!(
            label, "main",
            "{config} 的主窗口 label 与 capability 的授权目标不一致"
        );
    }
}

#[test]
fn the_frontend_ipc_layer_declares_exactly_the_methods_these_permissions_cover() {
    // 权限清单与前端接口是一对必须同步的东西，而漂移的方向是单向危险的：
    // 接口加方法却忘了加权限 = 新按钮静默失效。这里按方法名逐一核对，
    // 让「加方法」这一步在门禁上必须同时改权限。
    let source =
        std::fs::read_to_string(crate_dir().join("../../app/src/chrome/windowControls.ts"))
            .expect("读 app/src/chrome/windowControls.ts");

    for (permission, method) in [
        ("allow-minimize", "minimize("),
        ("allow-toggle-maximize", "toggleMaximize("),
        ("allow-close", "close("),
        ("allow-start-dragging", "startDragging("),
    ] {
        assert!(
            source.contains(method),
            "`{permission}` 授了权，但前端的 WindowControls 里没有 `{method}` 这个方法。\
             要么方法被删了（那么这条权限该收回），要么改了名（那么这份对照表要跟上）。"
        );
    }

    // 反向：接口里不该出现一个没授权的窗口动作。这条只能列举，所以列的是最容易被顺手
    // 加上、且各自需要独立权限的几个。
    for forbidden in ["setFullscreen(", "setAlwaysOnTop(", "setDecorations("] {
        assert!(
            !source.contains(forbidden),
            "WindowControls 里出现了 `{forbidden}`，但 capabilities 没有授对应权限，\
             调用它只会得到一个被拒的 promise。要用就先授权，并把它加进本文件的对照表。"
        );
    }
}
