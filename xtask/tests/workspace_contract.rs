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
