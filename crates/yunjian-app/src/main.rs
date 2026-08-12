//! `yunjian-desktop` 可执行文件入口。
//!
//! 这里只做一件事：把控制权交给 [`yunjian_app::run`]。逻辑全在库目标里，
//! 因为窗口配置的一致性与日志引导顺序要能被测试覆盖，而拉起真实 WebView 只能靠真机验收。
//!
//! **不调用 `std::process::exit`**：那会跳过日志 guard 的析构，把尚未落盘的日志丢掉。
//! Tauri 的事件循环正常返回即进程结束。

fn main() {
    yunjian_app::run();
}
