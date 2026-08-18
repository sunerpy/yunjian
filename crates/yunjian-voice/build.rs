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

    declare_model_cfgs();

    if std::env::var_os("CARGO_FEATURE_VOICE").is_none() {
        return;
    }

    let target = std::env::var("TARGET").unwrap_or_default();
    if let Err(missing) =
        check_prerequisites(&target).and_then(|()| link_unmapped_android_prebuilt(&target))
    {
        panic!(
            "\n\n云笺语音（--features voice）前置条件缺失：{missing}\n\
             目标平台：{target}\n\
             安装步骤见 docs/VOICE-BUILD.zh.md「按平台前置条件」一节。\n\
             不需要语音时去掉 `--features voice` 即可正常构建词典与默写功能。\n\n"
        );
    }
}

/// 需要真实权重的测试各自依赖的模型目录，以及它落地时打开的 cfg 名。
///
/// # 为什么要在构建期探测，而不是在测试里判目录
///
/// 这些测试要么真跑推理，要么读真实词典，缺模型时无法执行。三种处理方式里只有一种诚实：
///
/// - `assert!(dir.is_dir())`：缺模型时**变红**，而红的原因与被测契约无关（原状，F1 把
///   todo 54 判 FAIL 就是撞在这上面）；
/// - 运行时 `if !dir.is_dir() { return; }`：harness 打印 `ok`，**「没跑」冒充「通过」**；
/// - `#[cfg_attr(not(<cfg>), ignore = "<原因>")]`：测试输出里留下一行 `ignored` 及理由，
///   而模型在位时照常真跑。
///
/// `cfg_attr` 的条件必须是编译期常量，所以由本 build script 供给。这与
/// `crates/yunjian-cli/tests/install_scripts.rs` 用 `not(unix)` 门控 POSIX 脚本用例
/// 是同一个手法，只是条件来源不同。
///
/// # 陈旧性与其边界
///
/// cfg 在编译期定，模型是运行期文件。下面对模型目录与 `YUNJIAN_MODEL_DIR` 都声明了
/// rerun，因此「下载完模型再跑测试」会触发重编译并让 cfg 翻转。反向的陈旧
/// （cfg 说在、目录被删）由测试里保留的 `is_dir` 断言兜住：那时应当变红，
/// 因为环境在两次动作之间被改坏了。
const WEIGHT_BACKED_MODELS: &[(&str, &str)] = &[
    ("vits-melo-tts-zh_en", "melo_model_present"),
    ("kitten-nano-en-v0_2-fp16", "kitten_model_present"),
    ("sherpa-onnx-whisper-tiny", "whisper_tiny_model_present"),
];

fn declare_model_cfgs() {
    println!("cargo:rerun-if-env-changed=YUNJIAN_MODEL_DIR");
    let root = model_cache_root();
    for (model, cfg) in WEIGHT_BACKED_MODELS {
        println!("cargo::rustc-check-cfg=cfg({cfg})");
        let dir = root.join(model);
        println!("cargo:rerun-if-changed={}", dir.display());
        if dir.is_dir() {
            println!("cargo:rustc-cfg={cfg}");
        }
    }
}

/// 必须与 `crate::models::cache_root()` 同口径：`YUNJIAN_MODEL_DIR` 覆盖，
/// 否则仓库内 `models/cache`。两处口径分叉会让 cfg 与测试看的是不同目录。
fn model_cache_root() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("YUNJIAN_MODEL_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo 提供 manifest 目录"))
        .join("..")
        .join("..")
        .join("models")
        .join("cache")
}

/// 上游 sherpa-onnx 发布归档里**确实有**预编译产物的目标三元组。不在表内的目标会让
/// `sherpa-rs-sys` 去请求一个不存在的归档，报出难以理解的下载错误。
///
/// # `i686-linux-android` 为什么在表内
///
/// Android 那一档是**一个归档四个 ABI**：`sherpa-onnx-<tag>-android.tar.bz2` 解出来是
/// `jniLibs/{arm64-v8a,armeabi-v7a,x86,x86_64}/`，`jniLibs/x86/*.so` 实测是
/// `ELF 32-bit LSB shared object, Intel 80386`。所以「x86 没有上游预编译产物」是错的，
/// 曾据此把 `android_per_abi_apks` 判成需用户裁量的取舍——**产物一直在同一个包里**。
///
/// 缺的是中间那层映射：`sherpa-rs-sys` 的 `dist.json` 里 `targets.android.targets` 只写了
/// 另外三个 triple。它对任何含 `android` 的 triple 都返回同一个归档（i686 照样下载、
/// 照样解包），但随后取不到 i686 的 lib 清单，回退去 glob 一个 Android 归档里不存在的
/// 目录，于是**一条 `cargo:rustc-link-lib` 都不发**。补这一层的是下面的
/// `link_unmapped_android_prebuilt`。
const PREBUILT_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-linux-android",
    "x86_64-linux-android",
    "i686-linux-android",
    "armv7-linux-androideabi",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
];

/// `sherpa-rs-sys` 的 `dist.json` 没有映射、因而拿不到链接标志的 Android triple。
///
/// 这份清单只此一处。曾经打算让 `xtask` 也持一份并由 gradle 传参，那会造出两份可以
/// 悄悄分叉的清单（分叉的形态是摆了位却不发标志、或反过来，都表现为链接期
/// undefined symbol）；同时改 `mobile/android/app/build.gradle.kts` 还会打断
/// `docs/reports/mobile-qa-*.json` 的溯源断言——那份报告对它做内容摘要强制。
/// 由本脚本自己完成全部工作，两个问题一起没有。
const UNMAPPED_ANDROID_TARGETS: &[&str] = &["i686-linux-android"];

/// 未映射 triple 需要的两个 `.so`。与 `dist.json` 为已映射 Android triple 列出的同名：
/// x86 是同一个产品的同一套依赖，不多不少。
///
/// 两个都必须在：`libsherpa-onnx-c-api.so` 是 `libyunjian_mobile.so` 的 `NEEDED`，
/// `libonnxruntime.so` 又是前者的 `NEEDED`。少任何一个都在 `System.loadLibrary`
/// 抛 `UnsatisfiedLinkError`，而报错只提主库的名字。
const ANDROID_SHERPA_LIBS: &[&str] = &["libsherpa-onnx-c-api.so", "libonnxruntime.so"];

/// 为 `dist.json` 未映射的 Android triple 补上链接标志，并把那两个 `.so` 摆进
/// cargo 的目标目录。
///
/// 不补链接标志的后果是 cdylib 链接期的 undefined symbol——那种报错读起来像 Rust 侧
/// 代码写错了，而真因在三层之外（上游 `dist.json` 的映射表）。
///
/// 摆进目标目录是因为 `mobile/android/app/build.gradle.kts` 从
/// `target/<triple>/release/` 取这两个 `.so` 放进 `jniLibs/<abi>/`。`sherpa-rs-sys`
/// 对**已映射**的 triple 自己做这件事（`copy_file` 到 `target_dir`），这里对未映射的
/// triple 做同一件事，目的地与算法都与它一致，所以 gradle 侧无需知道有例外。
///
/// 归档不重新下载：它对四个 Android ABI 是同一份（缓存目录名就是同一个校验和），
/// 所以任何一个已解包的 Android triple 缓存都能供给。重写下载就要复制上游的 tag、
/// URL 与校验和，那是第二份会分叉的清单。
fn link_unmapped_android_prebuilt(target: &str) -> Result<(), String> {
    if !UNMAPPED_ANDROID_TARGETS.contains(&target) {
        return Ok(());
    }
    let abi = android_abi(target)?;
    let cache_root = sherpa_cache_root()
        .ok_or_else(|| "定位不到 sherpa-rs 缓存根（XDG_CACHE_HOME 与 HOME 都没有）".to_owned())?;
    let source = find_android_jni_libs(&cache_root, abi).ok_or_else(|| {
        format!(
            "目标 `{target}` 不在 sherpa-rs-sys 的 dist.json 映射表里，需要复用上游 Android \
             归档里的 jniLibs/{abi}/，但在 {} 下找不到已解包的那一份。\
             上游把四个 ABI 打在同一个归档里，所以先构建一个已映射的 ABI \
             （aarch64-linux-android / armv7-linux-androideabi / x86_64-linux-android 任一）\
             让 sherpa-rs-sys 下载并解包它，再重来",
            cache_root.display()
        )
    })?;

    println!("cargo:rustc-link-search=native={}", source.display());
    for name in ANDROID_SHERPA_LIBS {
        let stem = name
            .strip_prefix("lib")
            .and_then(|rest| rest.strip_suffix(".so"))
            .expect("清单里的名字都形如 lib*.so");
        println!("cargo:rustc-link-lib=dylib={stem}");
    }

    let target_dir = cargo_target_dir()?;
    for name in ANDROID_SHERPA_LIBS {
        let from = source.join(name);
        let to = target_dir.join(name);
        if to.exists() {
            continue;
        }
        if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).map_err(|error| {
                format!(
                    "把 {} 复制到 {} 失败：{error}",
                    from.display(),
                    to.display()
                )
            })?;
        }
    }
    Ok(())
}

/// `<triple>` → 上游归档里 `jniLibs/` 下的目录名。两者不是同一套命名
/// （`i686` 对 `x86`），照抄 triple 会去找一个不存在的目录。
fn android_abi(target: &str) -> Result<&'static str, String> {
    match target {
        "i686-linux-android" => Ok("x86"),
        "x86_64-linux-android" => Ok("x86_64"),
        "aarch64-linux-android" => Ok("arm64-v8a"),
        "armv7-linux-androideabi" => Ok("armeabi-v7a"),
        other => Err(format!("`{other}` 不是已知的 Android target triple")),
    }
}

/// 上游 Android 归档解包后的 `jniLibs/<abi>/`，要求两个 `.so` 都在。
///
/// 只认齐备的目录：半个归档比没有更危险——构建会过，装到设备上才在
/// `System.loadLibrary` 抛 `UnsatisfiedLinkError`，那时离构建已经很远了。
fn find_android_jni_libs(cache_root: &std::path::Path, abi: &str) -> Option<std::path::PathBuf> {
    let mut triples: Vec<std::path::PathBuf> = std::fs::read_dir(cache_root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("linux-android"))
        })
        .collect();
    triples.sort();
    for triple_dir in triples {
        let mut sums: Vec<std::path::PathBuf> = std::fs::read_dir(&triple_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        sums.sort();
        for sum_dir in sums {
            let candidate = sum_dir.join("jniLibs").join(abi);
            if ANDROID_SHERPA_LIBS
                .iter()
                .all(|name| candidate.join(name).is_file())
            {
                return Some(candidate);
            }
        }
    }
    None
}

/// `dirs::cache_dir()/sherpa-rs` 的等价物。口径必须与 `sherpa-rs-sys` 一致，
/// 分叉了就会去一个空目录里找归档，然后报「先构建已映射 ABI」——而那条提示是错的。
fn sherpa_cache_root() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return Some(std::path::PathBuf::from(dir).join("sherpa-rs"));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| {
            std::path::PathBuf::from(home)
                .join(".cache")
                .join("sherpa-rs")
        })
}

/// `target/<triple>/<profile>/`，即 gradle 取 `.so` 的那个目录。
///
/// 从 `OUT_DIR` 上溯到名字等于 `PROFILE` 的那一级，与 `sherpa-rs-sys` 的
/// `get_cargo_target_dir` 同一个算法：目的地必须与它对已映射 triple 用的完全一致，
/// 否则 gradle 会在两个地方之一找不到文件。
fn cargo_target_dir() -> Result<std::path::PathBuf, String> {
    let out_dir = std::env::var("OUT_DIR").map_err(|_| "cargo 提供 OUT_DIR".to_owned())?;
    let profile = std::env::var("PROFILE").map_err(|_| "cargo 提供 PROFILE".to_owned())?;
    let mut cursor = std::path::Path::new(&out_dir);
    while let Some(parent) = cursor.parent() {
        if parent.ends_with(&profile) {
            return Ok(parent.to_path_buf());
        }
        cursor = parent;
    }
    Err(format!("从 OUT_DIR `{out_dir}` 上溯不到 `{profile}` 目录"))
}

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
