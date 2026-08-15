//! 桌面安装包入口的门禁：`make bundle` 必须能跑通，且必须逐类核对产物。
//!
//! # 这组断言是为哪一次真实失败写的
//!
//! F1 合规审计在 `main@3047b62` 上跑 `cargo tauri build --debug`，读到「AppImage 阶段
//! failed to run linuxdeploy，整体退出 1」，据此把 todo 59 判 FAIL。实测复现的结论与这句
//! 描述**不同**：AppImage 阶段成功了（163 MiB 产物，`--appimage-extract` 能解开并含
//! `usr/bin/yunjian-desktop`），非零退出来自打包**之后**的签名步骤——
//! `A public key has been found, but no private key`。
//!
//! 两件事因此必须被机制守住，而不是靠人跑一次再解读：
//!
//! 1. **入口本身要能在没有发布私钥的机器上跑到底。** `plugins.updater.pubkey` 一旦存在，
//!    裸 `cargo tauri build` 在本机与容器里必然以签名失败收尾，而那句报错听起来像打包坏了。
//!    `make bundle` 用 `--no-sign` 把这条路走通；发布签名留在 workflow 里，且那边缺 key
//!    是硬失败——下面第四条断言就是防止 `--no-sign` 漂进发布路径。
//! 2. **产物必须逐类点名核对。** Linux updater 只消费 AppImage（`.deb` 不能自动更新），
//!    所以「少一个 AppImage」不是少一个可选格式，是断掉 Linux 的自动更新链。
//!
//! # 为什么扫 `Makefile` 而不是重实现一份校验
//!
//! 「哪些产物必须存在」在仓库里已经有两个消费方：`make bundle` 与
//! `.github/workflows/release-please.yml` 的收集步骤。再用 Rust 写第三份实现就多出一处
//! 会漂的事实来源。这里的做法与 README 行数守卫同形：常量自锁 + 扫真实文件，
//! 只判「入口有没有保住那几条性质」。
//!
//! # 三层范围不要混
//!
//! - `BUNDLE_KINDS` 的取值域：本文件的 `REQUIRED_BUNDLE_KINDS`，自锁。
//! - 本机入口：`Makefile` 的 `bundle` 目标，必须 `--no-sign` 且 `-v`。
//! - 发布入口：workflow，必须签名、**不得**出现 `--no-sign`。

use serde_json::Value;
use std::path::{Path, PathBuf};

/// 本机打包必须逐类核对的产物类别。
///
/// 取值就是执行机制：把 `appimage` 从 `Makefile` 的 `BUNDLE_KINDS` 里删掉（无论是为了
/// 「先让命令绿起来」还是手滑），下面第一条断言立刻变红。
const REQUIRED_BUNDLE_KINDS: [&str; 3] = ["deb", "rpm", "appimage"];

/// CLI 发布矩阵的冻结契约：目标、原生 runner 与是否使用 cargo-zigbuild。
///
/// Linux 必须是 musl 静态目标且只在这两条腿使用 zigbuild；ARM Windows 必须在
/// `windows-11-arm` 上原生构建，不能悄悄退回交叉编译。
const REQUIRED_RELEASE_TARGETS: [(&str, &str, bool); 6] = [
    ("x86_64-unknown-linux-musl", "ubuntu-24.04", true),
    ("aarch64-unknown-linux-musl", "ubuntu-24.04-arm", true),
    ("x86_64-apple-darwin", "macos-15-intel", false),
    ("aarch64-apple-darwin", "macos-14", false),
    ("x86_64-pc-windows-msvc", "windows-latest", false),
    ("aarch64-pc-windows-msvc", "windows-11-arm", false),
];

/// Linux 自动更新消费的那一个产物。少了它 updater 拿不到可安装的包。
const LINUX_UPDATER_ARTIFACT: &str = "appimage";

/// `updater.rs` 里 Linux 的平台键。它存在，就意味着上面那个产物是发布必需项。
const LINUX_UPDATE_TARGET: &str = "linux-x86_64";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 仓库根。本 crate 的 manifest 目录是 `<root>/crates/yunjian-app`。
fn repo_root() -> PathBuf {
    crate_dir()
        .join("../..")
        .canonicalize()
        .expect("仓库根应当存在")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()))
}

fn makefile() -> String {
    read(&repo_root().join("Makefile"))
}

/// 取 `Makefile` 里某个立即展开变量的值。刻意只认 `:=`：仓库的门禁约定是
/// 「一律 `:=`，绝不 `?=`」，用 `?=` 写的变量能被环境变量悄悄改掉，那种写法本身就是漏洞。
fn immediate_variable(name: &str) -> String {
    let makefile = makefile();
    let prefix = format!("{name} :=");
    let line = makefile
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| {
            panic!(
                "Makefile 里找不到 `{prefix}`。这个变量是产物核对的取值域，\
                 必须用 `:=` 立即展开赋值——`?=` 允许环境变量覆盖，等于门禁可被削弱"
            )
        });
    line[prefix.len()..].trim().to_string()
}

/// 抽出 `bundle` 目标的配方正文（到下一个顶格的目标定义为止）。
///
/// 只判配方而不判整份文件，是因为文件里的注释同样会提到 `--no-sign`；
/// 用 `Makefile.contains("--no-sign")` 判断会在配方丢掉这个开关时依然为真——
/// 那正是「解释一条规则的文字命中这条规则」这类假通过。
fn bundle_recipe() -> String {
    let makefile = makefile();
    let mut lines = makefile
        .lines()
        .skip_while(|line| !line.starts_with("bundle:"));
    let header = lines
        .next()
        .expect("Makefile 里必须有 `bundle:` 目标：它是桌面安装包的本机入口");
    let mut recipe = String::from(header);
    recipe.push('\n');
    for line in lines {
        // 配方行以 tab 开头；续行也一样。顶格的非空行意味着下一个目标或变量开始了。
        if !line.starts_with('\t') && !line.trim().is_empty() {
            break;
        }
        recipe.push_str(line);
        recipe.push('\n');
    }
    recipe
}

fn workflow_sources() -> Vec<(PathBuf, String)> {
    let dir = repo_root().join(".github/workflows");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("`.github/workflows` 应当存在") {
        let path = entry.expect("读取 workflow 目录项失败").path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yml" || ext == "yaml")
        {
            let text = read(&path);
            found.push((path, text));
        }
    }
    assert!(
        !found.is_empty(),
        "`.github/workflows` 下没有任何工作流文件"
    );
    found
}

/// 只解析 `build-binaries.matrix.include`，不让文件头解释性注释里的 target 名称冒充矩阵条目。
fn release_matrix_targets() -> Vec<(String, String, bool)> {
    let workflow = read(&repo_root().join(".github/workflows/release-please.yml"));
    let matrix = workflow
        .split_once("  build-binaries:")
        .expect("release workflow 必须有 build-binaries job")
        .1
        .split_once("    steps:")
        .expect("build-binaries 必须有 steps")
        .0;

    let mut targets = Vec::new();
    let mut current: Option<(String, Option<String>, Option<bool>)> = None;
    for line in matrix.lines() {
        let trimmed = line.trim();
        if let Some(target) = trimmed.strip_prefix("- target: ") {
            if let Some((target, os, use_zigbuild)) = current.take() {
                targets.push((
                    target,
                    os.expect("每个发布目标必须声明 os"),
                    use_zigbuild.expect("每个发布目标必须声明 use_zigbuild"),
                ));
            }
            current = Some((target.to_string(), None, None));
        } else if let Some((_, os, _)) = current.as_mut()
            && let Some(value) = trimmed.strip_prefix("os: ")
        {
            *os = Some(value.to_string());
        }
        if let Some((_, _, use_zigbuild)) = current.as_mut()
            && let Some(value) = trimmed.strip_prefix("use_zigbuild: ")
        {
            *use_zigbuild = Some(match value {
                "true" => true,
                "false" => false,
                other => panic!("use_zigbuild 只能是 true/false，实际是 {other}"),
            });
        }
    }
    if let Some((target, os, use_zigbuild)) = current {
        targets.push((
            target,
            os.expect("每个发布目标必须声明 os"),
            use_zigbuild.expect("每个发布目标必须声明 use_zigbuild"),
        ));
    }
    targets
}

#[test]
fn release_matrix_is_locked_to_six_native_platform_targets() {
    assert_eq!(
        REQUIRED_RELEASE_TARGETS,
        [
            ("x86_64-unknown-linux-musl", "ubuntu-24.04", true),
            ("aarch64-unknown-linux-musl", "ubuntu-24.04-arm", true),
            ("x86_64-apple-darwin", "macos-15-intel", false),
            ("aarch64-apple-darwin", "macos-14", false),
            ("x86_64-pc-windows-msvc", "windows-latest", false),
            ("aarch64-pc-windows-msvc", "windows-11-arm", false),
        ],
        "发布矩阵由方案冻结为六目标；要调整先改方案，不要改这条断言"
    );

    let actual = release_matrix_targets();
    let expected: Vec<_> = REQUIRED_RELEASE_TARGETS
        .iter()
        .map(|(target, os, zig)| (target.to_string(), os.to_string(), *zig))
        .collect();
    assert_eq!(
        actual, expected,
        "release workflow 必须精确发布两个 Linux musl、两个 macOS 和两个 Windows 目标；\
         Linux 两腿用 cargo-zigbuild，ARM Windows 在 windows-11-arm 原生构建"
    );
}

#[test]
fn bundle_verification_covers_every_required_artifact_kind() {
    // 取值域自锁：三类都是发布路径上的真实产物，加减一类要连同理由一起改。
    assert_eq!(
        REQUIRED_BUNDLE_KINDS,
        ["deb", "rpm", "appimage"],
        "本机打包必须核对的产物类别被改了。改它要同时说明新的类别集为什么仍然覆盖 \
         Linux 自动更新（updater 只消费 AppImage）"
    );

    let declared = immediate_variable("BUNDLE_KINDS");
    let declared: Vec<&str> = declared.split_whitespace().collect();
    for kind in REQUIRED_BUNDLE_KINDS {
        assert!(
            declared.contains(&kind),
            "Makefile 的 BUNDLE_KINDS 少了 `{kind}`，实际是 `{declared:?}`。\
             这个列表就是产物核对的范围：从里面删掉一类，`make bundle` 会在那一类\
             打不出来时照样报「安装包齐备」"
        );
    }
    assert_eq!(
        declared.len(),
        REQUIRED_BUNDLE_KINDS.len(),
        "BUNDLE_KINDS 多出了未声明的类别 `{declared:?}`。新增类别要先在本断言里立条目，\
         否则配方的 `case` 分支会以「未知产物类别」中止"
    );
}

#[test]
fn bundle_recipe_runs_without_a_release_signing_key() {
    let recipe = bundle_recipe();
    assert!(
        recipe.contains("$(CARGO) tauri build --debug --no-sign -v"),
        "`bundle` 配方必须是 `cargo tauri build --debug --no-sign -v`，实际配方：\n{recipe}\n\
         `--no-sign` 不是省事：`plugins.updater.pubkey` 存在时，缺 TAURI_SIGNING_PRIVATE_KEY \
         会让整条命令在三个安装包**都已产出之后**退出 1，报错却像打包坏了。\
         `-v` 也不是啰嗦：tauri-bundler 默认 log_level=Error 会吞掉 linuxdeploy 的 stderr，\
         只留一句 `failed to run linuxdeploy`"
    );
}

#[test]
fn bundle_recipe_fails_when_an_artifact_is_missing() {
    let recipe = bundle_recipe();
    assert!(
        recipe.contains("status=1"),
        "`bundle` 配方必须在某一类产物缺失时置失败标记，实际配方：\n{recipe}"
    );
    assert!(
        recipe.contains("exit 1"),
        "`bundle` 配方必须在产物不齐时非零退出——只打印一行警告等于没有门禁：\n{recipe}"
    );
    assert!(
        recipe.contains("$(BUNDLE_MIN_FREE_MB)"),
        "`bundle` 配方必须先判可用磁盘。AppImage 阶段要先铺开一个未压缩 AppDir，\
         空间不足时 linuxdeploy 的失败只显示成 `failed to run linuxdeploy`，\
         一个字都不提 ENOSPC——这正是本仓库已经踩过一次的那种误导性失败"
    );
}

#[test]
fn linux_auto_update_requires_the_appimage_artifact() {
    let updater = read(&crate_dir().join("src/updater.rs"));
    assert!(
        updater.contains(LINUX_UPDATE_TARGET),
        "`updater.rs` 不再声明 `{LINUX_UPDATE_TARGET}`。若真要放弃 Linux 自动更新，\
         那是一次发布契约变更，本断言与 BUNDLE_KINDS 要一起改"
    );
    assert!(
        REQUIRED_BUNDLE_KINDS.contains(&LINUX_UPDATER_ARTIFACT),
        "声明了 `{LINUX_UPDATE_TARGET}` 却不要求 `{LINUX_UPDATER_ARTIFACT}` 产物：\
         Linux updater 只消费 AppImage，`.deb` 不能自动更新，这两者不能各自成立"
    );

    // `targets: "all"` 是 AppImage 在 Linux 上被打出来的前提。改成显式数组也行，
    // 但那份数组必须点名 appimage，否则上面的要求在配置层就已经落空。
    let config: Value = serde_json::from_str(&read(&crate_dir().join("tauri.conf.json")))
        .expect("tauri.conf.json 必须是合法 JSON");
    let targets = &config["bundle"]["targets"];
    let covers_appimage = match targets {
        Value::String(value) => value == "all",
        Value::Array(values) => values
            .iter()
            .any(|value| value.as_str() == Some(LINUX_UPDATER_ARTIFACT)),
        other => panic!("bundle.targets 只能是 \"all\" 或数组，实际是 {other}"),
    };
    assert!(
        covers_appimage,
        "bundle.targets 是 {targets}，不会在 Linux 上产出 AppImage。\
         `make bundle` 的产物核对会因此在一个配置问题上报「缺少 appimage 产物」，\
         而真正该改的是这里"
    );
}

#[test]
fn the_release_path_still_signs_and_never_borrows_the_local_no_sign_switch() {
    let mut signing_workflows = 0usize;
    for (path, text) in workflow_sources() {
        assert!(
            !text.contains("--no-sign"),
            "{} 出现了 `--no-sign`。它只属于本机入口：发布产物不签名就等于所有客户端\
             都拒绝这次更新，而这种失败在构建期是完全看不出来的",
            path.display()
        );
        if text.contains("TAURI_SIGNING_PRIVATE_KEY") {
            signing_workflows += 1;
            assert!(
                text.contains("updater 签名不可关闭"),
                "{} 用到了签名密钥，却没有「缺 key 就硬失败」那一段。\
                 缺 key 时 tauri 只是不生成 .sig 并继续，于是会产出一套结构完整、\
                 却被每个客户端拒绝的安装包",
                path.display()
            );
        }
    }
    assert_eq!(
        signing_workflows, 1,
        "恰好应有一个工作流持有 updater 签名密钥；多处持有等于多处可漂移"
    );
}
