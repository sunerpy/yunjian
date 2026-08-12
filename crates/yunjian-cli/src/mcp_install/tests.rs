//! `mcp install` 的进程内单测。
//!
//! 端到端的退出码、stdout 纯净性与真实文件系统行为在 `tests/mcp_install.rs`，
//! 这里只验合并算法、路径推导与程序名选择——它们是纯函数，不该为了验一次字符串
//! 拉起一个子进程。

use super::{
    Action, Client, Dirs, ENTRY, InstallArgs, InstallOut, Refusal, Scope, install, merge, program,
    resolve,
};
use crate::output::Renderable;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// 一次测试的独立目录，析构时整棵删掉。
struct Temp {
    dir: PathBuf,
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Temp {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-install-unit-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        Self { dir }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

fn args(client: Client, path: &Path) -> InstallArgs {
    InstallArgs {
        client,
        global: false,
        path: Some(path.to_path_buf()),
        dry_run: false,
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("结果必须是合法 JSON")
}

// ------------------------------------------------------------------ 两种形态

#[test]
fn a_fresh_claude_config_gets_a_string_command_and_a_separate_args_array() {
    let document = merge(
        Client::Claude,
        Path::new("claude_desktop_config.json"),
        None,
        "yunjian",
    )
    .expect("新建应当成功");
    let value = parse(&document);
    assert_eq!(value["mcpServers"][ENTRY]["command"], json!("yunjian"));
    assert_eq!(value["mcpServers"][ENTRY]["args"], json!(["mcp"]));
    // 顶层键必须是 `mcpServers`，不是 OpenCode 的 `mcp`。
    assert!(
        value.get("mcp").is_none(),
        "Claude 不该出现 `mcp` 键：{document}"
    );
}

#[test]
fn a_fresh_opencode_config_gets_an_array_command_including_the_argument() {
    let document = merge(
        Client::OpenCode,
        Path::new("opencode.json"),
        None,
        "yunjian",
    )
    .expect("新建应当成功");
    let value = parse(&document);
    assert_eq!(value["mcp"][ENTRY]["command"], json!(["yunjian", "mcp"]));
    assert_eq!(value["mcp"][ENTRY]["type"], json!("local"));
    assert_eq!(value["mcp"][ENTRY]["enabled"], json!(true));
    assert!(
        value.get("mcpServers").is_none(),
        "OpenCode 不该出现 `mcpServers` 键：{document}"
    );
}

#[test]
fn the_two_clients_never_share_a_shape() {
    // 这条断言存在的意义是：将来谁把两种形态合并成一份「通用」条目时，它会立刻变红。
    let claude = Client::Claude.entry("yunjian");
    let opencode = Client::OpenCode.entry("yunjian");
    assert_ne!(
        Client::Claude.container_key(),
        Client::OpenCode.container_key()
    );
    assert!(claude["command"].is_string(), "Claude 的 command 是字符串");
    assert!(opencode["command"].is_array(), "OpenCode 的 command 是数组");
    assert!(claude.get("args").is_some() && opencode.get("args").is_none());
    assert!(opencode.get("type").is_some() && claude.get("type").is_none());
}

// ------------------------------------------------------------------ 合并

#[test]
fn an_unrelated_server_entry_survives_the_merge_byte_identically() {
    let existing = "{\n  \"mcp\": {\n    \"other-server\": {\n      \"type\": \"local\",\n      \
                    \"command\": [\"other\", \"serve\"],\n      \"enabled\": true\n    }\n  }\n}\n";
    let document = merge(
        Client::OpenCode,
        Path::new("opencode.json"),
        Some(existing),
        "yunjian",
    )
    .expect("合并应当成功");
    // 逐字节：原文里那一整段必须原样出现在结果里。
    let fragment = "\"other-server\": {\n      \"type\": \"local\",\n      \
                    \"command\": [\"other\", \"serve\"],\n      \"enabled\": true\n    }";
    assert!(document.contains(fragment), "无关条目被改写了：{document}");
    let value = parse(&document);
    assert_eq!(value["mcp"][ENTRY]["type"], json!("local"));
}

#[test]
fn merging_into_a_config_without_the_container_key_keeps_the_other_top_level_keys() {
    let existing = "{\n  \"theme\": \"system\",\n  \"plugin\": [\"a\", \"b\"]\n}\n";
    let document = merge(
        Client::OpenCode,
        Path::new("opencode.json"),
        Some(existing),
        "yunjian",
    )
    .expect("合并应当成功");
    let value = parse(&document);
    assert_eq!(value["theme"], json!("system"));
    assert_eq!(value["plugin"], json!(["a", "b"]));
    assert_eq!(value["mcp"][ENTRY]["command"], json!(["yunjian", "mcp"]));
}

#[test]
fn installing_twice_is_idempotent() {
    let first = merge(Client::Claude, Path::new("c.json"), None, "yunjian").expect("首次");
    let second = merge(Client::Claude, Path::new("c.json"), Some(&first), "yunjian").expect("二次");
    assert_eq!(first, second, "第二次安装不该改变文件内容");
}

#[test]
fn a_stale_entry_is_replaced_rather_than_duplicated() {
    let existing = "{\n  \"mcpServers\": {\n    \"yunjian\": {\n      \"command\": \"/old/yunjian\"\n    }\n  }\n}\n";
    let document = merge(
        Client::Claude,
        Path::new("c.json"),
        Some(existing),
        "/new/yunjian",
    )
    .expect("合并应当成功");
    let value = parse(&document);
    assert_eq!(value["mcpServers"][ENTRY]["command"], json!("/new/yunjian"));
    assert_eq!(value["mcpServers"][ENTRY]["args"], json!(["mcp"]));
    assert!(
        !document.contains("/old/yunjian"),
        "旧路径应当被换掉：{document}"
    );
}

#[test]
fn comments_in_a_json_named_file_are_preserved_and_do_not_break_parsing() {
    // 实测过的真实形态：文件名是 `.json`，里面却有 `//` 注释。
    let existing = "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {\n    \
                    // 这条先留着\n    \"other\": {\"type\": \"local\"}\n  }\n}\n";
    let document = merge(
        Client::OpenCode,
        Path::new("opencode.json"),
        Some(existing),
        "yunjian",
    )
    .expect("含注释的 .json 也必须能合并");
    assert!(document.contains("// 这条先留着"), "注释丢了：{document}");
    assert!(
        document.contains("https://opencode.ai/config.json"),
        "URL 被当注释截断了"
    );
    let value = parse(&super::jsonc::strip_comments(&document));
    assert_eq!(value["mcp"][ENTRY]["enabled"], json!(true));
    assert_eq!(value["mcp"]["other"]["type"], json!("local"));
}

// ------------------------------------------------------------------ 拒绝

#[test]
fn an_invalid_config_is_refused_instead_of_replaced() {
    let path = Path::new("broken.json");
    let refusal = merge(Client::Claude, path, Some("{ \"mcpServers\": "), "yunjian")
        .expect_err("非法 JSON 必须被拒绝");
    assert!(
        matches!(refusal, Refusal::Invalid { .. }),
        "实际是 {refusal:?}"
    );
    let (exit, failure) = refusal.describe();
    assert_eq!(exit, crate::exit::Exit::Usage);
    assert!(
        failure.render().contains("不会替换"),
        "必须说明我们没有替换文件：{}",
        failure.render()
    );
}

#[test]
fn a_non_object_root_is_refused() {
    let refusal = merge(
        Client::Claude,
        Path::new("a.json"),
        Some("[1, 2]"),
        "yunjian",
    )
    .expect_err("数组顶层必须被拒绝");
    assert!(
        matches!(refusal, Refusal::RootNotObject { .. }),
        "实际是 {refusal:?}"
    );
}

#[test]
fn a_container_key_that_is_not_an_object_is_refused_rather_than_clobbered() {
    let refusal = merge(
        Client::OpenCode,
        Path::new("a.json"),
        Some("{\"mcp\": \"disabled\"}"),
        "yunjian",
    )
    .expect_err("容器键不是对象时必须拒绝");
    assert!(
        matches!(refusal, Refusal::ContainerNotObject { key: "mcp", .. }),
        "实际是 {refusal:?}"
    );
}

// ------------------------------------------------------------------ 路径推导

fn dirs(root: &Path) -> Dirs {
    Dirs {
        config: Some(root.join("config")),
        xdg_config: Some(root.join("xdg")),
        cwd: root.join("project"),
    }
}

#[test]
fn claude_always_resolves_to_the_user_level_file() {
    let root = Path::new("/tmp/yunjian-resolve");
    let (path, scope) = resolve(Client::Claude, false, None, &dirs(root)).expect("推导成功");
    assert_eq!(
        path,
        root.join("config")
            .join("Claude")
            .join("claude_desktop_config.json")
    );
    assert_eq!(scope, Scope::Global);
    // Claude 没有项目级配置，`--global` 不改变目标。
    let (with_global, _) = resolve(Client::Claude, true, None, &dirs(root)).expect("推导成功");
    assert_eq!(with_global, path);
    assert!(!Client::Claude.has_project_scope());
}

#[test]
fn opencode_defaults_to_the_project_file_and_global_switches_to_xdg() {
    let root = Path::new("/tmp/yunjian-resolve");
    let (project, scope) = resolve(Client::OpenCode, false, None, &dirs(root)).expect("推导成功");
    assert_eq!(project, root.join("project").join("opencode.json"));
    assert_eq!(scope, Scope::Project);

    let (global, scope) = resolve(Client::OpenCode, true, None, &dirs(root)).expect("推导成功");
    // XDG 而不是平台配置目录：OpenCode 在 macOS 上也读 `~/.config/opencode`。
    assert_eq!(
        global,
        root.join("xdg").join("opencode").join("opencode.json")
    );
    assert_eq!(scope, Scope::Global);
}

#[test]
fn an_existing_jsonc_project_file_is_preferred_over_creating_a_json_one() {
    let temp = Temp::new();
    let project = temp.file("project");
    std::fs::create_dir_all(&project).expect("建项目目录");
    std::fs::write(project.join("opencode.jsonc"), "{}").expect("写 jsonc");
    let dirs = Dirs {
        config: Some(temp.file("config")),
        xdg_config: Some(temp.file("xdg")),
        cwd: project.clone(),
    };
    let (path, _) = resolve(Client::OpenCode, false, None, &dirs).expect("推导成功");
    assert_eq!(path, project.join("opencode.jsonc"));
}

#[test]
fn an_explicit_path_overrides_every_derivation() {
    let root = Path::new("/tmp/yunjian-resolve");
    let explicit = Path::new("/tmp/somewhere/else.json");
    let (path, scope) =
        resolve(Client::Claude, true, Some(explicit), &dirs(root)).expect("推导成功");
    assert_eq!(path, explicit);
    assert_eq!(scope, Scope::Explicit);
}

#[test]
fn a_missing_platform_directory_is_reported_with_an_actionable_hint() {
    let dirs = Dirs {
        config: None,
        xdg_config: None,
        cwd: PathBuf::from("/tmp"),
    };
    let refusal = resolve(Client::Claude, false, None, &dirs).expect_err("取不到目录必须报错");
    let (_, failure) = refusal.describe();
    assert!(
        failure.render().contains("--path"),
        "必须给出可执行的下一步：{}",
        failure.render()
    );
}

// ------------------------------------------------------------------ 程序名

#[test]
fn a_yunjian_on_path_is_written_as_the_bare_name() {
    let temp = Temp::new();
    let exe = temp.file("yunjian");
    std::fs::write(&exe, b"binary").expect("放一个假二进制");
    let path_var = std::ffi::OsString::from(temp.dir.display().to_string());
    // 在 PATH 上：写裸名，配置因此在换机器、换安装位置后仍然成立。
    assert_eq!(program(Some(&exe), Some(&path_var)), "yunjian");
}

#[test]
fn an_executable_outside_path_is_written_as_an_absolute_path() {
    let temp = Temp::new();
    let exe = temp.file("yunjian");
    std::fs::write(&exe, b"binary").expect("放一个假二进制");
    // 反向对照：不在 PATH 上时必须落绝对路径，否则这类安装根本起不来。
    let empty = std::ffi::OsString::new();
    assert_eq!(program(Some(&exe), Some(&empty)), exe.display().to_string());
    assert_eq!(program(Some(&exe), None), exe.display().to_string());
}

#[test]
fn a_different_yunjian_earlier_on_path_forces_the_absolute_path() {
    let temp = Temp::new();
    let mine = temp.file("mine");
    let theirs = temp.file("theirs");
    std::fs::create_dir_all(&mine).expect("建目录");
    std::fs::create_dir_all(&theirs).expect("建目录");
    let exe = mine.join("yunjian");
    std::fs::write(&exe, b"mine").expect("写自己的二进制");
    std::fs::write(theirs.join("yunjian"), b"theirs").expect("写别人的二进制");
    // PATH 里先命中的是另一个同名二进制，此时裸名会起错程序。
    let path_var = std::ffi::OsString::from(theirs.display().to_string());
    assert_eq!(
        program(Some(&exe), Some(&path_var)),
        exe.display().to_string()
    );
}

#[test]
fn a_renamed_executable_never_becomes_the_bare_name() {
    let temp = Temp::new();
    let exe = temp.file("yunjian-dev");
    std::fs::write(&exe, b"binary").expect("放一个假二进制");
    let path_var = std::ffi::OsString::from(temp.dir.display().to_string());
    assert_eq!(
        program(Some(&exe), Some(&path_var)),
        exe.display().to_string()
    );
}

// ------------------------------------------------------------------ 落盘

#[test]
fn a_dry_run_touches_nothing() {
    let temp = Temp::new();
    let target = temp.file("opencode.json");
    let mut args = args(Client::OpenCode, &target);
    args.dry_run = true;
    let outcome = install(&args, &dirs(&temp.dir)).expect("演练应当成功");
    assert!(outcome.dry_run);
    assert!(!target.exists(), "演练不得创建 {}", target.display());
    assert!(outcome.backup.is_none(), "演练不该产生备份");
    assert!(outcome.document.contains("\"mcp\""), "演练必须给出结果全文");
}

#[test]
fn writing_creates_a_backup_only_when_it_changes_an_existing_file() {
    let temp = Temp::new();
    let target = temp.file("opencode.json");
    let args = args(Client::OpenCode, &target);

    // 新建：没有旧内容可备份。
    let created = install(&args, &dirs(&temp.dir)).expect("新建成功");
    assert_eq!(created.action, Action::Created);
    assert!(created.backup.is_none(), "新建不该产生备份");

    // 二次安装：内容不变，不写盘也不备份。
    let again = install(&args, &dirs(&temp.dir)).expect("二次安装成功");
    assert_eq!(again.action, Action::Unchanged);
    assert!(again.backup.is_none(), "未改动就不该产生备份");

    // 手改一处再装：这次必须留下备份，且备份内容是改动前的。
    std::fs::write(&target, "{\n  \"theme\": \"dark\"\n}\n").expect("手改配置");
    let updated = install(&args, &dirs(&temp.dir)).expect("合并成功");
    assert_eq!(updated.action, Action::Updated);
    let backup = updated.backup.as_ref().expect("合并必须留下备份");
    assert_eq!(
        std::fs::read_to_string(backup).expect("读备份"),
        "{\n  \"theme\": \"dark\"\n}\n"
    );
    let value = parse(&std::fs::read_to_string(&target).expect("读结果"));
    assert_eq!(value["theme"], json!("dark"));
    assert_eq!(value["mcp"][ENTRY]["type"], json!("local"));
}

#[test]
fn a_refused_config_is_left_on_disk_untouched() {
    let temp = Temp::new();
    let target = temp.file("opencode.json");
    let broken = "{ \"mcp\": { oops }\n";
    std::fs::write(&target, broken).expect("写一份坏配置");
    let refusal =
        install(&args(Client::OpenCode, &target), &dirs(&temp.dir)).expect_err("坏配置必须被拒绝");
    assert!(
        matches!(refusal, Refusal::Invalid { .. }),
        "实际是 {refusal:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("读回"),
        broken,
        "被拒绝时目标文件必须逐字节不变"
    );
    // 也不该留下备份或临时文件。
    let leftovers: Vec<String> = std::fs::read_dir(&temp.dir)
        .expect("列目录")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().display().to_string())
        .filter(|name| name != "opencode.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "拒绝路径留下了残余文件：{leftovers:?}"
    );
}

// ------------------------------------------------------------------ 载荷

#[test]
fn the_dry_run_payload_renders_the_document_and_the_write_payload_reports_the_action() {
    let temp = Temp::new();
    let target = temp.file("opencode.json");
    let mut args = args(Client::OpenCode, &target);
    args.dry_run = true;
    let dry = InstallOut::new(&install(&args, &dirs(&temp.dir)).expect("演练"));
    let rendered = dry.render().join("\n");
    assert!(
        rendered.contains("演练"),
        "演练输出要说明没写盘：{rendered}"
    );
    assert!(
        rendered.contains("\"mcp\""),
        "演练输出要含结果全文：{rendered}"
    );

    args.dry_run = false;
    let written = InstallOut::new(&install(&args, &dirs(&temp.dir)).expect("写入"));
    assert_eq!(written.action, "created");
    assert_eq!(written.container_key, "mcp");
    assert_eq!(written.client, "opencode");
    let rendered = written.render().join("\n");
    assert!(
        rendered.contains("mcp.yunjian"),
        "要点名写进了哪个键：{rendered}"
    );
}
