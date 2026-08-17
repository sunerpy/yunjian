use std::collections::{BTreeMap, BTreeSet};
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

/// 发一条 rpath 的字符串前缀。**取到它之后剩下的整段就是 rpath 值本身。**
///
/// 这个「取值而非包含」的差别是本节全部意义所在：`contains` 判据下
/// `-rpath,$ORIGIN/app-only-drift` 与 `-rpath,$ORIGIN` 无法区分（后者是前者的前缀），
/// 而按前缀切出值之后它们是两个不同的字符串。
const RPATH_EMISSION_PREFIX: &str = "cargo:rustc-link-arg-bins=-Wl,-rpath,";

/// 两份 build script 必须发出的**规范 rpath 计划**：目标 OS → 该 OS 下发出的 rpath。
///
/// 取值即执行机制。这张表同时锁住三件事，任何一件漂移都点名变红：
///
/// 1. 每个 OS 拿到的**值**（`$ORIGIN` 而不是 `$ORIGIN/lib`）；
/// 2. 哪些 OS 会拿到（`android` 与 `linux` 同待遇，`ios` 与 `macos` 同待遇）；
/// 3. 表里**没有的 OS 一条也不发**——Windows 走兜底分支，而它必须一条也不发：DLL 在 exe
///    同目录被搜索，`-Wl,-rpath` 对 MSVC 链接器根本不是合法参数。
const EXPECTED_RPATH_PLAN: [(&str, &str); 4] = [
    ("linux", "$ORIGIN"),
    ("android", "$ORIGIN"),
    ("macos", "@loader_path"),
    ("ios", "@loader_path"),
];

/// 一份 build script 的 rpath 计划：目标 OS → 该 OS 下会被发出的 rpath 集合。
///
/// 只有真的发了东西的 OS 才会成为键，所以「发射跑到兜底分支里去了」会表现为多出一个
/// `_` 键，而不是静默通过。
type RpathPlan = BTreeMap<String, BTreeSet<String>>;

fn expected_rpath_plan() -> RpathPlan {
    let mut plan = RpathPlan::new();
    for (os, rpath) in EXPECTED_RPATH_PLAN {
        plan.entry(os.to_owned())
            .or_default()
            .insert(rpath.to_owned());
    }
    plan
}

/// 从**已剔注释**的 build script 代码里解析出它的 rpath 计划。
///
/// # 为什么必须解析而不能只查子串包含
///
/// 这条断言叫「等价」，而在 2026-08-18 之前它做的事是「两侧各自含某个子串」。实测过六种
/// 单侧漂移在那个判据下全部 1 passed（见
/// `the_rpath_plan_parser_tells_a_single_sided_drift_from_the_canonical_source` 里逐条列出的
/// 六种），其中最坏的一种是**把两个平台的 rpath 整体对调**：Linux 拿到 `@loader_path`
/// （ld.so 不认）、macOS 拿到 `$ORIGIN`（dyld 不认），两个平台的发布产物都退回到
/// 「链接了 .so 却没有可用 rpath」这个原始故障，而守卫一声不响。
///
/// # 解析不出来的形状一律判红
///
/// 这个解析器只认「`match` 的分支模式是字符串字面量或 `_`、分支体用花括号界定、rpath 由
/// 字面量 `println!` 发出」这一种形状。换成 `format!` 拼串、抽成辅助函数、加分支守卫，
/// 都会让它解析不到发射或直接 panic——**方向是红，不是绿**。这是刻意的：一个能被重写绕过
/// 的守卫等于没有守卫。
fn parse_rpath_plan(member: &str, code: &str) -> RpathPlan {
    let mut plan = RpathPlan::new();
    let mut attributed = 0usize;
    // 当前所在的 match 分支模式；`None` 表示不在任何分支体内。
    let mut arm: Option<Vec<String>> = None;
    // 当前分支体内还没闭合的花括号层数。
    let mut depth = 0usize;

    for (index, line) in code.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim();

        if depth == 0 {
            if let Some((head, tail)) = trimmed.split_once("=>") {
                let patterns = parse_arm_patterns(member, number, head);
                let opened = tail.matches('{').count();
                let closed = tail.matches('}').count();
                assert!(
                    opened > 0,
                    "{member}/build.rs:{number} 的 match 分支没有用花括号界定分支体。\
                     解析器靠花括号确定「这条发射属于哪个 OS」，无花括号的单表达式分支\
                     无法可靠界定，因此判红而不是猜"
                );
                assert!(
                    opened >= closed,
                    "{member}/build.rs:{number} 的花括号在同一行里闭合多于打开，形状无法解析"
                );
                for rpath in emissions_in(member, number, tail) {
                    attributed += 1;
                    for pattern in &patterns {
                        plan.entry(pattern.clone())
                            .or_default()
                            .insert(rpath.clone());
                    }
                }
                depth = opened - closed;
                arm = if depth == 0 { None } else { Some(patterns) };
                continue;
            }
            assert!(
                !trimmed.contains(RPATH_EMISSION_PREFIX),
                "{member}/build.rs:{number} 在 match 分支之外发 rpath，无法把它归属到具体 \
                 目标 OS。无条件发射会让 Windows 也拿到 `-Wl,-rpath`，那对 MSVC 链接器不是\
                 合法参数"
            );
            continue;
        }

        let patterns = arm.clone().unwrap_or_else(|| {
            panic!("{member}/build.rs:{number} 解析器状态不一致：在分支体内却没有模式")
        });
        for rpath in emissions_in(member, number, trimmed) {
            attributed += 1;
            for pattern in &patterns {
                plan.entry(pattern.clone())
                    .or_default()
                    .insert(rpath.clone());
            }
        }
        depth = (depth + trimmed.matches('{').count()).saturating_sub(trimmed.matches('}').count());
        if depth == 0 {
            arm = None;
        }
    }

    assert_eq!(
        depth, 0,
        "{member}/build.rs 的花括号没有闭合，解析器与源码形状不匹配"
    );
    // 兜底自检：解析器自己漏掉一条发射，会让下面的计划比对在残缺数据上通过。
    assert_eq!(
        code.matches(RPATH_EMISSION_PREFIX).count(),
        attributed,
        "{member}/build.rs 里有 rpath 发射没有被归属到任何 match 分支。\
         解析器与源码形状已经不匹配，此时的计划比对是在残缺数据上做的，不可采信"
    );
    plan
}

/// `"linux" | "android"` → `["linux", "android"]`；`_` → `["_"]`。
fn parse_arm_patterns(member: &str, number: usize, head: &str) -> Vec<String> {
    let head = head.trim();
    assert!(
        !head.contains(" if "),
        "{member}/build.rs:{number} 的 match 分支带守卫。带守卫时「哪个 OS 拿到哪条 rpath」\
         不再由模式决定，解析器无从判定，因此判红"
    );
    if head == "_" {
        return vec!["_".to_owned()];
    }
    let patterns: Vec<String> = head
        .split('|')
        .map(|part| {
            let part = part.trim();
            let literal = part
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or_else(|| {
                    panic!(
                        "{member}/build.rs:{number} 的 match 模式 `{part}` 不是字符串字面量\
                         也不是 `_`。这里比对的是 CARGO_CFG_TARGET_OS 的取值，只认这两种形状"
                    )
                });
            assert!(
                !literal.contains('"'),
                "{member}/build.rs:{number} 的 match 模式 `{part}` 形状异常"
            );
            literal.to_owned()
        })
        .collect();
    assert!(
        !patterns.is_empty(),
        "{member}/build.rs:{number} 的 match 分支没有解析出任何模式"
    );
    patterns
}

/// 取出一段文本里每一条 rpath 发射的**值**（前缀之后到字符串字面量收尾之间的整段）。
fn emissions_in(member: &str, number: usize, text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(RPATH_EMISSION_PREFIX) {
        let tail = &rest[at + RPATH_EMISSION_PREFIX.len()..];
        let end = tail.find('"').unwrap_or_else(|| {
            panic!(
                "{member}/build.rs:{number} 的 rpath 发射没有收尾的引号，\
                 无法切出它到底发了哪条路径"
            )
        });
        values.push(tail[..end].to_owned());
        rest = &tail[end..];
    }
    values
}

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

/// 六种**实测过能让旧判据假绿**的单侧漂移，解析器必须逐条把它们与规范形状区分开。
///
/// 这条测试是 2026-08-18 那次注入验证的常驻形态。当时对 `crates/yunjian-app/build.rs`
/// 逐个施加下面六种改法、精确运行
/// `every_distributed_voice_binary_emits_an_equivalent_rpath`，**六次全部 1 passed**——
/// 因为旧判据只问「代码里有没有出现这两个子串」，而这六种改法都保留了那两个子串。
///
/// 每一条都先断言「旧判据确实会放它过去」，再断言「新判据的计划与规范计划不同」。少了前一半，
/// 这组反例就证明不了新判据更强。
#[test]
fn the_rpath_plan_parser_tells_a_single_sided_drift_from_the_canonical_source() {
    let canonical = r#"
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "linux" | "android" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
        }
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path");
        }
        _ => {}
    }
"#;
    assert_eq!(
        parse_rpath_plan("<canonical>", canonical),
        expected_rpath_plan(),
        "前提：规范形状必须解析成规范计划，否则下面的反例比对没有基准"
    );

    let linux_emission = "println!(\"cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN\");";
    let macos_emission = "println!(\"cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path\");";
    let drifts: Vec<(&str, String)> = vec![
        (
            "Linux 的 rpath 加了后缀（旧判据下 `$ORIGIN` 仍是它的前缀）",
            canonical.replace("-rpath,$ORIGIN\"", "-rpath,$ORIGIN/app-only-drift\""),
        ),
        (
            "macOS 的 rpath 加了后缀（`@loader_path` 仍是它的前缀）",
            canonical.replace("-rpath,@loader_path\"", "-rpath,@loader_path/Frameworks\""),
        ),
        (
            "分支丢掉了 android，Android 产物拿不到 rpath",
            canonical.replace("\"linux\" | \"android\"", "\"linux\""),
        ),
        (
            "两个平台的 rpath 整体对调，两侧产物都退回原始故障",
            canonical
                .replace(linux_emission, "__SWAP__")
                .replace(macos_emission, linux_emission)
                .replace("__SWAP__", macos_emission),
        ),
        (
            "Windows 也被发了 rpath，而那对 MSVC 链接器不是合法参数",
            canonical.replace(
                "        _ => {}",
                &format!("        \"windows\" => {{\n            {linux_emission}\n        }}\n        _ => {{}}"),
            ),
        ),
        (
            "Linux 分支多发了一条搜索路径，两侧不再等价",
            canonical.replace(
                linux_emission,
                &format!(
                    "{linux_emission}\n            println!(\"cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib\");"
                ),
            ),
        ),
    ];

    for (label, drifted) in drifts {
        assert_ne!(drifted, canonical, "漂移没有真的改到源码：{label}");
        assert!(
            drifted.contains("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN")
                && drifted.contains("cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path"),
            "前提不成立：这种漂移在旧的「只查子串包含」判据下并不假绿，\
             那它证明不了新判据更强——{label}"
        );
        assert_ne!(
            parse_rpath_plan("<drifted>", &drifted),
            expected_rpath_plan(),
            "解析出的计划与规范计划相同，说明新判据同样漏掉了这种漂移：{label}"
        );
    }
}

/// 解析器碰到自己不认识的形状时必须 panic，而不是解析出一份空计划静默放行。
///
/// 这条守的是「绕过守卫的最省力办法」：把 `println!` 换成拼串或抽成辅助函数，让含
/// `contains` 的判据和只按字面量取值的解析器都找不到发射。方向必须是红。
#[test]
fn the_rpath_plan_parser_refuses_a_shape_it_cannot_attribute() {
    let outside_the_match = r#"
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        _ => {}
    }
"#;
    let panicked = std::panic::catch_unwind(|| parse_rpath_plan("<x>", outside_the_match));
    assert!(
        panicked.is_err(),
        "match 之外的无条件发射必须判红：那会让 Windows 也拿到 `-Wl,-rpath`"
    );

    let guarded = r#"
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        other if other == "linux" => {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
        }
        _ => {}
    }
"#;
    let panicked = std::panic::catch_unwind(|| parse_rpath_plan("<x>", guarded));
    assert!(
        panicked.is_err(),
        "带守卫的分支必须判红：那时哪个 OS 拿到哪条 rpath 不再由模式决定"
    );

    let indirect = r#"
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "linux" | "android" => emit("$ORIGIN"),
        _ => {}
    }
"#;
    let panicked = std::panic::catch_unwind(|| parse_rpath_plan("<x>", indirect));
    assert!(
        panicked.is_err(),
        "无花括号的单表达式分支必须判红：解析器无从界定这条发射属于哪个 OS"
    );
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
    let mut plans: Vec<(&str, RpathPlan)> = Vec::new();
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
        assert!(
            !code.contains("cargo:rustc-link-arg=") && !code.contains("cargo:rustc-link-arg-bin="),
            "{member}/build.rs 用了非 `-bins` 形式的 link-arg。rpath 要覆盖该包的每个可执行\
             文件，`cargo:rustc-link-arg-bin=<name>` 只作用于点名的那一个，改名或加二进制\
             时会静默漏掉"
        );

        // 判的是**剔掉注释之后**的代码：文件头的散文提到过 `cargo:rustc-link-arg`，
        // 只判原文会在发射整行被删掉时依然通过。
        plans.push((member, parse_rpath_plan(member, &code)));
    }

    let (first_member, first_plan) = &plans[0];
    for (member, plan) in &plans[1..] {
        assert_eq!(
            first_plan, plan,
            "{first_member} 与 {member} 对同一个 CARGO_CFG_TARGET_OS 发出的 rpath 不同。\
             这条断言的名字是「等价」，兑现它的正是这一步：两份 build.rs 是被 \
             `cargo:rustc-link-arg` 不经 rlib 传递这件事逼出来的重复，重复必须不许漂移。\
             单侧漂移的代价是那一侧的发布产物在用户机器上以 `cannot open shared object \
             file` 起不来，而 cargo test 因为自带 LD_LIBRARY_PATH 一片绿"
        );
    }

    let expected = expected_rpath_plan();
    for (member, plan) in &plans {
        assert_eq!(
            plan, &expected,
            "{member}/build.rs 的 rpath 计划偏离规范。两侧一起漂到同一个错值时上面那条\
             等价断言是绿的，只有这条能拦住；`_`（含 Windows）必须一条也不发"
        );
    }
}
