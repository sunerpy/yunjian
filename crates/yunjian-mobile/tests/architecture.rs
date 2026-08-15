use std::path::{Path, PathBuf};
use std::process::Command;

const PRE_MOBILE_SHA: &str = "98b008a5062f96ee6036eaf9fef4816b67b2b21f";
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
fn protected_domain_logic_is_byte_identical_to_pre_mobile_commit() {
    let output = Command::new("git")
        .current_dir(workspace_root())
        .args([
            "diff",
            "--exit-code",
            PRE_MOBILE_SHA,
            "--",
            "crates/yunjian-core/src/search/",
            "crates/yunjian-recite/src/score.rs",
            "crates/yunjian-voice/src/session.rs",
        ])
        .output()
        .expect("运行 git diff");
    assert!(
        output.status.success(),
        "移动门面不得修改受保护领域逻辑：\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
