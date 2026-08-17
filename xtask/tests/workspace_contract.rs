use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const EXPECTED_WORKSPACE_MEMBERS: [&str; 10] = [
    "crates/yunjian-core",
    "crates/yunjian-corpus",
    "crates/yunjian-ai",
    "crates/yunjian-recite",
    "crates/yunjian-voice",
    "crates/yunjian-mcp",
    "crates/yunjian-cli",
    "xtask",
    "crates/yunjian-app",
    "crates/yunjian-mobile",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("从 xtask/ 推出仓库根目录")
        .to_path_buf()
}

fn parse_manifest(path: &Path) -> toml::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("解析 {} 失败: {error}", path.display()))
}

#[test]
fn workspace_members_match_the_frozen_ten_member_plan_exactly() {
    let manifest = parse_manifest(&repo_root().join("Cargo.toml"));
    let actual: Vec<&str> = manifest["workspace"]["members"]
        .as_array()
        .expect("[workspace].members 必须是数组")
        .iter()
        .map(|member| member.as_str().expect("workspace 成员必须是字符串"))
        .collect();

    assert_eq!(
        actual, EXPECTED_WORKSPACE_MEMBERS,
        "workspace 必须逐项等于冻结方案的 8 个基础成员，再依次追加 yunjian-app 与 yunjian-mobile"
    );
}

#[test]
fn android_spike_builds_from_its_standalone_manifest_into_the_shared_target_dir() {
    let root = repo_root();
    let spike_manifest = parse_manifest(&root.join("crates/yunjian-spike/Cargo.toml"));
    assert!(
        spike_manifest.get("workspace").is_some(),
        "yunjian-spike 必须用自己的 [workspace] 脱离产品 workspace"
    );
    assert!(
        root.join("crates/yunjian-spike/Cargo.lock").is_file(),
        "独立 spike workspace 必须提交自己的 Cargo.lock"
    );

    let buildspec = std::fs::read_to_string(root.join(".aws/buildspec-android-spike.yml"))
        .expect("读取 Android spike buildspec");
    let command = buildspec
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("- "))
        .find(|line| line.starts_with("cargo build ") && line.contains("yunjian-spike"))
        .expect("buildspec 必须包含 yunjian-spike 的 cargo build 命令");

    assert!(
        command.contains("--manifest-path crates/yunjian-spike/Cargo.toml"),
        "spike 已不是产品 workspace 成员，必须按独立 manifest 构建: {command}"
    );
    assert!(
        command.contains("--locked"),
        "独立 spike 构建必须消费已提交的 Cargo.lock: {command}"
    );
    assert!(
        command.contains("--target-dir \"$CODEBUILD_SRC_DIR/target\""),
        "独立 workspace 必须继续把 .so 写到 buildspec 后续步骤读取的根 target/: {command}"
    );
    assert!(
        !command.contains("-p yunjian-spike"),
        "根 workspace 的 -p 选择器不能再用于独立 spike: {command}"
    );
}

// ---------------------------------------------------------------------------
// 分发二进制的 voice rpath 契约
//
// # 这条守的是哪一次真实失败
//
// 开了 voice 的可执行文件链接 `libsherpa-onnx-c-api.so`，若二进制自己没有 rpath，
// 即使 `.so` 就躺在同一目录里也会在**进入 main 之前**报
// `cannot open shared object file` 而起不来。而 `cargo test` 会自己注入
// `LD_LIBRARY_PATH`，于是**整套测试全绿、发布产物却跑不起来**——`yunjian-cli` 实测过一次，
// `yunjian-app` 直到 F3 复核才被发现漏了同一段（`readelf -d` 无 RUNPATH，直接运行即报错）。
//
// 这是「测试环境注入的便利掩盖了发布环境的缺失」这一类问题里最隐蔽的一种：它在测试里
// 完全不可见，不像占位标记或空 verdict 那样留下痕迹。所以守它的断言不能是另一条测试
// （测试环境永远有 `LD_LIBRARY_PATH`），只能是**对链接参数本身的核对**。
//
// # 为什么必须逐包重复而不能抽公因子
//
// `cargo:rustc-link-arg` 不会从 rlib 依赖传递到最终链接步骤，所以 rpath 必须由
// **二进制所属的包**发；`yunjian-voice` 替它们发是无效的。两份 build.rs 因此是被迫重复的，
// 这条断言的作用就是让重复不许漂移，也不许其中一份被删。
// ---------------------------------------------------------------------------

/// 一个 crate 的 feature 值里出现这个串，就意味着它能把原生语音链进自己的产物。
const VOICE_FEATURE_PATH: &str = "yunjian-voice/voice";

/// 必须发 rpath 的成员：它们的可执行文件会被分发到用户机器上，那里没有 `cargo`
/// 也没有 `LD_LIBRARY_PATH`。
const RPATH_REQUIRED_MEMBERS: [&str; 2] = ["crates/yunjian-cli", "crates/yunjian-app"];

/// 能开 voice 但**不必**发 rpath 的成员，连同免除理由。
///
/// 免除写在这里而不是靠「没人想起来」，是为了让下面那条集合相等断言能成立：
/// 新增一个能开 voice 的二进制包时，它既不在必需集也不在免除集，断言立刻变红并逼人做决定。
const RPATH_EXCUSED_MEMBERS: [(&str, &str); 2] = [
    (
        "xtask",
        "只经 `cargo run -p xtask` 运行，而 cargo 会注入 LD_LIBRARY_PATH；它不进任何分发物",
    ),
    (
        "crates/yunjian-mobile",
        "唯一二进制是 `uniffi-bindgen` 代码生成器，只在生成脚本里跑；移动端真正被加载的是 \
         cdylib，由 System.loadLibrary 从 APK 的 jniLibs 解析，不经 rpath",
    ),
];

/// 每个必需成员的 build.rs 都要发的链接参数，按目标 OS 分。
///
/// 取值即执行机制：删掉任一行，下面的断言会点名是哪个包缺了哪条。
const REQUIRED_LINK_ARGS: [&str; 2] = [
    "cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN",
    "cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path",
];

/// 去掉 Rust 源码的行注释，但不动字符串字面量里的 `//`。
///
/// **不剔注释这条断言就是假的。** 两份 build.rs 的文件头都在散文里写着
/// `cargo:rustc-link-arg`（解释「为什么必须由二进制所属的包发」），于是一条朴素的
/// `source.contains("cargo:rustc-link-arg")` 在 `println!` 被整行删掉之后**依然为真**。
/// 这正是本仓库已经栽过的「解释一条规则的文字命中这条规则」，`bundle_targets.rs` 的
/// `bundle_recipe` 为同一个原因只取配方而不取整份 Makefile。
fn strip_line_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let bytes = line.as_bytes();
        let mut cut = line.len();
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                cut = index;
                break;
            }
            index += 1;
        }
        // `cut` 只落在 ASCII 的 `/` 上或行尾，因此这个切片不会切断多字节字符。
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// 该成员的产物里有可执行文件吗。
fn produces_a_binary(root: &Path, member: &str, manifest: &toml::Value) -> bool {
    manifest.get("bin").is_some() || root.join(member).join("src/main.rs").is_file()
}

/// 该成员能把 `yunjian-voice/voice` 链进自己的产物吗。
fn can_enable_voice(manifest: &toml::Value) -> bool {
    let Some(features) = manifest.get("features").and_then(toml::Value::as_table) else {
        return false;
    };
    features.values().any(|value| {
        value.as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.as_str() == Some(VOICE_FEATURE_PATH))
        })
    })
}

#[test]
fn the_comment_stripper_does_not_let_prose_impersonate_an_emission() {
    // 这段就是两份 build.rs 文件头的形状：散文里出现了标记，代码里什么也没发。
    let prose_only = "//! rpath 必须由 `cargo:rustc-link-arg-bins` 在二进制所属的包里发。\n\
                      fn main() {}\n";
    assert!(
        prose_only.contains("cargo:rustc-link-arg-bins"),
        "前提：注释原文确实命中标记，否则这条反例没有意义"
    );
    assert!(
        !strip_line_comments(prose_only).contains("cargo:rustc-link-arg-bins"),
        "剔注释之后散文不得再命中标记——否则下面的 rpath 断言在发射被删掉时仍会通过"
    );

    // 字符串字面量里的 `//` 不许被当成注释起点。
    let with_url = "let a = \"https://example.invalid/x\"; // 说明\n";
    let stripped = strip_line_comments(with_url);
    assert!(
        stripped.contains("https://example.invalid/x"),
        "字符串里的 `//` 被误判成注释：{stripped}"
    );
    assert!(!stripped.contains("说明"), "行尾注释未被剔除：{stripped}");
}

#[test]
fn every_voice_capable_binary_crate_is_either_required_to_emit_rpath_or_excused() {
    let root = repo_root();
    let discovered: BTreeSet<String> = EXPECTED_WORKSPACE_MEMBERS
        .iter()
        .filter(|member| {
            let manifest = parse_manifest(&root.join(member).join("Cargo.toml"));
            produces_a_binary(&root, member, &manifest) && can_enable_voice(&manifest)
        })
        .map(|member| (*member).to_string())
        .collect();

    let classified: BTreeSet<String> = RPATH_REQUIRED_MEMBERS
        .iter()
        .copied()
        .chain(RPATH_EXCUSED_MEMBERS.iter().map(|(member, _)| *member))
        .map(str::to_string)
        .collect();

    assert_eq!(
        discovered, classified,
        "有成员能把 {VOICE_FEATURE_PATH} 链进自己的可执行文件，却既不在必需集也不在免除集。\
         新增这样一个包时必须做一次决定：产物会被分发就补 rpath（照 \
         crates/yunjian-cli/build.rs），只在 cargo 下运行就登记免除理由。\
         漏做这个决定的代价是产物在用户机器上以 `cannot open shared object file` 起不来，\
         而 cargo test 因为自带 LD_LIBRARY_PATH 一片绿"
    );
}

#[test]
fn every_distributed_voice_binary_emits_an_equivalent_rpath() {
    let root = repo_root();
    for member in RPATH_REQUIRED_MEMBERS {
        let build_script = root.join(member).join("build.rs");
        let source = std::fs::read_to_string(&build_script).unwrap_or_else(|error| {
            panic!(
                "{member} 必须有 build.rs 来发 rpath（读取 {} 失败：{error}）。\
                 链接参数只能由 build script 交给 cargo，没有别的落点",
                build_script.display()
            )
        });
        assert!(
            !source.contains("/*"),
            "{member}/build.rs 出现了块注释，而这里的剔注释只处理行注释；\
             要用块注释就得先把 strip_line_comments 补齐，否则断言会在块注释里假绿"
        );

        let code = strip_line_comments(&source);
        assert!(
            code.contains("CARGO_FEATURE_VOICE"),
            "{member}/build.rs 没有按 CARGO_FEATURE_VOICE 分流。不开 voice 的构建不链接 \
             sherpa-onnx，给它发 rpath 是无来由地改变纯 MIT 产物的链接行为"
        );
        for link_arg in REQUIRED_LINK_ARGS {
            assert!(
                code.contains(link_arg),
                "{member}/build.rs 的代码里没有发 `{link_arg}`。\
                 注意这里判的是**剔掉注释之后**的代码：文件头的散文提到过 \
                 `cargo:rustc-link-arg`，只判原文会在发射整行被删掉时依然通过"
            );
        }
        assert!(
            !code.contains("cargo:rustc-link-arg=") && !code.contains("cargo:rustc-link-arg-bin="),
            "{member}/build.rs 用了非 `-bins` 形式的 link-arg。rpath 要覆盖该包的每个可执行\
             文件，`cargo:rustc-link-arg-bin=<name>` 只作用于点名的那一个，改名或加二进制\
             时会静默漏掉"
        );
    }
}
