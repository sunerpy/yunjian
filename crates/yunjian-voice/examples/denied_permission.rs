//! 失败场景 2 的实跑：**权限被拒时应用降级到打字练习并给出解释性消息**，而不是崩溃、
//! 也不是静默录不到声音。
//!
//! 「静默录不到」是这里真正要排除的失败模式，也是最难发现的一种：一个只产出零的通路
//! 能通过所有格式断言。因此本文件既断言权限路径，也断言采集路径的失败会被翻译成同一
//! 套降级判定。
//!
//! ```text
//! cargo run -p yunjian-voice --features capture --example denied_permission
//! ```

#![expect(
    clippy::print_stderr,
    reason = "手动探针的产物就是给人看的诊断输出；工作区把 print_stderr 设成 warn 是为了把\
              日志逼进 tracing，而这个 example 不是日志、也不进任何发布产物"
)]

use std::time::Duration;

use yunjian_voice::permission::{self, DegradeReason, MicPermission, PermissionState, Practice};
use yunjian_voice::platform::Platform;

fn main() {
    let mut failures = Vec::new();

    eprintln!("=== 场景 A：五个平台上权限被明确拒绝 ===");
    for platform in Platform::ALL {
        let report = MicPermission::new(platform, PermissionState::Denied, "模拟用户点了拒绝");
        let practice = permission::decide(&report);
        match &practice {
            Practice::Voice => {
                failures.push(format!("{}：被拒却仍判定语音可用", platform.label()));
            }
            Practice::Typed { reason, message } => {
                eprintln!("  {} -> {reason:?}\n      {message}", platform.label());
                if !message.contains("打字练习") {
                    failures.push(format!("{}：消息没告知降级去向", platform.label()));
                }
            }
        }
    }

    eprintln!();
    eprintln!("=== 场景 B：其余四种权限状态 ===");
    for (state, want) in [
        (
            PermissionState::Restricted,
            Some(DegradeReason::PermissionRestricted),
        ),
        (
            PermissionState::Undetermined,
            Some(DegradeReason::PermissionUndetermined),
        ),
        (PermissionState::Granted, None),
    ] {
        let report = MicPermission::new(Platform::MacOs, state, "");
        let practice = permission::decide(&report);
        eprintln!("  {state:?} -> {:?}", practice.reason());
        if practice.reason() != want {
            failures.push(format!(
                "{state:?} 应判定 {want:?}，实际 {:?}",
                practice.reason()
            ));
        }
    }

    eprintln!();
    eprintln!("=== 场景 B2：crate 入口在未编译语音能力时短路 ===");
    let granted = MicPermission::new(Platform::Linux, PermissionState::Granted, "");
    let entry = yunjian_voice::practice(&granted);
    eprintln!(
        "  is_available()={} -> {:?}",
        yunjian_voice::is_available(),
        entry.reason()
    );
    let expected = if yunjian_voice::is_available() {
        None
    } else {
        Some(DegradeReason::FeatureDisabled)
    };
    if entry.reason() != expected {
        failures.push(format!(
            "crate 入口判定错：期望 {expected:?}，实际 {:?}",
            entry.reason()
        ));
    }

    eprintln!();
    eprintln!("=== 场景 C：采集侧失败也走同一套降级判定 ===");
    let outcome = yunjian_voice::capture::capture_from(
        "绝不存在的设备名-yunjian-denied-probe",
        Duration::from_millis(200),
    );
    match outcome {
        Ok(_) => failures.push("不存在的设备名居然采集成功了".to_owned()),
        Err(err) => {
            let reason = err.degrade_reason();
            let practice = permission::degrade(reason, Platform::current());
            eprintln!("  {err}");
            eprintln!("  -> {reason:?}");
            if reason != DegradeReason::NoInputDevice {
                failures.push(format!("设备名匹配不到应报 NoInputDevice，实际 {reason:?}"));
            }
            if !practice.is_typed() {
                failures.push("采集失败没有降级".to_owned());
            }
        }
    }

    eprintln!();
    if failures.is_empty() {
        eprintln!("全部降级路径成立：没有崩溃，没有静默失败，每一条都带解释。");
    } else {
        eprintln!("发现 {} 处问题：", failures.len());
        for f in &failures {
            eprintln!("  - {f}");
        }
        std::process::exit(1);
    }
}
