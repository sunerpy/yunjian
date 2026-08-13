//! 线上形状与源码守卫。**这一半在三个平台都跑，包括 Windows。**
//!
//! # 为什么与 `tests.rs` 分成两个文件
//!
//! `tests.rs` 里的用例要在 `MockRuntime` 上构造真 `tauri::App`，而那在 Windows CI 上会让
//! 整个 lib 测试二进制以 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND` 在**加载期**失败——一条
//! 测试都还没跑。同一份代码在 Linux 上全过，且 `main` 上的 Windows 作业是绿的（那时
//! `yunjian-app` 的 lib 测试里没有任何一条构造 `App`）。所以那是 Windows 加载器与 tauri 的
//! `test` 特性之间的事，不是本模块的逻辑问题；本环境没有 Windows 宿主机可供定位。
//!
//! **处置是分文件而不是 `#[ignore]`**：`ignore` 仍然会把那段代码链进二进制，而失败发生在
//! 加载期，只有整段不编译才躲得过。于是 `tests.rs` 挂 `cfg(not(windows))`，本文件无条件编译。
//!
//! 落到覆盖上：**线上形状这一层三平台都有守**——原因码、进度阶段、命令必须 async、不得返回
//! `Vec<u8>`、不得用 event/eval、不得阻塞运行时、标记时基。需要 Windows 侧真的验到 `App`
//! 驱动的那一半时，正确的层是 todo 67 的真机验收：它本来就要在交互式桌面会话里拉起真
//! WebView，那一层能观察到加载器行为，而 headless CI 观察不到。

use serde_json::Value;

use super::{MARK_TIMEBASE_HZ, ModelFetchOut, WIRE_DEGRADE_REASONS, degrade_reason_key};

// ---------------------------------------------------------------------------
// 源码守卫
// ---------------------------------------------------------------------------

fn production_source() -> String {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/voice_ipc.rs"),
    )
    .expect("读取语音 IPC 源码");
    source
        .split("#[cfg(test)]\nmod ")
        .next()
        .expect("存在生产源码")
        .to_owned()
}

const VOICE_COMMANDS: &[&str] = &[
    "voice_availability",
    "voice_demonstrate",
    "voice_start_session",
    "voice_fetch_model",
];

/// 四条语音命令都必须是 `async`。同步命令的函数体在 WebView 主线程上跑，采集与识别放进去
/// 就是一次窗口冻结，而那种冻结在开发机上看不见——数据小、机器快。
#[test]
fn every_voice_command_is_async() {
    let source = production_source();
    for command in VOICE_COMMANDS {
        assert!(
            source.contains(&format!("async fn {command}")),
            "语音命令 `{command}` 必须是 async"
        );
    }
}

/// 命令不得返回 `Vec<u8>`：实测 6.3 MB 的音频当返回值序列化会在线上膨胀到 22.5 MB。
#[test]
fn no_voice_command_returns_audio_as_a_byte_vector() {
    let source = production_source();
    assert!(
        !source.lines().any(|line| {
            line.contains("async fn") && (line.contains("Vec<u8>") || line.contains("Vec < u8 >"))
        }),
        "命令不得返回 Vec<u8>；音频必须通过自定义 URI 协议读取"
    );
}

/// async 命令体内不得出现阻塞式睡眠。
///
/// `std::thread::sleep` 会占住一个运行时 worker 而不是把它让出去。多线程运行时下这不会
/// 立刻表现为界面冻结（[`a_running_session_does_not_serialize_other_commands`] 的文档记了
/// 这次实测），但它会在运行时线程数被占满时突然变成一次真的冻结，而那时症状与原因隔得
/// 很远。**这条守卫已验证会为该缺陷变红。**
#[test]
fn async_command_bodies_never_block_the_runtime() {
    let source = production_source();
    assert!(
        !source.contains("std::thread::sleep"),
        "async 命令体内不得阻塞运行时；让出线程请用 tokio::time::sleep"
    );
}

/// `spawn_blocking` 的闭包里不得 await，也不得构造嵌套运行时。
///
/// 前者会把长任务钉在阻塞线程池上并让 drop 取消失效，后者会造出第二个运行时。
#[test]
fn blocking_workers_neither_await_nor_nest_a_runtime() {
    let source = production_source();
    for body in source.split("blocking(").skip(1) {
        let closure = body.split(".await").next().expect("存在闭包体");
        assert!(
            !closure.contains("Runtime::new") && !closure.contains("Builder::new_"),
            "阻塞闭包不得构造嵌套 async runtime"
        );
    }
}

/// Windows 上的排除必须是**声明过的**，而不是一句 `cfg` 了事。
///
/// 一处没有记录的平台排除等于一处静默缩小的门禁：谁都看不出来 Windows 上少跑了十一条。
/// 这条断言把「排除存在」与「排除有理由、有后续覆盖层」钉在一起——删掉声明或改成
/// `#[ignore]`（挡不住加载期）都会让它变红。
#[test]
fn the_windows_exclusion_is_declared_with_its_reason_and_follow_up() {
    let declaration = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/voice_ipc.rs"),
    )
    .expect("读取语音 IPC 源码");
    assert!(
        declaration.contains("#[cfg(all(test, not(windows)))]\nmod tests;"),
        "App 驱动的那一半必须整段不编译于 Windows；`#[ignore]` 不行，失败在加载期"
    );

    let reason = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/voice_ipc/wire_tests.rs"),
    )
    .expect("读取本文件");
    assert!(
        reason.contains("STATUS_ENTRYPOINT_NOT_FOUND"),
        "排除理由必须写出实测到的失败码，而不是一句「Windows 上有问题」"
    );
    assert!(
        reason.contains("todo 67"),
        "排除必须指明由哪一层补上，否则它就是一处永久缺口"
    );
}

/// 十条降级原因各有一个互不相同的线上串。
#[test]
fn every_degrade_reason_has_a_distinct_wire_key() {
    let mut keys: Vec<&str> = WIRE_DEGRADE_REASONS
        .iter()
        .copied()
        .map(degrade_reason_key)
        .collect();
    keys.sort_unstable();
    let count = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), count, "原因码的线上串必须互不相同");
}

/// 标记时基是毫秒。写成断言而不是只写在注释里：换算比例错了会让高亮整体拉偏。
#[test]
fn marks_use_a_millisecond_timebase() {
    assert_eq!(MARK_TIMEBASE_HZ, 1_000);
}

/// 流式路径不得改用 Tauri event 或 `eval`。
#[test]
fn voice_streaming_never_uses_events_or_eval() {
    let production = production_source();
    assert!(
        production.contains("ipc::Channel"),
        "流式数据必须通过 ipc::Channel"
    );
    assert!(
        !production.contains(".emit("),
        "流式路径不得使用 Tauri event"
    );
    assert!(!production.contains(".eval("), "不得通过 eval 传输数据");
}

/// 四段进度的线上串与模型层的枚举逐项对应。
///
/// 与上一条分工明确：上一条验「进度到得了界面」（受可合并协议约束，只能验存在性），
/// 这一条验「映射没写错」（纯函数，可逐项断言）。合成一条会让其中一半失去判据。
#[test]
fn every_fetch_stage_maps_onto_a_distinct_wire_string() {
    let mapped = [
        (
            yunjian_voice::models::FetchProgress::Downloading {
                bytes_done: 7,
                bytes_total: 9,
            },
            "downloading",
        ),
        (
            yunjian_voice::models::FetchProgress::Verifying { bytes: 9 },
            "verifying",
        ),
        (yunjian_voice::models::FetchProgress::Verified, "verified"),
        (yunjian_voice::models::FetchProgress::Unpacking, "unpacking"),
    ];
    for (source, expected) in mapped {
        let value = serde_json::to_value(ModelFetchOut::from(source)).expect("可序列化");
        assert_eq!(value["stage"], Value::from(expected));
    }
    let downloading = serde_json::to_value(ModelFetchOut::from(
        yunjian_voice::models::FetchProgress::Downloading {
            bytes_done: 7,
            bytes_total: 9,
        },
    ))
    .expect("可序列化");
    assert_eq!(downloading["bytes_done"], Value::from(7));
    assert_eq!(downloading["bytes_total"], Value::from(9));
}
