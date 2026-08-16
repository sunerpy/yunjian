//! 分发许可门禁：带 `voice` 的发布产物必须随带许可文本。
//!
//! # 这组断言是为哪一个真实缺口写的
//!
//! 项目 2026-08-10 的许可裁决把 `voice` 特性开关本身当作许可边界：默认构建纯 MIT，
//! 开启 `voice` 后产物静态包含 GPL-3.0 的 `espeak-ng`，于是**结合作品整体落在 GPL-3.0**。
//! 这条结论被写进了 `LICENSES.md`、`docs/readme/LICENSES.md` 与 README。
//!
//! **但义务没有随产物走。** CLI 发布归档用 `--features voice,mcp` 构建，归档里却只有
//! 可执行文件与语音运行库，**没有任何许可文本**；整份发布工作流搜 `LICENSE` 零命中。
//! GPL-3.0 要求随分发物保留许可与版权声明——源码在 GitHub 上可得这一条满足，
//! 「分发一个裸二进制、归档里连许可原文都没有」不满足声明义务。
//!
//! **写下义务 ≠ 履行义务。** 这组断言把「履行」变成可执行判据。
//!
//! # 两层，各守一件事，不要混
//!
//! 1. **本文件（`make ci` / CI Success 会跑）**：`packaging/licenses/` 这份载荷本身
//!    完整且正确——三份文件齐全、MIT 副本与仓库根 `LICENSE` 逐字节相同、GPL 原文是
//!    记录的那一份且真的是 GPL-3.0、署名说明点到 espeak-ng 与源码去处。
//!    另外断言这个目录**只装可分发文件**，因为工作流是整目录拷贝，多一个 README
//!    就会漏进用户手上的归档。
//! 2. **发布工作流（打 tag 时跑）**：把归档**解开**，逐个核对
//!    `packaging/licenses/` 里的每个文件都在 `licenses/` 下。那是对**真实产物字节**的
//!    判断，不是对工作流文本的判断。
//!
//! **两层为什么串得起来**：`build-binaries` 作业挂在 release-please 的
//! 「等 CI Success」门之后（见 workflow 的 `等待本提交的 CI Success`），所以第二层跑之前
//! 第一层必然已经绿过。第一层保证目录非空且正确，第二层保证归档镜像了这个目录。
//! 把工作流里那行拷贝删掉 → 第二层红；把载荷目录清空 → 第一层红。
//!
//! # 为什么还要扫工作流，以及它守不住什么
//!
//! 下面第四、五条断言按**步骤**（而不是整文件）在 workflow 里定位那两个钩子。
//! 这与 `yunjian-app/tests/bundle_targets.rs` 的做法同形：**按名字切出单个步骤再判**，
//! 于是文件头解释这条规则的注释无法冒充命中——「解释一条规则的文字命中这条规则」
//! 是这类门禁最典型的假通过。
//!
//! **诚实的边界**：这两条耦合在 `PAYLOAD_DIR` 这个字面路径上。改目录名会让它变红
//! （那是想要的，改名就该同步改这里）；但如果有人**同时**删掉工作流里的拷贝步骤与
//! 校验步骤，Rust 侧只能靠这两条发现，而它们本身就是被删的那两条的镜像。
//! 用 Rust 去证明 YAML 的行为是做不到的——**真正承重的是工作流里那道运行时校验**，
//! 它拆开归档看字节。本文件的职责是「载荷正确」加「钩子还在」，不是「YAML 一定被执行」。

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 随分发件的许可载荷目录，仓库相对路径。
///
/// 取值就是执行机制：工作流按这个目录整体拷贝，本文件按这个目录判完整性。
/// 改这里必须同时改 `.github/workflows/release-please.yml` 的两处，否则下面两条断言变红。
const PAYLOAD_DIR: &str = "packaging/licenses";

/// 载荷必须且只能包含这三个文件。
///
/// **「只能」和「必须」一样重要**：工作流是整目录拷贝，往这里放一份 `README.md`
/// 之类的内部说明，它会跟着进用户手上的归档。说明文字归 `LICENSES.md`。
const PAYLOAD_FILES: [&str; 3] = ["LICENSE-GPL-3.0.txt", "LICENSE-MIT.txt", "NOTICE.md"];

/// 随包 GPL-3.0 原文的 SHA-256。
///
/// 这份字节取自 `csukuangfj/espeak-ng` 的 `COPYING`——**即真正约束我们所再分发的那份
/// 代码的那个文件**，而不是随便一份 GPL-3.0 抄本。锁住摘要是为了防一类具体的漂移：
/// 把 674 行原文换成一句「见 GNU 官网」，归档里依然有个名字对得上的文件，
/// 而声明义务已经不再被满足。
///
/// 要改这里，先确认换文本是有意的。
const GPL3_SHA256: &str = "8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903";

/// GPL-3.0 原文的标题行。摘要能防替换，这条防「摘要和文件一起被换成另一个许可」。
const GPL3_TITLE: &str = "GNU GENERAL PUBLIC LICENSE";

/// 归档内许可文本的落点，相对归档根。
const ARCHIVE_SUBDIR: &str = "licenses";

/// 仓库根。本 crate 的 manifest 目录是 `<root>/crates/yunjian-cli`。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根应当存在")
}

fn read_bytes(relative: &str) -> Vec<u8> {
    let path = repo_root().join(relative);
    std::fs::read(&path).unwrap_or_else(|error| panic!("读不到 {}：{error}", path.display()))
}

fn read_text(relative: &str) -> String {
    String::from_utf8(read_bytes(relative)).expect("许可载荷必须是 UTF-8")
}

fn release_workflow() -> String {
    String::from_utf8(read_bytes(".github/workflows/release-please.yml"))
        .expect("发布工作流必须是 UTF-8")
}

/// 切出 `build-binaries` 作业里某个命名步骤的**可执行正文**，注释行一律剔除。
///
/// 两道收窄都是必要的，各挡一种假通过：
///
/// - **只判单个步骤**，不判整份文件——否则文件头解释这条规则的文字就能顶替真实的拷贝。
/// - **剔掉注释行**，因为解释这条规则的注释就写在被守护的那几行旁边。
///   这不是假想的风险：本测试的第一版没剔注释，注入验证时把 zip 那段校验代码删掉、
///   注释留着，**断言照绿**。`bundle_targets.rs` 早把这类「解释一条规则的文字命中
///   这条规则」记成了教训，第一版还是踩了。
fn build_step(name: &str) -> String {
    let workflow = release_workflow();
    let job = workflow
        .split_once("  build-binaries:")
        .expect("发布工作流必须有 build-binaries 作业")
        .1;
    let header = format!("      - name: {name}");
    let body = job
        .split_once(header.as_str())
        .unwrap_or_else(|| {
            panic!(
                "build-binaries 作业里找不到步骤「{name}」。\
                 它是随分发件带许可文本的钩子之一；改名请同步改本测试的常量"
            )
        })
        .1;
    // 下一个同级步骤以同样的缩进开始；到那里为止就是本步骤的全部。
    let step = match body.split_once("\n      - ") {
        Some((step, _)) => step,
        None => body,
    };
    step.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 一、载荷本身：三份文件齐全，且没有多余文件
// ---------------------------------------------------------------------------

#[test]
fn the_license_payload_holds_exactly_the_files_that_must_ship() {
    let dir = repo_root().join(PAYLOAD_DIR);
    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{} 必须存在：{error}", dir.display()))
        .map(|entry| {
            let entry = entry.expect("读目录项");
            assert!(
                entry.file_type().expect("读文件类型").is_file(),
                "{}/{} 不是普通文件。载荷会被整目录拷进归档，子目录会让归档结构不可预期",
                PAYLOAD_DIR,
                entry.file_name().to_string_lossy()
            );
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect();
    found.sort();

    let mut expected: Vec<String> = PAYLOAD_FILES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    expected.sort();

    assert_eq!(
        found, expected,
        "{PAYLOAD_DIR}/ 的内容必须与 PAYLOAD_FILES 完全一致。\
         少一个 → 分发件缺声明；多一个 → 那个文件会跟着进用户手上的归档"
    );
}

// ---------------------------------------------------------------------------
// 二、MIT 副本不许与仓库根 LICENSE 漂开
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_mit_copy_is_byte_identical_to_the_repository_license() {
    let shipped = read_bytes(&format!("{PAYLOAD_DIR}/LICENSE-MIT.txt"));
    let canonical = read_bytes("LICENSE");
    assert_eq!(
        shipped, canonical,
        "{PAYLOAD_DIR}/LICENSE-MIT.txt 必须与仓库根 LICENSE 逐字节相同。\
         两份漂开意味着「分发时给出的许可」与「项目声明的许可」是两份东西"
    );
}

// ---------------------------------------------------------------------------
// 三、GPL-3.0 原文是记录的那一份，且真的是 GPL-3.0
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_gpl_text_is_the_recorded_upstream_copying_file() {
    let bytes = read_bytes(&format!("{PAYLOAD_DIR}/LICENSE-GPL-3.0.txt"));
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest, GPL3_SHA256,
        "随包 GPL-3.0 原文的摘要不符。它必须是 csukuangfj/espeak-ng 的 COPYING 原样副本——\
         那才是约束我们再分发的那份代码的文件"
    );

    let text = String::from_utf8(bytes).expect("GPL-3.0 原文必须是 UTF-8");
    assert!(
        text.contains(GPL3_TITLE),
        "随包 GPL-3.0 文件里没有「{GPL3_TITLE}」标题。摘要防替换，这条防摘要与文件被一起换掉"
    );
    assert!(
        text.contains("Version 3, 29 June 2007"),
        "随包 GPL 文件不是第 3 版。voice 产物的许可边界是 GPL-3.0，别的版本不满足义务"
    );
}

// ---------------------------------------------------------------------------
// 四、署名说明必须点到义务的每一个要件
// ---------------------------------------------------------------------------

#[test]
fn the_notice_names_the_component_the_license_and_where_to_get_the_source() {
    let notice = read_text(&format!("{PAYLOAD_DIR}/NOTICE.md"));
    // GPL-3.0 的声明义务落到具体要件就是这几样：是什么组件、按什么许可、
    // 源码去哪拿、以及本项目自身的许可。少任何一件，这份 NOTICE 就只是一句空话。
    for required in [
        "espeak-ng",
        "GPL-3.0",
        "https://github.com/csukuangfj/espeak-ng",
        "https://github.com/sunerpy/yunjian",
        "MIT",
        GPL3_SHA256,
    ] {
        assert!(
            notice.contains(required),
            "{PAYLOAD_DIR}/NOTICE.md 里没有「{required}」。\
             这份文件是随分发件唯一的声明，缺一个要件就不构成声明"
        );
    }
}

// ---------------------------------------------------------------------------
// 五、工作流的两个钩子还在：暂存时拷入、打包后校验
// ---------------------------------------------------------------------------

#[test]
fn the_release_workflow_stages_the_license_payload_into_the_cli_archive() {
    let staging = build_step("暂存 CLI 与语音运行库");
    assert!(
        staging.contains(PAYLOAD_DIR),
        "发布工作流的暂存步骤没有引用 {PAYLOAD_DIR}。\
         CLI 用 --features voice,mcp 构建，是 GPL-3.0 结合作品；\
         归档里不带许可文本不满足声明义务"
    );
    assert!(
        staging.contains(ARCHIVE_SUBDIR),
        "暂存步骤没有把许可文本放到归档的 {ARCHIVE_SUBDIR}/ 下。\
         落点是打包后校验那一步的判据，两处必须一致"
    );
}

#[test]
fn both_archive_formats_verify_the_license_payload_after_packing() {
    // tar.gz（Linux musl 与 macOS）与 zip（Windows）是两条**独立实现**的打包路径，
    // 一条加了校验另一条没加，等于 Windows 用户拿到的归档不受任何约束。
    for step in ["打包 CLI（tar.gz）", "打包 CLI（zip）"] {
        let body = build_step(step);
        assert!(
            body.contains(PAYLOAD_DIR),
            "步骤「{step}」没有按 {PAYLOAD_DIR} 核对归档内容。\
             这道校验必须解开归档看真实字节——它是唯一能证明许可文本真的进了产物的判据"
        );
        assert!(
            body.contains(ARCHIVE_SUBDIR),
            "步骤「{step}」没有核对归档的 {ARCHIVE_SUBDIR}/ 落点"
        );
    }
}

// ---------------------------------------------------------------------------
// 六、桌面安装包标 MIT，并且真的把 MIT 原文带上
// ---------------------------------------------------------------------------

#[test]
fn the_desktop_bundle_declares_mit_and_ships_the_license_file() {
    let config: serde_json::Value =
        serde_json::from_slice(&read_bytes("crates/yunjian-app/tauri.conf.json"))
            .expect("tauri.conf.json 必须是合法 JSON");
    let bundle = config
        .get("bundle")
        .expect("tauri.conf.json 必须有 bundle 段");

    assert_eq!(
        bundle.get("license").and_then(serde_json::Value::as_str),
        Some("MIT"),
        "桌面安装包必须标 MIT。桌面产物刻意不编译 voice 特性（见 release workflow 的 \
         `cargo tauri build`，它不带 --features voice），因此整体是 MIT 而非 GPL-3.0"
    );
    assert_eq!(
        bundle
            .get("licenseFile")
            .and_then(serde_json::Value::as_str),
        Some("../../LICENSE"),
        "桌面安装包必须指向仓库根 LICENSE。这个字段喂给 dmg / msi / nsis 的许可页"
    );

    // **licenseFile 管不到 deb 与 AppImage**：tauri-bundler 的 `linux/debian.rs` 根本不读
    // 这个字段（实测：该文件里 `license` 只出现在两行 SPDX 文件头注释里）。deb 唯一会
    // 复制的是 `resources`，落到 `/usr/lib/<productName>/`，AppImage 从 deb 的载荷组装，
    // 于是也跟着有。少了这一条，Linux 两种安装包里一个字的许可都没有。
    let resources = bundle
        .get("resources")
        .expect("桌面安装包必须声明 resources：licenseFile 管不到 deb / AppImage");
    assert_eq!(
        resources
            .get("../../LICENSE")
            .and_then(serde_json::Value::as_str),
        Some("LICENSE"),
        "resources 必须把仓库根 LICENSE 映射到 LICENSE。\
         这是 deb 与 AppImage 里唯一能带上许可原文的通路"
    );
}
