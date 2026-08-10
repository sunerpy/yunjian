//! 全工作区唯一允许向 stdout 写入的模块。
//!
//! 这里是 CLI 的展示层：人类可读输出与 `--json` 机器可读输出都从这里出去，
//! 别处一律不许碰 stdout。原因不是洁癖：同一个二进制还承载 `yunjian mcp` 的
//! stdio 服务端，stdout 就是 MCP 的协议流，掺进一行杂音就让整个会话解析失败。
//!
//! 因此禁令是工作区级 `deny`（根 `Cargo.toml` 的 `[workspace.lints.clippy]`
//! 加 `clippy.toml`），豁免只有本文件顶部这**一处**，且有测试盯着它的数量。
//! 要新开一个 stdout 出口，请改成调用这里的函数，而不是再加一个 `#[allow]`。
//!
//! 诊断、日志、进度提示都不属于这里——那些走 `tracing`，最终落到 stderr 与日志文件。
#![allow(clippy::print_stdout, clippy::disallowed_methods)]

use std::io::{self, Write};

/// 输出一行人类可读文本。
pub fn line(text: &str) {
    println!("{text}");
}

/// 输出一段已序列化的机器可读载荷（`--json` 路径）。
///
/// 不换行由调用方决定：JSON Lines 需要行尾换行，单份文档不需要多余空行。
pub fn payload(serialized: &str) {
    println!("{serialized}");
}

/// 把缓冲区刷到 stdout。
///
/// 进程正常退出时 Rust 会自行刷新，但 `std::process::exit` 不会——CLI 用非零退出码
/// 结束时若不显式刷一次，最后几行输出会被丢掉。
pub fn flush() -> io::Result<()> {
    io::stdout().flush()
}
