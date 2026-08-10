//! `yunjian` 可执行文件入口。
//!
//! 参数解析、配置与日志初始化、子命令分派由后续任务补全；此处仅保留
//! 可编译骨架。
//!
//! 注意：本二进制同时承载 MCP stdio 服务端，因此除唯一一处 CLI 展示
//! 模块外，任何位置都不得向 stdout 写入内容。

/// 语音后端的诊断串。返回值目前无人消费，但这次调用必须留着：它是
/// `--features voice` 下 onnxruntime 真的被链接进来的证据。去掉它，链接器就会把
/// 整个原生依赖丢弃，`ldd` 断言与安装包体积测量都会变成空话。
fn voice_backend() -> String {
    match yunjian_voice::backend_version() {
        Some(version) => format!("sherpa-onnx {version}"),
        None => "disabled".to_owned(),
    }
}

fn main() {
    let _ = voice_backend();
}
