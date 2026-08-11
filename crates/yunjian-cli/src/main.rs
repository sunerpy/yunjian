//! `yunjian` 可执行文件入口。
//!
//! 这里只做一件事：把 [`yunjian_cli::run`] 返回的退出码交给进程。逻辑全在库目标里，
//! 因为信封、退出码与参数定义要能被进程内单测覆盖，而 stdout/stderr 分离只能由
//! `tests/cli.rs` 用子进程验证。
//!
//! `run` 内部**不**调用 `std::process::exit`：那会跳过日志的 `WorkerGuard` 析构，
//! 把尚未落盘的日志丢掉。退出码因此一路返回到这里。

fn main() {
    std::process::exit(yunjian_cli::run());
}
