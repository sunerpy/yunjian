//! Tauri 的构建期代码生成：解析 `tauri.conf.json`（在 macOS 上再叠加
//! `tauri.macos.conf.json`）、内嵌前端资源与图标、生成 ACL 权限表；再为
//! `--features voice` 构出的可执行文件发一条 rpath。
//!
//! 除这两件事之外刻意不做别的。图标校验属 todo 65，权限扩充属 todo 60，两者都不该藏在
//! build script 里——build script 的失败诊断远比一条测试断言难读。
//!
//! # rpath 为什么不违反上面那条原则
//!
//! 那条原则拒绝的是**能用一条断言完成、却被塞进 build script** 的校验工作。rpath 不属于
//! 这一类，理由有两条，缺一不可：
//!
//! 1. **没有「用断言替代」这个选项。** 链接参数只能由 build script 经
//!    `cargo:rustc-link-arg-*` 交给 cargo，而且必须由**二进制所属的包**发——
//!    `cargo:rustc-link-arg` 不会从 rlib 依赖传递到最终链接步骤，所以
//!    `yunjian-voice` 替这里发是无效的（`crates/yunjian-cli/build.rs` 记的是同一件事）。
//! 2. **这段代码不产生需要诊断的失败。** 它只读两个环境变量再打印一行，没有 I/O、
//!    没有校验、没有可 panic 的路径；「诊断难读」这个代价根本不会发生。
//!
//! # 缺了它的失败形态
//!
//! 二进制链接的是 `libsherpa-onnx-c-api.so` 却没有任何 rpath，即使 `.so` 就躺在同一目录
//! 里也会在启动时报 `cannot open shared object file` 而起不来——而 `cargo test` 会自己
//! 注入 `LD_LIBRARY_PATH`，于是**测试全绿、发布产物却跑不起来**。`yunjian-cli` 实测过这个
//! 失败，`yunjian-desktop` 在补上这段之前同样实测复现过。
//!
//! 逻辑与 `crates/yunjian-cli/build.rs` 刻意保持一致，并由
//! `xtask/tests/workspace_contract.rs` 的一条断言守住「每个可开 voice 的分发二进制都发了
//! 等价的 rpath」，防止两份之中的任何一份被删掉或改歪。

fn main() {
    tauri_build::build();
    emit_voice_rpath();
}

fn emit_voice_rpath() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var_os("CARGO_FEATURE_VOICE").is_none() {
        return;
    }

    match std::env::var("CARGO_CFG_TARGET_OS")
        .unwrap_or_default()
        .as_str()
    {
        "linux" | "android" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
        }
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path");
        }
        // Windows 在可执行文件所在目录搜索 DLL，无需等价设置。
        _ => {}
    }
}
