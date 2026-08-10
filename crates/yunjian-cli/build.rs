//! 让 `--features voice` 构出的可执行文件能在自己旁边找到 sherpa-onnx 动态库。
//!
//! 缺了这一步，二进制链接的是 `libsherpa-onnx-c-api.so` 却没有任何 rpath，即使
//! `.so` 就躺在同一目录里也会以退出码 127 启动失败——而 `cargo test` 会自己注入
//! `LD_LIBRARY_PATH`，所以测试全绿、发布产物却跑不起来。实测过这个失败。
//!
//! rpath 必须在**二进制所属的包**里发，`cargo:rustc-link-arg` 不会从 rlib 依赖
//! 传递到最终链接步骤。

fn main() {
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
