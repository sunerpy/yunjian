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

const VOICE_CAPTURE: &str = "mobile/ios/Yunjian/VoiceCapture.swift";
const ANDROID_VIEW_MODEL: &str =
    "mobile/android/app/src/main/java/top/onethinker/yunjian/MainViewModel.kt";

/// 停用 session 的调用与它必须带的选项。**逐字取自 Apple 文档**，不凭记忆写：
/// `func setActive(_ active: Bool, options: AVAudioSession.SetActiveOptions = []) throws`，
/// 选项 `notifyOthersOnDeactivation` 且文档明载它只在停用时有效。
const SESSION_DEACTIVATION_CALL: &str = "setActive(false";
const SESSION_RESUME_OPTION: &str = ".notifyOthersOnDeactivation";
const SESSION_ACTIVATION_HELPER: &str = "activateRecordingSession";
const SESSION_DEACTIVATION_HELPER: &str = "deactivateRecordingSession";
const FRAME_QUEUE_TYPE: &str = "final class FrameQueue";

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
    verify_voice_capture_contract(root)?;
    Ok(())
}

/// `mobile/ios/Yunjian/VoiceCapture.swift` 的两条运行期契约。
///
/// # 为什么这两条值得一条会红的断言
///
/// 两者都是 F2 第三轮在**没有 Xcode** 的前提下读出来的真实运行风险：编译能过、静态门禁全绿，
/// 而真机上表现为「录完一轮之后别的应用没声了」和「说了三秒只录到一秒」。这类缺陷不会被任何
/// 现有断言碰到，只能靠人读代码发现——除非把它变成文本层可判定的东西。
///
/// 1. **`AVAudioSession` 必须在所有出口停用。** 只 `engine.stop()` 不 `setActive(false)`，
///    已激活的录音 session 会继续占用系统音频策略；后果落在别的应用上，所以本应用自己的
///    验收永远量不到它。
/// 2. **一轮的时长必须由采样率换算，不能用「回调次数 × 假定帧长」近似。** iOS 的 tap 按
///    **设备**采样率交付，`bufferSize` 还只是建议值；在常见的 44.1/48 kHz 上「30 帧」是
///    约 1 秒而不是 3 秒。重采样只改采样率不改源时长，补不回缺掉的两秒。
fn verify_voice_capture_contract(root: &Path) -> Result<()> {
    let source = read(root, VOICE_CAPTURE)?;
    // 判据一律建立在**剔掉注释之后**的代码上。这个文件的头部散文逐字写着
    // `setActive(false)`、`notifyOthersOnDeactivation` 与两个辅助函数名（它们在解释这条
    // 契约），所以只判原文时把调用整行删掉仍然全绿——本仓库已经栽过五次
    // 「解释一条规则的文字命中这条规则」。
    let swift = strip_swift_line_comments(&source);
    verify_session_is_deactivated(&swift)?;
    verify_round_duration_is_derived(&swift)?;
    verify_round_duration_parity(root, &swift)?;
    Ok(())
}

/// 去掉 Swift 的行注释（含 `///` 文档注释），但不动字符串字面量里的 `//`。
///
/// 与 `xtask/tests/workspace_contract.rs` 的 `strip_line_comments` 同形态。**没有共用一份
/// 实现**是因为那份住在集成测试 target 里，而这里是 `xtask` 的 bin target——xtask 没有 lib
/// target，两者无法互相 `use`。共用需要先给 xtask 加 lib，那是另一件事。两份各自带一条
/// 「散文不得冒充调用」的反例测试，防止其中任何一份退化。
fn strip_swift_line_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let bytes = line.as_bytes();
        let mut cut = line.len();
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                cut = index;
                break;
            }
            index += 1;
        }
        // `cut` 只落在 ASCII 的 `/` 上或行尾，因此这个切片不会切断多字节字符。
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// 每一次激活都要配一次停用，而且停用要写在 `defer` 里。
///
/// 判据是**配对计数**而不是「文件里有没有出现 setActive(false)」：后者在「加了第二条采集
/// 路径但忘了给它停用」时依然为真，而那正是这类泄漏最常见的引入方式。
fn verify_session_is_deactivated(swift: &str) -> Result<()> {
    if !swift.contains(SESSION_DEACTIVATION_CALL) {
        bail!(
            "`{VOICE_CAPTURE}` 里没有 `{SESSION_DEACTIVATION_CALL}`：\
             只 `engine.stop()` 不停用 AVAudioSession，已激活的录音 session 会继续占用\
             系统音频策略，后果落在别的应用上——本应用自己的验收永远量不到它"
        );
    }
    if !swift.contains(SESSION_RESUME_OPTION) {
        bail!(
            "`{VOICE_CAPTURE}` 停用 session 时没有带 `{SESSION_RESUME_OPTION}`：\
             不带这个选项时被打断的其他应用不会收到可以恢复的通知，表现为「录完一轮之后\
             别的应用没声了」"
        );
    }

    let activations = call_sites(swift, SESSION_ACTIVATION_HELPER);
    let deactivations = deferred_call_sites(swift, SESSION_DEACTIVATION_HELPER);
    if activations == 0 {
        bail!(
            "`{VOICE_CAPTURE}` 里找不到 `{SESSION_ACTIVATION_HELPER}()` 的调用点：\
             这条断言靠「激活次数 == 停用次数」配对，找不到激活点说明它已经量不到东西了"
        );
    }
    if activations != deactivations {
        bail!(
            "`{VOICE_CAPTURE}` 激活 session {activations} 次，但只有 {deactivations} 处 \
             `defer {{ {SESSION_DEACTIVATION_HELPER}() }}`。每一次激活都要配一次停用，\
             而且必须写在 `defer` 里——写在函数末尾的话，中途 `return` 的降级路径\
             （无输入设备、engine 启动失败）会把 session 留在激活状态"
        );
    }
    Ok(())
}

/// 一轮的帧数必须由目标时长与采样率换算出来，不能是字面量。
fn verify_round_duration_is_derived(swift: &str) -> Result<()> {
    let declaration = declaration_line(swift, "framesPerRound")
        .ok_or_else(|| anyhow::anyhow!("`{VOICE_CAPTURE}` 里找不到 `framesPerRound` 的声明"))?;
    for token in ["roundSeconds", "sampleRate", "frameSamples"] {
        if !declaration.contains(token) {
            bail!(
                "`{VOICE_CAPTURE}` 的 `framesPerRound` 声明里没有 `{token}`：\
                 它必须由「目标时长 × 采样率 ÷ 每帧采样数」算出来。写成字面量时\
                 「30 帧 ≈ 3 秒」只在 16 kHz、每帧 1600 采样这一种组合下成立，\
                 而 iOS 的 tap 按**设备**采样率交付，改任一个常量都会静默改掉一轮的真实时长。\
                 实际声明：`{declaration}`"
            );
        }
    }
    if declaration_line(swift, "silenceProbeFrames").is_some() {
        bail!(
            "`{VOICE_CAPTURE}` 还留着 `silenceProbeFrames`：静音探测的长度也要按时长记\
             （`silenceProbeSeconds`），按回调次数记会随设备采样率漂移，\
             而名字里的「3 帧 ≈ 300 ms」在 48 kHz 上是约 100 ms"
        );
    }
    if declaration_line(swift, "silenceProbeSeconds").is_none() {
        bail!("`{VOICE_CAPTURE}` 里找不到 `silenceProbeSeconds`：静音探测长度必须按时长声明");
    }

    // 帧长必须由 16 kHz 目标帧长决定，而不是由 tap 交付的缓冲长度决定——这是「30 帧只有
    // 1 秒」的真正根因：tap 按设备采样率交付，直接把回调缓冲当成一帧就等于把帧长交给设备。
    let queue = type_body(swift, FRAME_QUEUE_TYPE)
        .ok_or_else(|| anyhow::anyhow!("`{VOICE_CAPTURE}` 里找不到 `{FRAME_QUEUE_TYPE}` 的定义"))?;
    if !queue.contains("frameSamples") {
        bail!(
            "`{VOICE_CAPTURE}` 的 `{FRAME_QUEUE_TYPE}` 不认识 `frameSamples`，\
             说明它按 tap 交付的缓冲长度往外给帧。tap 按**设备**采样率交付且 `bufferSize` \
             只是建议值，那样一帧的时长由设备决定：48 kHz 下 1600 采样是 33 ms 而不是 \
             100 ms，一轮 30 帧就只有约 1 秒。队列必须自己按目标帧长切分重采样后的流"
        );
    }
    Ok(())
}

/// 两个平台一轮采集的**时长**必须相同，各自从自己的常量算出来。
///
/// 这条是 `verify_tag_parity` 的同形态：十条判据两侧共用，而「一轮多长」直接决定
/// `total_ms` 与「是否开口／停顿」的读数。两侧算出不同的毫秒数时，同名断言在量不同的东西。
fn verify_round_duration_parity(root: &Path, swift: &str) -> Result<()> {
    let kotlin = read(root, ANDROID_VIEW_MODEL)?;
    let android_rate = kotlin_int(&kotlin, "SAMPLE_RATE")?;
    let android_frame = kotlin_int(&kotlin, "FRAME_SAMPLES")?;
    let android_frames_per_round = kotlin_int(&kotlin, "FRAMES_PER_ROUND")?;
    let ios_rate = swift_number(swift, "sampleRate")?;
    let ios_frame = swift_number(swift, "frameSamples")?;
    let ios_round_seconds = swift_number(swift, "roundSeconds")?;

    if (ios_rate - f64::from(android_rate)).abs() > f64::EPSILON {
        bail!(
            "采样率两侧不同：Android {android_rate}，iOS {ios_rate}；\
             宿主侧按同一个采样率解读 `total_ms`"
        );
    }
    if (ios_frame - f64::from(android_frame)).abs() > f64::EPSILON {
        bail!("每帧采样数两侧不同：Android {android_frame}，iOS {ios_frame}");
    }

    let android_ms = f64::from(android_frames_per_round) * f64::from(android_frame) * 1000.0
        / f64::from(android_rate);
    let ios_ms = ios_round_seconds * 1000.0;
    if (android_ms - ios_ms).abs() > 1.0 {
        bail!(
            "一轮采集的时长两侧不同：Android {android_ms} ms\
             （{android_frames_per_round} 帧 × {android_frame} 采样 ÷ {android_rate} Hz），\
             iOS {ios_ms} ms（roundSeconds = {ios_round_seconds}）。\
             十条判据两侧共用，一轮多长直接决定 `total_ms` 与「是否开口／停顿」的读数"
        );
    }
    Ok(())
}

/// 某个标识符被**调用**的次数（不含它自己的定义行）。
fn call_sites(source: &str, name: &str) -> usize {
    source
        .lines()
        .filter(|line| !line.contains("func ") && calls(line, name))
        .count()
}

/// 某个标识符出现在 `defer` 里的次数。
fn deferred_call_sites(source: &str, name: &str) -> usize {
    source
        .lines()
        .filter(|line| line.contains("defer") && calls(line, name))
        .count()
}

/// 这一行有没有调用 `name()`，**按标识符边界判**。
///
/// 边界判定不是讲究：`deactivateRecordingSession()` 的字面里含有
/// `activateRecordingSession()`，用朴素 `contains` 数激活点会把每一处停用也数成一次激活，
/// 于是「激活次数 == 停用次数」在真的配平时反而失败。实测踩到过这一条。
fn calls(line: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut rest = line;
    while let Some(at) = rest.find(&needle) {
        let preceded_by_identifier = rest[..at]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        if !preceded_by_identifier {
            return true;
        }
        rest = &rest[at + needle.len()..];
    }
    false
}

/// `static let name...` / `static var name...` 的整行声明。
fn declaration_line<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    source.lines().map(str::trim).find(|line| {
        (line.starts_with("static let ") || line.starts_with("static var "))
            && line
                .split_whitespace()
                .nth(2)
                .is_some_and(|token| token.trim_end_matches(':') == name)
    })
}

/// 一个 Swift 类型定义的正文（从声明行到与它同缩进的收尾花括号）。
fn type_body<'a>(source: &'a str, declaration: &str) -> Option<&'a str> {
    let start = source.find(declaration)?;
    let rest = &source[start..];
    let mut depth = 0usize;
    let mut entered = false;
    for (offset, ch) in rest.char_indices() {
        match ch {
            '{' => {
                depth += 1;
                entered = true;
            }
            '}' => {
                depth -= 1;
                if entered && depth == 0 {
                    return Some(&rest[..=offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn kotlin_int(source: &str, name: &str) -> Result<u32> {
    let literal = source
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix("const val ")?
                .strip_prefix(name)?
                .trim_start()
                .strip_prefix('=')
                .map(str::trim)
        })
        .with_context(|| format!("`{ANDROID_VIEW_MODEL}` 里找不到 `const val {name}`"))?;
    parse_number(literal)
        .and_then(|value| u32::try_from(value as i64).ok())
        .with_context(|| format!("`{name}` 的取值 `{literal}` 不是整数字面量"))
}

fn swift_number(source: &str, name: &str) -> Result<f64> {
    let declaration = declaration_line(source, name)
        .with_context(|| format!("`{VOICE_CAPTURE}` 里找不到 `{name}` 的声明"))?;
    let literal = declaration
        .split_once('=')
        .map(|(_, tail)| tail.trim())
        .with_context(|| format!("`{name}` 的声明 `{declaration}` 没有取值"))?;
    parse_number(literal).with_context(|| format!("`{name}` 的取值 `{literal}` 不是数字字面量"))
}

/// `16_000` / `1_600` / `3` / `0.3` → f64。分隔下划线是两种语言共有的写法。
fn parse_number(literal: &str) -> Option<f64> {
    literal
        .trim_end_matches([',', ';'])
        .replace('_', "")
        .parse()
        .ok()
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

    #[test]
    fn the_shipped_voice_capture_closes_both_runtime_contracts() {
        verify_voice_capture_contract(&root())
            .expect("VoiceCapture 必须停用 session，并按采样率换算一轮时长");
    }

    /// 注入：删掉停用调用。这是 2026-08-18 之前的实际状态（F2 第三轮读出来的那一条）。
    #[test]
    fn dropping_the_session_deactivation_is_reported() {
        let shipped = strip_swift_line_comments(
            &read(&root(), VOICE_CAPTURE).expect("读取 VoiceCapture.swift"),
        );
        // 连辅助函数的函数体一起删掉，才是 F2 之前那个「文件里根本没有 setActive(false)」
        // 的状态；只删调用点会留着函数体，那时先命中的是配对计数那条判词。
        let without: String = shipped
            .lines()
            .filter(|line| {
                !calls(line, SESSION_DEACTIVATION_HELPER)
                    && !line.contains(SESSION_DEACTIVATION_CALL)
            })
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(
            without.contains(SESSION_ACTIVATION_HELPER),
            "注入的前提是激活调用还在，否则这条反例证明不了停用被漏掉"
        );
        let error =
            verify_session_is_deactivated(&without).expect_err("漏掉 setActive(false) 必须判红");
        assert!(
            error.to_string().contains("setActive(false"),
            "判词必须点出缺的是停用：{error}"
        );
    }

    /// 注入：停用还在，但从 `defer` 里挪到函数末尾。降级路径的提前 `return` 会绕过它。
    #[test]
    fn moving_the_deactivation_out_of_defer_is_reported() {
        let shipped = strip_swift_line_comments(
            &read(&root(), VOICE_CAPTURE).expect("读取 VoiceCapture.swift"),
        );
        let moved = shipped.replace(
            &format!("defer {{ {SESSION_DEACTIVATION_HELPER}() }}"),
            &format!("{SESSION_DEACTIVATION_HELPER}()"),
        );
        assert_ne!(moved, shipped, "注入没有真的改到源码");
        assert!(
            moved.contains(SESSION_DEACTIVATION_CALL),
            "前提：这种漂移下「文件里有没有 setActive(false)」判据依然为真，\
             所以它证明了配对计数比存在性检查更强"
        );
        let error = verify_session_is_deactivated(&moved).expect_err("停用不在 defer 里必须判红");
        assert!(
            error.to_string().contains("defer"),
            "判词必须点出它得写在 defer 里：{error}"
        );
    }

    /// 注入：把一轮帧数改回字面量 30。这正是「30 帧 ≈ 3 秒」那条错近似。
    #[test]
    fn a_literal_frame_count_per_round_is_reported() {
        let shipped = strip_swift_line_comments(
            &read(&root(), VOICE_CAPTURE).expect("读取 VoiceCapture.swift"),
        );
        let literal: String = shipped
            .lines()
            .map(|line| {
                if line.trim().starts_with("static var framesPerRound") {
                    "    static let framesPerRound: Int = 30\n".to_owned()
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        assert_ne!(literal, shipped, "注入没有真的改到 framesPerRound 的声明");
        let error =
            verify_round_duration_is_derived(&literal).expect_err("一轮帧数写成字面量必须判红");
        let message = error.to_string();
        assert!(
            message.contains("framesPerRound") && message.contains("30 帧"),
            "判词必须点出它为什么不成立：{message}"
        );
    }

    /// 注入：让队列不再按目标帧长切分，即把帧长交回给设备采样率。
    #[test]
    fn a_frame_queue_that_does_not_resplit_is_reported() {
        let shipped = strip_swift_line_comments(
            &read(&root(), VOICE_CAPTURE).expect("读取 VoiceCapture.swift"),
        );
        let body = type_body(&shipped, FRAME_QUEUE_TYPE).expect("仓库里必须有 FrameQueue");
        assert!(
            body.contains("frameSamples"),
            "前提：现行 FrameQueue 确实按目标帧长切分"
        );
        let unsplit = shipped.replace(body, "final class FrameQueue { }");
        let error =
            verify_round_duration_is_derived(&unsplit).expect_err("队列不按目标帧长切分必须判红");
        assert!(
            error
                .to_string()
                .contains(FRAME_QUEUE_TYPE.trim_start_matches("final class ")),
            "判词必须点名 FrameQueue：{error}"
        );
    }

    /// 注入：让 iOS 的一轮时长与 Android 分叉。
    #[test]
    fn a_round_duration_that_diverges_from_android_is_reported() {
        let shipped = strip_swift_line_comments(
            &read(&root(), VOICE_CAPTURE).expect("读取 VoiceCapture.swift"),
        );
        let diverged = shipped.replace(
            "static let roundSeconds: Double = 3",
            "static let roundSeconds: Double = 1",
        );
        assert_ne!(diverged, shipped, "注入没有真的改到 roundSeconds");
        let error =
            verify_round_duration_parity(&root(), &diverged).expect_err("两侧一轮时长不同必须判红");
        assert!(
            error.to_string().contains("3000") && error.to_string().contains("1000"),
            "判词必须把两侧的毫秒数都报出来：{error}"
        );
    }

    /// 散文不得冒充调用——与 `workspace_contract.rs` 那条同一个理由。
    #[test]
    fn the_swift_comment_stripper_does_not_let_prose_impersonate_a_call() {
        let prose_only = "/// 结束时必须 setActive(false, options: [.notifyOthersOnDeactivation])。\n\
                          enum VoiceCapture {}\n";
        assert!(
            prose_only.contains(SESSION_DEACTIVATION_CALL),
            "前提：注释原文确实命中标记，否则这条反例没有意义"
        );
        let stripped = strip_swift_line_comments(prose_only);
        assert!(
            !stripped.contains(SESSION_DEACTIVATION_CALL),
            "剔注释之后散文不得再命中标记，否则调用被删掉时断言仍会通过：{stripped}"
        );
        assert!(
            verify_session_is_deactivated(&stripped).is_err(),
            "只剩散文时必须判红"
        );

        let with_url = "let a = \"https://example.invalid/x\" // 说明\n";
        let stripped = strip_swift_line_comments(with_url);
        assert!(
            stripped.contains("https://example.invalid/x"),
            "字符串里的 `//` 被误判成注释：{stripped}"
        );
        assert!(!stripped.contains("说明"), "行尾注释未被剔除：{stripped}");
    }

    /// 标识符边界：`deactivateRecordingSession()` 不得被数成一次 `activateRecordingSession()`。
    #[test]
    fn a_longer_identifier_is_not_counted_as_a_call_to_its_suffix() {
        let line = "        defer { deactivateRecordingSession() }";
        assert!(calls(line, SESSION_DEACTIVATION_HELPER));
        assert!(
            !calls(line, SESSION_ACTIVATION_HELPER),
            "朴素 contains 会把停用数成激活，于是配平的代码反而判红——实测踩过"
        );
        assert!(calls(
            "        try activateRecordingSession()",
            SESSION_ACTIVATION_HELPER
        ));
    }
}
