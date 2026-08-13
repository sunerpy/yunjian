//! `bundle.icon` 的**顺序**门禁。
//!
//! # 为什么顺序是语义而不是排版
//!
//! `tauri-codegen` 的 `context.rs` 挑窗口图标的方式是
//! `config.bundle.icon.iter().find(|i| i.ends_with(".png"))` ——**第一个**匹配项，
//! 不是最大的那个、也不是名字最像的那个。于是把 `icons/32x32.png` 写在 `icons/icon.png`
//! 前面，窗口与任务栏图标就变成 32×32 的位图，在高 DPI 屏上是一团糊。
//!
//! 这个缺陷不会让构建失败、不会让 `verify-icons` 变红（那边验的是文件本身合不合格，
//! 不是配置引用了哪一个），也不会让任何现有测试变红。它只在**跑起来看窗口**时才暴露，
//! 而那正是最容易被跳过的一步。因此把它固定成一条本机就会红的断言。
//!
//! 三条 `find` 谓词各自独立（`.png` / `.ico` / `.icns`），所以三者都要断言。

use serde_json::Value;
use std::path::PathBuf;

/// 各扩展名下**必须排在最前**的那一项。
///
/// 取值就是执行机制本身：往 `bundle.icon` 里插一个更小的 PNG 到 `icon.png` 之前，
/// 下面的断言立刻变红。
const EXPECTED_FIRST: [(&str, &str); 3] = [
    (".png", "icons/icon.png"),
    (".ico", "icons/icon.ico"),
    (".icns", "icons/icon.icns"),
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bundle_icons() -> Vec<String> {
    let path = crate_dir().join("tauri.conf.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    let config: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", path.display()));
    config["bundle"]["icon"]
        .as_array()
        .expect("tauri.conf.json 的 bundle.icon 必须是数组")
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .expect("bundle.icon 的每一项都必须是字符串")
                .to_owned()
        })
        .collect()
}

#[test]
fn first_entry_per_extension_matches_what_codegen_picks() {
    let icons = bundle_icons();
    for (extension, expected) in EXPECTED_FIRST {
        let first = icons
            .iter()
            .find(|entry| entry.ends_with(extension))
            .unwrap_or_else(|| panic!("bundle.icon 里没有任何 {extension} 项：{icons:?}"));
        assert_eq!(
            first, expected,
            "tauri-codegen 取的是第一个以 {extension} 结尾的项。\
             当前第一个是 {first}，会让窗口图标退化。完整清单：{icons:?}"
        );
    }
}

#[test]
fn every_referenced_icon_exists_on_disk() {
    let dir = crate_dir();
    for entry in bundle_icons() {
        let path = dir.join(&entry);
        assert!(
            path.exists(),
            "bundle.icon 引用了不存在的 {entry}（{}）",
            path.display()
        );
    }
}

/// 托盘图标不在 `bundle.icon` 里（它由运行期代码加载，不是打包产物），
/// 所以上面那条存在性断言盖不到它。单独断言一条，否则删掉 `tray.png` 只会在
/// `verify-icons` 里红，而那条命令不属于 `make ci`。
#[test]
fn tray_icon_exists_because_bundle_icon_does_not_cover_it() {
    let path = crate_dir().join("icons/tray.png");
    assert!(path.exists(), "缺少托盘图标 {}", path.display());
}
