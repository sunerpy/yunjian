//! 手动采集探针。存在的理由是 todo 68 的真机验收：macOS 的 TCC 弹窗与移动端的运行时
//! 授权都只能由一次真实运行触发，自动化测试在无人值守的 runner 上覆盖不到那一段。
//!
//! ```text
//! cargo run -p yunjian-voice --features capture --example capture_probe
//! cargo run -p yunjian-voice --features capture --example capture_probe -- "设备名片段"
//! ```

#![expect(
    clippy::print_stderr,
    reason = "手动探针的产物就是给人看的诊断输出；工作区把 print_stderr 设成 warn 是为了把\
              日志逼进 tracing，而这个 example 不是日志、也不进任何发布产物"
)]

use std::time::Duration;

use yunjian_voice::capture;

fn main() {
    match capture::list_inputs() {
        Ok(list) => eprintln!("可用输入设备（{} 个）：{list:#?}", list.len()),
        Err(err) => eprintln!("枚举失败：{err}"),
    }

    let wanted = std::env::args().nth(1);
    let result = match wanted.as_deref() {
        Some(name) => capture::capture_from(name, Duration::from_secs(1)),
        None => capture::capture_default(Duration::from_secs(1)),
    };

    match result {
        Ok(got) => eprintln!(
            "设备 {} 采集 {} 样本 / {} Hz / {} 声道（设备原生 {} Hz / {} 声道）RMS={:.6} 时长={:?}",
            got.device,
            got.samples.len(),
            got.sample_rate,
            got.channels,
            got.device_sample_rate,
            got.device_channels,
            got.rms(),
            got.duration(),
        ),
        Err(err) => {
            let practice = yunjian_voice::permission::degrade(
                err.degrade_reason(),
                yunjian_voice::platform::Platform::current(),
            );
            eprintln!("采集失败：{err}");
            eprintln!("降级判定：{practice:?}");
        }
    }
}
