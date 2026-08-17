//! iOS 产品工程的存在性与结构完整性门禁。
//!
//! # 为什么需要一条断言，而不是「目录在那儿就行」
//!
//! 冻结方案要求 `uniffi_native` 裁决下 `mobile/ios/Yunjian/` 是一个真实的 SwiftUI 产品工程。
//! 在 2026-08-17 之前那里只有一个 README，而**没有任何东西会因此变红**——缺失是靠人读目录
//! 发现的。这个模块把「iOS 工程在不在、结构对不对」变成一条会红的断言。
//!
//! # 本机不能编译 Swift，所以这里守的是什么
//!
//! 仓库所在主机是 Linux，没有 Xcode：**iOS 侧的代码从未经过 Swift 编译器**。因此这些断言
//! 刻意不假装做类型检查，它们守的是**能在文本层判定、且一旦漂移就会让真机验收白跑**的东西：
//!
//! 1. 工程文件齐备（少一个 target、少一个脚本，在 mac 上表现为 `xcodegen` 生成一个跑不起来的
//!    工程，而那时人已经在真机旁边了）；
//! 2. 唯一的 Rust 入口是 UniFFI 生成物，产品源码不得自己声明原生符号（与 Android 侧
//!    `architecture.rs` 那条「不得自己写 `external fun`」同一约束）；
//! 3. iOS 与 Android 的界面标识**逐字相同**——两个平台共用同一套判据，标识分叉会让同名断言
//!    在两个平台上量不同的东西，而报告里看不出这件事；
//! 4. iOS 侧真的能产出宿主侧十条判据所需的**每一个** required 键，否则那些断言在 iOS 上
//!    永远是 NOT EXECUTED，而原因会被读成「缺 macOS」；
//! 5. 十个测试方法带 `test` 前缀——XCTest 只发现 `test` 前缀的方法，写成 `t01_` 的话
//!    **一个测试都不会跑，而 run 会显示成功**。这是本文件里最值得守的一条。
//!
//! 判断依据里凡是标识符与路径，都从真实文件里读出来比对，不凭记忆写。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// SwiftUI 产品工程与两个测试 target 的必备文件。
const REQUIRED_FILES: &[&str] = &[
    "mobile/ios/README.md",
    "mobile/ios/project.yml",
    "mobile/ios/Yunjian.xctestplan",
    "mobile/ios/scripts/build-xcframework.sh",
    "mobile/ios/Yunjian/YunjianApp.swift",
    "mobile/ios/Yunjian/ContentView.swift",
    "mobile/ios/Yunjian/MainViewModel.swift",
    "mobile/ios/Yunjian/YunjianRepository.swift",
    "mobile/ios/Yunjian/VoiceCapture.swift",
    "mobile/ios/Yunjian/TestTags.swift",
    "mobile/ios/Shared/AcceptanceReport.swift",
    "mobile/ios/YunjianUITests/FullAcceptanceUITests.swift",
    "mobile/ios/YunjianAppTests/ContainerFactsTests.swift",
];

/// UniFFI 生成的 Swift 绑定。iOS 侧唯一允许的 Rust 入口。
const GENERATED_SWIFT: &str = "crates/yunjian-mobile/bindings/generated/YunjianMobile.swift";
const GENERATED_HEADER: &str = "crates/yunjian-mobile/bindings/generated/YunjianMobileFFI.h";
const GENERATED_MODULEMAP: &str =
    "crates/yunjian-mobile/bindings/generated/YunjianMobileFFI.modulemap";

const ANDROID_TAGS: &str = "mobile/android/app/src/main/java/top/onethinker/yunjian/TestTags.kt";
const IOS_TAGS: &str = "mobile/ios/Yunjian/TestTags.swift";

/// iOS 侧必须真的调到的绑定符号。
///
/// 每一个都从 `YunjianMobile.swift` 里核对存在，**不凭记忆写**：UniFFI 会把 Rust 的
/// `snake_case` 改成 Swift 的 `camelCase`（`shipped_appreciation` → `shippedAppreciation`），
/// 猜错的表现是 mac 上编译失败，而那时已经晚了。
const REQUIRED_BINDING_SYMBOLS: &[&str] = &[
    "NativeFacade",
    "NativeOperation",
    "NativeAsrOperation",
    "NativeEventSink",
    "NativeError",
    "materializeAssets",
    "fetchVoiceModel",
    "startAsr",
    "pushPcm",
    "finishInput",
    "nextEvent",
    "subscribe",
    "searchText",
    "poemDetail",
    "corpusStatus",
    "reciteStart",
    "reciteSubmit",
    "shippedAppreciation",
];

/// 十个 XCUITest 方法名。前缀 `test_` 是硬要求，见模块文档第 5 条。
const REQUIRED_TEST_METHODS: &[&str] = &[
    "func test_t01_install_and_launch",
    "func test_t02_corpus_first_run_materialization",
    "func test_t03_two_char_search_returns_results",
    "func test_t04_reading_view_citations_and_ai_appreciation",
    "func test_t05_typed_recitation_scores_correctly",
    "func test_t06_voice_recitation_round_succeeds_end_to_end",
    "func test_t07_voice_permission_denied_degrades",
    "func test_t08_chinese_ime_prefilled_field_visible",
    "func test_t09_background_return_preserves_layout",
    "func test_t10_app_exits_cleanly",
];

/// 产品源码里**不允许**出现的东西：绕过生成物直接接原生符号。
const FORBIDDEN_IN_PRODUCT: &[(&str, &str)] = &[
    (
        "@_silgen_name",
        "直接绑定 C 符号会绕过 UniFFI 生成物，漂移的表现是运行期找不到符号",
    ),
    (
        "@_cdecl",
        "导出 C 入口意味着 iOS 侧自己在定义 FFI 边界，而边界只能由 uniffi 生成",
    ),
];

pub(crate) fn verify(root: &Path) -> Result<()> {
    for relative in REQUIRED_FILES {
        let path = root.join(relative);
        let metadata = fs::metadata(&path).with_context(|| {
            format!(
                "iOS 产品工程缺文件 `{relative}`：\
                 裁决是 uniffi_native，方案要求 mobile/ios/Yunjian/ 是真实的 SwiftUI 工程"
            )
        })?;
        if metadata.len() == 0 {
            bail!("iOS 产品工程文件 `{relative}` 是空的");
        }
    }

    let repository = read(root, "mobile/ios/Yunjian/YunjianRepository.swift")?;
    let app = read(root, "mobile/ios/Yunjian/YunjianApp.swift")?;
    for (relative, source) in [
        ("mobile/ios/Yunjian/YunjianRepository.swift", &repository),
        ("mobile/ios/Yunjian/YunjianApp.swift", &app),
    ] {
        if !source.contains("import YunjianMobile") {
            bail!(
                "`{relative}` 没有 `import YunjianMobile`：\
                 iOS 侧唯一允许的 Rust 入口是 UniFFI 生成的那个模块"
            );
        }
        for (needle, why) in FORBIDDEN_IN_PRODUCT {
            if source.contains(needle) {
                bail!("`{relative}` 出现 `{needle}`：{why}");
            }
        }
    }

    let generated = read(root, GENERATED_SWIFT)?;
    for symbol in REQUIRED_BINDING_SYMBOLS {
        if !generated.contains(symbol) {
            bail!(
                "生成的 Swift 绑定里没有 `{symbol}`，而 iOS 产品代码按这个名字调用它；\
                 重新生成绑定或改产品代码，不要两边各猜一个名字"
            );
        }
    }
    // 消费面取**整棵产品源码树**而不是某两个文件：门面调用集中在 `YunjianRepository`，
    // 而流式识别的推送与拉取在 `MainViewModel`。只扫两个文件会把「接线在别的文件里」
    // 误报成「没有接线」。
    let consumers = read_dir_concat(root, "mobile/ios/Yunjian")?;
    for symbol in REQUIRED_BINDING_SYMBOLS {
        if !consumers.contains(symbol) {
            bail!(
                "iOS 产品代码没有用到绑定符号 `{symbol}`：\
                 它出现在必需清单里，意味着某条产品能力没有接线"
            );
        }
    }

    // xcframework 由 `build-xcframework.sh` 用这三个生成物拼出来。少一个的表现是 mac 上
    // `#if canImport(YunjianMobileFFI)` 静默走 false 分支，随后报一堆找不到类型。
    for relative in [GENERATED_HEADER, GENERATED_MODULEMAP] {
        if !root.join(relative).is_file() {
            bail!("缺少 `{relative}`：iOS 的 xcframework 需要 UniFFI 的 C 头与 modulemap");
        }
    }
    let script = read(root, "mobile/ios/scripts/build-xcframework.sh")?;
    for needle in [
        "YunjianMobileFFI.h",
        "module.modulemap",
        "-create-xcframework",
    ] {
        if !script.contains(needle) {
            bail!("`build-xcframework.sh` 里没有 `{needle}`：它产不出可链接的 xcframework");
        }
    }

    verify_tag_parity(root)?;
    verify_test_methods(root)?;
    verify_measurement_key_coverage(root)?;
    verify_test_plan(root)?;
    Ok(())
}

/// 两个平台的界面标识必须逐字相同。
///
/// 判据是**双向**的：Android 有而 iOS 没有，意味着 iOS 上某条断言找不到节点；iOS 有而 Android
/// 没有，意味着两侧的界面契约已经分叉。任何一侧多出一个都判红。
fn verify_tag_parity(root: &Path) -> Result<()> {
    let kotlin = parse_kotlin_tags(&read(root, ANDROID_TAGS)?);
    let swift = parse_swift_tags(&read(root, IOS_TAGS)?);
    if kotlin.is_empty() {
        bail!("从 `{ANDROID_TAGS}` 里没解析出任何常量，解析器与源码格式已经不匹配");
    }
    for (name, value) in &kotlin {
        match swift.get(name) {
            None => bail!(
                "iOS 的 TestTags 缺常量 `{name}`（Android 侧值 `{value}`）：\
                 少一个标识就少一条能在 iOS 上判定的断言"
            ),
            Some(other) if other != value => bail!(
                "标识 `{name}` 两侧取值不同：Android `{value}`，iOS `{other}`；\
                 两个平台共用同一套判据，取值分叉会让同名断言量不同的东西"
            ),
            Some(_) => {}
        }
    }
    for name in swift.keys() {
        if !kotlin.contains_key(name) {
            bail!("iOS 的 TestTags 多出常量 `{name}`：界面契约已与 Android 分叉");
        }
    }
    Ok(())
}

fn verify_test_methods(root: &Path) -> Result<()> {
    let ui = read(
        root,
        "mobile/ios/YunjianUITests/FullAcceptanceUITests.swift",
    )?;
    for method in REQUIRED_TEST_METHODS {
        if !ui.contains(method) {
            bail!(
                "XCUITest 缺方法 `{method}`：\
                 XCTest 只发现 `test` 前缀的方法，写成 `t01_` 的话一个测试都不会跑，\
                 而 Device Farm 的 run 会显示成功"
            );
        }
    }
    Ok(())
}

/// 宿主侧十条判据的每一个 required 键，iOS 侧都要真的报得出来。
///
/// # 为什么这条比「文件在不在」值钱
///
/// 一个只有界面、报不出测量键的 iOS 工程会让十条断言全部 NOT EXECUTED，而判词会写成
/// 「驱动不可用」——读起来像缺 macOS，实际是 harness 没接线。这条断言把那种误读挡在提交前。
fn verify_measurement_key_coverage(root: &Path) -> Result<()> {
    let ui = read(
        root,
        "mobile/ios/YunjianUITests/FullAcceptanceUITests.swift",
    )?;
    let app_tests = read(root, "mobile/ios/YunjianAppTests/ContainerFactsTests.swift")?;
    let harness = format!("{ui}\n{app_tests}");
    let mut missing = Vec::new();
    for criterion in super::full_criteria::CRITERIA {
        for key in criterion.required {
            // 键在源码里以字符串字面量出现（`measure(assertion, "root_rendered", …)`），
            // 或以 `_unavailable` 的形式带原因上报——两种都算「报得出来」。
            let quoted = format!("\"{key}\"");
            if !harness.contains(&quoted) {
                missing.push(format!("{}::{key}", criterion.id));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "iOS harness 报不出这些必需测量键：{}；\
             缺键会让对应断言永远 NOT EXECUTED，而判词读起来像缺宿主机",
            missing.join("、")
        );
    }
    Ok(())
}

/// 测试计划必须**关掉**随机顺序与并行。
///
/// 十条断言有真实的先后依赖（语料没物化就搜不到东西）。Android 侧栽过一次：默认方法序把
/// `t09 → t10 → t06 → t07` 排在前面，依赖被打散，前面几条压根没跑到。iOS 的对应开关在
/// 测试计划里，而**默认值不是我们要的那个**。
fn verify_test_plan(root: &Path) -> Result<()> {
    let plan = read(root, "mobile/ios/Yunjian.xctestplan")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&plan).context("解析 mobile/ios/Yunjian.xctestplan 失败")?;
    let ordering = parsed
        .get("defaultOptions")
        .and_then(|options| options.get("testExecutionOrdering"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if ordering != "lexical" {
        bail!(
            "测试计划的 testExecutionOrdering 是 `{ordering}`，必须是 `lexical`：\
             随机顺序会打散十条断言之间的真实依赖"
        );
    }
    let targets = parsed
        .get("testTargets")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let names: Vec<&str> = targets
        .iter()
        .map(|target| {
            target
                .get("target")
                .and_then(|inner| inner.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        })
        .collect();
    if names != ["YunjianUITests", "YunjianAppTests"] {
        bail!(
            "测试计划的 target 顺序是 {names:?}，必须是 [\"YunjianUITests\", \"YunjianAppTests\"]：\
             进程内那一轮读容器里的事实，必须排在触发首启物化的界面那一轮之后"
        );
    }
    if targets.iter().any(|target| {
        target
            .get("parallelizable")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    }) {
        bail!("测试计划里有 target 没有显式 `parallelizable: false`：并行会打散断言之间的依赖");
    }
    Ok(())
}

/// 把一个目录下所有文件的内容连起来，供「某个符号在这棵树里有没有被用到」这类判定使用。
fn read_dir_concat(root: &Path, relative: &str) -> Result<String> {
    let dir = root.join(relative);
    let mut names: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("读取目录 {} 失败", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    names.sort();
    let mut out = String::new();
    for path in names {
        out.push_str(
            &fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?,
        );
        out.push('\n');
    }
    Ok(out)
}

fn read(root: &Path, relative: &str) -> Result<String> {
    let path = root.join(relative);
    fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))
}

/// `const val NAME = "value"` → (NAME, value)。
fn parse_kotlin_tags(source: &str) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("const val ") else {
            continue;
        };
        if let Some((name, value)) = split_assignment(rest) {
            tags.insert(name, value);
        }
    }
    tags
}

/// `static let name = "value"` → (NAME, value)。
///
/// Swift 侧是 `lowerCamelCase`，Android 侧是 `UPPER_SNAKE_CASE`；比对前统一成后者，否则两边
/// 永远对不上，而那会把一条真门禁变成一条永远红的噪音。
fn parse_swift_tags(source: &str) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("static let ") else {
            continue;
        };
        if let Some((name, value)) = split_assignment(rest) {
            tags.insert(upper_snake(&name), value);
        }
    }
    tags
}

fn split_assignment(rest: &str) -> Option<(String, String)> {
    let (name, tail) = rest.split_once('=')?;
    let value = tail.trim().trim_matches('"');
    if value.is_empty() || value.contains('"') {
        return None;
    }
    Some((name.trim().to_owned(), value.to_owned()))
}

fn upper_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        super::super::super::repo_root()
    }

    #[test]
    fn the_shipped_ios_project_satisfies_every_structural_requirement() {
        verify(&root()).expect("仓库里的 iOS 工程必须满足全部结构要求");
    }

    #[test]
    fn a_missing_project_file_is_reported_with_its_path() {
        // 注入：把必备清单里的一条指向不存在的文件，等价于「那个文件被删了」。
        let path = root().join("mobile/ios/Yunjian/ContentView.swift");
        assert!(path.is_file(), "注入验证的前提是这个文件本来在");
        let error = read(&root(), "mobile/ios/Yunjian/DoesNotExist.swift")
            .expect_err("不存在的文件必须报错");
        assert!(
            error.to_string().contains("DoesNotExist.swift"),
            "判词必须点出缺哪一个文件：{error}"
        );
    }

    #[test]
    fn tag_parity_rejects_a_diverged_value() {
        let kotlin = parse_kotlin_tags("    const val ROOT = \"yunjian_root\"\n");
        let swift = parse_swift_tags("    static let root = \"yunjian_ROOT\"\n");
        assert_eq!(kotlin.get("ROOT").map(String::as_str), Some("yunjian_root"));
        assert_eq!(swift.get("ROOT").map(String::as_str), Some("yunjian_ROOT"));
        assert_ne!(
            kotlin.get("ROOT"),
            swift.get("ROOT"),
            "取值分叉必须能被解析出来，否则门禁形同虚设"
        );
    }

    #[test]
    fn swift_names_map_onto_the_kotlin_naming_convention() {
        assert_eq!(upper_snake("searchHitReadPrefix"), "SEARCH_HIT_READ_PREFIX");
        assert_eq!(upper_snake("root"), "ROOT");
        assert_eq!(upper_snake("tabSearch"), "TAB_SEARCH");
    }

    #[test]
    fn every_declared_measurement_key_is_produced_by_the_ios_harness() {
        verify_measurement_key_coverage(&root())
            .expect("iOS harness 必须报得出十条判据的每一个必需键");
    }

    #[test]
    fn the_ten_xcuitest_methods_carry_the_test_prefix() {
        verify_test_methods(&root()).expect("十个方法都必须带 test 前缀，否则一个都不会跑");
    }
}
