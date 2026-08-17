use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 受保护的领域逻辑：移动门面只许接线，不许改内核。
///
/// 判据刻意**不是** `git diff <pre-mobile-sha>`。那种写法要求测试环境持有某个历史提交，
/// 而 `actions/checkout` 默认浅克隆（`fetch-depth: 1`）拿不到它——同一条断言在本机通过、
/// 在 Linux 与 Windows 两个 runner 上必然失败。一条「内核有没有被动过」的断言不该因为
/// 克隆深度而失效或通过，所以改成扫真实文件算摘要，与 git、`.git` 目录、克隆深度全部解耦。
const PROTECTED_PATHS: [&str; 3] = [
    "crates/yunjian-core/src/search",
    "crates/yunjian-recite/src/score.rs",
    "crates/yunjian-voice/src/session.rs",
];

/// 与 `PROTECTED_PATHS` 一一对应的期望摘要。
///
/// **要改这里，先确认那次内核改动是有意的。** 摘要变了只有两种可能：受保护逻辑真的被改了
/// （那就该被显式确认一次），或者有人在改这条常量让红测试变绿（那正是守卫要拦的事）。
///
/// 摘要覆盖「仓库相对路径 + 文件长度 + 内容」，因此改名和增删文件同样会被抓到，
/// 代价是它**不等于** `sha256sum` 对单个文件的输出，不要拿后者来核对。
const PROTECTED_DIGESTS: [&str; 3] = [
    "2c4ef34e2882ea479943036ffccc1f82705582fb0ab5ee205ffeef986b73b457",
    "4b95365d8f9f00919b4e5bc3707cdf4197e5a17418e3f1ea29e1ec9d8f9f5a3f",
    "558d75731a7482dd3b480848553893228627268148169ef310b37886560f72d4",
];

/// 每条受保护路径下的文件数。目录摘要已经覆盖增删文件，这条只为把「文件集合变了」与
/// 「文件内容变了」报成两句不同的话——否则新增一个文件只会得到一个无从下手的摘要不符。
const PROTECTED_FILE_COUNTS: [usize; 3] = [10, 1, 1];

const DOMAIN_CRATES: [&str; 4] = [
    "yunjian-core",
    "yunjian-ai",
    "yunjian-recite",
    "yunjian-voice",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("mobile crate 应位于 workspace/crates 下")
        .to_path_buf()
}

#[test]
fn manifest_directly_depends_on_all_four_domain_crates() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("读取 mobile manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("解析 mobile manifest");
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应有 dependencies 表");

    for domain in DOMAIN_CRATES {
        assert!(
            dependencies.contains_key(domain),
            "移动门面缺少直接领域依赖 {domain}"
        );
    }
}

#[test]
fn manifest_builds_only_the_selected_uniffi_branch() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("读取 mobile manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("解析 mobile manifest");
    let features = parsed
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应声明 features");
    assert!(
        features.contains_key("uniffi") && !features.contains_key("tauri"),
        "裁决为 UniFFI 时必须只声明 uniffi binding feature"
    );
    let uniffi = features
        .get("uniffi")
        .and_then(toml::Value::as_array)
        .expect("uniffi feature 应为依赖数组");
    let uniffi_members = uniffi
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert!(uniffi_members.contains(&"dep:uniffi"));
    assert!(
        !uniffi_members
            .iter()
            .any(|member| member.contains("yunjian-voice/voice")),
        "普通 UniFFI 绑定不得静默拖入 GPL 语音栈"
    );
    let native_voice = features
        .get("native-voice")
        .and_then(toml::Value::as_array)
        .expect("native-voice feature 应为依赖数组")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        native_voice,
        ["uniffi", "yunjian-voice/voice"],
        "真实 ASR 必须是显式的 UniFFI 语音扩展"
    );
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应有 dependencies 表");
    assert!(dependencies.contains_key("uniffi"), "缺少 UniFFI 依赖");
    assert!(
        !dependencies.contains_key("tauri"),
        "不得构建 Tauri mobile 分支"
    );
}

#[test]
fn generated_bindings_and_android_initializer_are_versioned() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let kotlin = std::fs::read_to_string(
        crate_root.join("bindings/generated/top/yunjian/mobile/yunjian_mobile.kt"),
    )
    .expect("缺少生成的 Kotlin binding；运行 generate-bindings.sh");
    let swift = std::fs::read_to_string(crate_root.join("bindings/generated/YunjianMobile.swift"))
        .expect("缺少生成的 Swift binding；运行 generate-bindings.sh");
    let header = crate_root.join("bindings/generated/YunjianMobileFFI.h");
    let modulemap = crate_root.join("bindings/generated/YunjianMobileFFI.modulemap");
    let android = std::fs::read_to_string(
        crate_root.join("bindings/android/top/yunjian/mobile/YunjianAndroid.kt"),
    )
    .expect("缺少 Android context 初始化包装器");

    for required in [
        "package top.yunjian.mobile",
        "open class NativeFacade",
        "open class NativeOperation",
        "open class NativeAsrOperation",
        "public interface NativeEventSink",
    ] {
        assert!(
            kotlin.contains(required),
            "Kotlin binding 缺少 `{required}`"
        );
    }
    for required in [
        "open class NativeFacade",
        "open class NativeOperation",
        "open class NativeAsrOperation",
        "public protocol NativeEventSink",
        "func appreciateStream",
        "func startAsr",
    ] {
        assert!(swift.contains(required), "Swift binding 缺少 `{required}`");
    }
    assert!(header.is_file(), "缺少 Swift C header");
    assert!(modulemap.is_file(), "缺少 Swift modulemap");
    for required in [
        "object YunjianAndroid",
        "context.applicationContext",
        "System.loadLibrary(\"yunjian_mobile\")",
        "initializeNative(context: Context)",
    ] {
        assert!(
            android.contains(required),
            "Android 初始化器缺少 `{required}`"
        );
    }
    assert!(crate_root.join("generate-bindings.sh").is_file());
    assert!(crate_root.join("uniffi.toml").is_file());
}

/// Android 产品工程必须存在，且只能经 UniFFI 生成物调用 Rust。
///
/// 判据是源码扫描而不是「构建一次看看」：构建 Android 需要 SDK + NDK + Gradle，
/// 而这条断言要在普通 `cargo test` 里跑。它拦的是三件会让「UniFFI 分支已落地」退回
/// 声明的事——工程消失、绕过生成物直接 JNI、以及 `YunjianAndroid.initialize` 不在
/// 任何 Rust 调用之前。
#[test]
fn android_shell_exists_and_only_calls_rust_through_the_bindings() {
    let root = workspace_root();
    let android = root.join("mobile/android");
    for required in [
        "settings.gradle.kts",
        "build.gradle.kts",
        "gradle.properties",
        "gradle/libs.versions.toml",
        "app/build.gradle.kts",
        "app/src/main/AndroidManifest.xml",
        "app/src/main/java/top/onethinker/yunjian/YunjianApplication.kt",
        "app/src/main/java/top/onethinker/yunjian/MainActivity.kt",
        "app/src/main/java/top/onethinker/yunjian/YunjianRepository.kt",
        "app/src/androidTest/java/top/onethinker/yunjian/FullAcceptanceTest.kt",
    ] {
        assert!(
            android.join(required).is_file(),
            "Android 产品工程缺 {required}；缺了它 `UNIFFI_NATIVE_BINDING = true` 只是一句声明"
        );
    }

    let application = std::fs::read_to_string(
        android.join("app/src/main/java/top/onethinker/yunjian/YunjianApplication.kt"),
    )
    .expect("读取 YunjianApplication.kt");
    assert!(
        application.contains("YunjianAndroid.initialize(this)"),
        "Application.onCreate 必须先交出 application context；\
         漏掉它会让 ndk-context 在钥匙串路径上取不到 JavaVM"
    );

    // 除 UniFFI 生成的初始化器之外，产品源码不得自己声明 `external fun`：那意味着绕过
    // 生成物直接接 JNI，而方案要求移动端只经 `yunjian-mobile` 的绑定调用领域逻辑。
    let mut offenders = Vec::new();
    let mut sources = Vec::new();
    collect_files(&android.join("app/src"), &mut sources);
    for source in sources {
        if source.extension().and_then(|ext| ext.to_str()) != Some("kt") {
            continue;
        }
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", source.display()));
        // 剔除注释再判：本项目已六次踩「解释一条规则的文字命中这条规则」。
        let code = text
            .lines()
            .filter(|line| {
                !line.trim_start().starts_with("//") && !line.trim_start().starts_with('*')
            })
            .collect::<Vec<_>>()
            .join("\n");
        if code.contains("external fun") {
            offenders.push(source.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "Android 产品源码不得自己声明 external fun（绕过 UniFFI 生成物直接接 JNI）：{offenders:?}"
    );
}

/// 开着语音的 Android 构建必须把许可原文打进 APK。
///
/// GPL-3.0 的声明义务要求原文随分发物走，「源码可得」只满足一半。这条断言看的是
/// 构建脚本真的接了那个目录，因为漏掉它的表现不是构建失败而是**合规缺口**——
/// 一份看起来正常的 APK 里少一份声明，没有任何红色能提示它。
#[test]
fn android_build_ships_the_license_payload() {
    let root = workspace_root();
    let script = std::fs::read_to_string(root.join("mobile/android/app/build.gradle.kts"))
        .expect("读取 app/build.gradle.kts");
    let code = script
        .lines()
        .filter(|line| !line.trim_start().starts_with("//") && !line.trim_start().starts_with('*'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("packaging/licenses"),
        "Android 构建必须把 packaging/licenses/ 搬进产物"
    );
    assert!(
        code.contains("assets.srcDir") && code.contains("licenseAssets"),
        "许可载荷必须经 assets 源集进 APK，否则它不在分发物里"
    );
    assert!(
        code.contains("dependsOn(cargoNdkBuild, copyLicenseAssets)"),
        "许可拷贝必须挂在 preBuild 上；不挂就只在手动执行时生效"
    );
    for required in ["LICENSE-GPL-3.0.txt", "LICENSE-MIT.txt", "NOTICE.md"] {
        assert!(
            root.join("packaging/licenses").join(required).is_file(),
            "许可载荷缺 {required}"
        );
    }
}

#[test]
fn domain_crates_contain_no_uniffi_dependency() {
    let root = workspace_root();
    for domain in DOMAIN_CRATES {
        let manifest = root.join("crates").join(domain).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", manifest.display()));
        let parsed: toml::Value = toml::from_str(&text)
            .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", manifest.display()));
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let entries = parsed.get(section).and_then(toml::Value::as_table);
            assert!(
                entries.is_none_or(|table| !table.contains_key("uniffi")),
                "{domain} 的 {section} 不得引入 uniffi"
            );
        }
    }
}

#[test]
fn protected_domain_logic_matches_its_frozen_digest() {
    assert_eq!(
        PROTECTED_PATHS,
        [
            "crates/yunjian-core/src/search",
            "crates/yunjian-recite/src/score.rs",
            "crates/yunjian-voice/src/session.rs",
        ],
        "受保护路径集合由方案冻结为这三条；增删或替换条目先确认这是有意的架构变更，\
         不要为了让红测试变绿改这里"
    );

    let root = workspace_root();
    for (index, relative) in PROTECTED_PATHS.into_iter().enumerate() {
        let (digest, file_count) = digest_protected_path(&root, relative);
        assert_eq!(
            file_count, PROTECTED_FILE_COUNTS[index],
            "{relative} 的文件数由 {} 变成 {file_count}：受保护逻辑增删了文件",
            PROTECTED_FILE_COUNTS[index]
        );
        assert_eq!(
            digest, PROTECTED_DIGESTS[index],
            "移动门面不得修改受保护领域逻辑：{relative} 的摘要已变。\
             若这次内核改动是有意的，把期望值改成 {digest}"
        );
    }
}

/// 逐字节摘要在 Windows 上成立的前提是仓库根 `.gitattributes` 把行尾钉成 LF。
/// 这条规则丢了，上面那条摘要守卫在 `windows-latest` 上不是「更严格」而是「必然误报」——
/// 与 `xtask verify-sources` 当年在 Windows 上算出 `fff02f3b…` 而记录值是 `c195319a…`
/// 是同一个成因。把这个前置条件写成可执行断言，而不是留在注释里。
#[test]
fn byte_exact_digest_guard_keeps_its_line_ending_precondition() {
    let attributes = std::fs::read_to_string(workspace_root().join(".gitattributes"))
        .expect("读取仓库根 .gitattributes");
    assert!(
        attributes
            .lines()
            .any(|line| line.split('#').next().unwrap_or(line).trim() == "* text=auto eol=lf"),
        "`.gitattributes` 丢了 `* text=auto eol=lf`，逐字节摘要守卫会在 Windows 上误报"
    );
}

fn digest_protected_path(root: &Path, relative: &str) -> (String, usize) {
    let mut files = Vec::new();
    collect_files(&root.join(relative), &mut files);

    let mut entries: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| {
            let key = path
                .strip_prefix(root)
                .unwrap_or_else(|error| panic!("{} 不在仓库根下：{error}", path.display()))
                .to_string_lossy()
                .replace('\\', "/");
            (key, path)
        })
        .collect();
    entries.sort();

    let mut hasher = Sha256::new();
    for (key, path) in &entries {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
        hasher.update(key.as_bytes());
        hasher.update([0u8]);
        hasher.update(
            u64::try_from(bytes.len())
                .expect("文件长度应可表示为 u64")
                .to_le_bytes(),
        );
        hasher.update(&bytes);
    }

    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(digest, "{byte:02x}");
    }
    (digest, entries.len())
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>) {
    let metadata = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("读取 {} 元信息失败：{error}", path.display()));
    if metadata.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("读取目录 {} 失败：{error}", path.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("遍历 {} 失败：{error}", path.display()))
                .path()
        })
        .collect();
    children.sort();
    for child in children {
        collect_files(&child, out);
    }
}
