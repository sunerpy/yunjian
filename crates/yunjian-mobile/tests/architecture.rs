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
fn undetermined_verdict_builds_neither_binding_branch() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("读取 mobile manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("解析 mobile manifest");
    let features = parsed.get("features").and_then(toml::Value::as_table);
    assert!(
        features.is_none_or(|table| !table.contains_key("uniffi") && !table.contains_key("tauri")),
        "undetermined 时不得声明任何 binding feature"
    );
    let dependencies = parsed
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("mobile manifest 应有 dependencies 表");
    for shell in ["uniffi", "tauri"] {
        assert!(
            !dependencies.contains_key(shell),
            "undetermined 时不得依赖 {shell}"
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
