//! `voice` 特性开启时的构建前置检查。
//!
//! 存在的唯一理由：`sherpa-rs-sys` 缺少前置条件时抛出的是 `ureq` / `bindgen` 的原始
//! 错误或裸链接错误，读者无法从中知道要装什么。这里在链接前把已知的三类前置条件
//! 检出来，并把错误指向 `docs/VOICE-BUILD.zh.md`。
//!
//! 已知顺序限制：cargo 先构建依赖的 build script，所以完全离线的首次构建里
//! `sherpa-rs-sys` 会先失败，本检查来不及发声。`docs/VOICE-BUILD.zh.md`
//! 「已知限制」一节记录了这一点以及绕过办法。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    println!("cargo:rerun-if-env-changed=SHERPA_LIB_PATH");

    if std::env::var_os("CARGO_FEATURE_VOICE").is_none() {
        return;
    }

    let target = std::env::var("TARGET").unwrap_or_default();
    if let Err(missing) = check_prerequisites(&target) {
        panic!(
            "\n\n云笺语音（--features voice）前置条件缺失：{missing}\n\
             目标平台：{target}\n\
             安装步骤见 docs/VOICE-BUILD.zh.md「按平台前置条件」一节。\n\
             不需要语音时去掉 `--features voice` 即可正常构建词典与默写功能。\n\n"
        );
    }
}

/// `dist.json` 声明有官方预编译产物的目标三元组。不在表内的目标会让
/// `sherpa-rs-sys` 去请求一个不存在的归档，报出难以理解的下载错误。
const PREBUILT_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-linux-android",
    "x86_64-linux-android",
    "armv7-linux-androideabi",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
];

fn check_prerequisites(target: &str) -> Result<(), String> {
    if !PREBUILT_TARGETS.contains(&target) {
        return Err(format!(
            "目标 `{target}` 没有 sherpa-onnx 官方预编译产物。\
             需改用源码编译（额外要求 cmake >= 3.13 与 C++17 编译器），\
             或退回无语音构建"
        ));
    }

    // libclang 只在 Linux 上探测。`clang-sys` 在 macOS 走 xcode-select、在 Windows 扫注册表
    // 与若干安装目录，那套发现逻辑无法在这里如实复制——照抄一半只会产生假阴性，
    // 拦下本来能成功的构建。实测就发生过：仅按 Linux 路径探测，导致 macOS / Windows /
    // iOS 三个 CI 作业被误判为缺前置条件。这两个平台交给 `bindgen` 自己报错，
    // 它的提示已经指名 `LIBCLANG_PATH`，足够可操作。
    if cfg!(target_os = "linux") && !linux_has_libclang() {
        return Err(
            "找不到 libclang，`bindgen` 无法生成 sherpa-onnx C API 绑定。\
             Debian/Ubuntu 装 `libclang-dev`，并在 libclang 不位于默认搜索路径时\
             设置 `LIBCLANG_PATH`（Debian 通常是 /usr/lib/llvm-<版本>/lib）"
                .to_owned(),
        );
    }

    if target.contains("android") && std::env::var_os("ANDROID_NDK_HOME").is_none() {
        return Err(
            "Android 目标需要 `ANDROID_NDK_HOME` 指向 NDK r26 或更高版本，\
             `bindgen` 与链接器都要用它的 sysroot"
                .to_owned(),
        );
    }

    Ok(())
}

fn linux_has_libclang() -> bool {
    if std::env::var_os("LIBCLANG_PATH").is_some() {
        return true;
    }
    let flat = ["/usr/lib", "/usr/local/lib", "/usr/lib/x86_64-linux-gnu"];
    flat.iter()
        .any(|dir| dir_holds_libclang(std::path::Path::new(dir)))
        || llvm_versioned_dirs()
}

fn dir_holds_libclang(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("libclang") && name.contains(".so")
        })
    })
}

/// Debian 把 libclang 放在 `/usr/lib/llvm-<版本>/lib` 下，不在默认搜索路径里。
fn llvm_versioned_dirs() -> bool {
    std::fs::read_dir("/usr/lib").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|e| {
            e.file_name().to_string_lossy().starts_with("llvm-")
                && dir_holds_libclang(&e.path().join("lib"))
        })
    })
}
