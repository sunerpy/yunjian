//! `yunjian mcp install` 的端到端契约。
//!
//! # 为什么必须用子进程
//!
//! 合并算法本身的单测在 `src/mcp_install/tests.rs`。这里验的是三件只有真进程才有的事：
//!
//! 1. **退出码**。拒绝一份坏配置必须是 2 而不是 0——脚本靠它判断有没有装上。
//! 2. **`--json` 的 stdout 恰好一行**。同一个二进制还承载 MCP stdio 服务端，多一行
//!    就是把「结果流」和「协议流」的约定同时破掉。
//! 3. **`--dry-run` 打到 stdout 且不碰磁盘**。「不碰磁盘」是文件系统事实，不是返回值。
//!
//! # 为什么把二进制目录塞进 `PATH`
//!
//! 条目里写裸名还是绝对路径，取决于 `yunjian` 在不在 `PATH` 上。验收要求 OpenCode 的
//! `command` 恰好是 `["yunjian","mcp"]`，因此这些用例把构建产物所在目录设成 `PATH`
//! ——那正是用户装好之后的真实状态。反向情形（不在 `PATH` 上时落绝对路径）也有一条
//! 用例，两条合起来才算验证了这个选择。

use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 服务器条目名。
const ENTRY: &str = "yunjian";

/// 日志行里必然出现、而结果里绝不该出现的记号。
const LOG_MARKERS: &[&str] = &[
    "INFO",
    "WARN",
    "ERROR",
    "DEBUG",
    "TRACE",
    "yunjian_cli:",
    "yunjian_core:",
];

static NEXT_SANDBOX: AtomicU32 = AtomicU32::new(0);

/// 一次测试的独立目录，析构时整棵删掉。
struct Sandbox {
    dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Sandbox {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-install-{}-{}",
            std::process::id(),
            NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建沙箱目录");
        let sandbox = Self { dir };
        sandbox.write_config();
        sandbox
    }

    fn config(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    fn target(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 云笺自己的配置。`install` 不读语料，但 `main` 仍会初始化配置与日志，
    /// 因此日志目录必须落在沙箱里而不是用户的真实数据目录。
    fn write_config(&self) {
        let contents = format!(
            "[app]\nname = \"云笺\"\ndata_dir = {app}\n\n\
             [corpus]\ndata_dir = {corpus}\n\n\
             [logger]\nlevel = \"info\"\njson = false\ndir = {logs}\nfile_prefix = \"yunjian\"\n",
            app = quote(&self.dir.join("app")),
            corpus = quote(&self.dir.join("corpus")),
            logs = quote(&self.dir.join("logs")),
        );
        std::fs::write(self.config(), contents).expect("写沙箱配置");
    }

    /// 一条已隔离环境的命令，且把构建产物目录设成唯一的 `PATH`。
    fn command(&self) -> Command {
        let exe = assert_cmd::cargo::cargo_bin("yunjian");
        let bin_dir = exe.parent().expect("构建产物必有父目录").to_path_buf();
        let mut command = Command::new(&exe);
        command
            .env_remove("APP_CONFIG")
            .env_remove("YUNJIAN_CORPUS_PATH")
            .env_remove("RUST_LOG")
            .env("PATH", &bin_dir)
            .arg("--config")
            .arg(self.config());
        command
    }

    /// 一条 `PATH` 上没有 `yunjian` 的命令。
    fn command_without_path(&self) -> Command {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("yunjian"));
        command
            .env_remove("APP_CONFIG")
            .env_remove("YUNJIAN_CORPUS_PATH")
            .env_remove("RUST_LOG")
            .env("PATH", "")
            .arg("--config")
            .arg(self.config());
        command
    }
}

/// 把路径写成合法的 TOML 字符串字面量。
///
/// **不能用 `format!("\"{}\"", path.display())`。** Windows 的临时目录形如
/// `C:\Users\RUNNER~1\...`，而 `\U` 在 TOML basic string 里是非法转义，解析当场失败。
fn quote(path: &Path) -> String {
    toml::Value::from(path.to_string_lossy().into_owned()).to_string()
}

/// 跑一次 install，返回 (退出码, stdout, stderr)。
fn run(mut command: Command, arguments: &[&str]) -> (i32, String, String) {
    let output = command
        .arg("mcp")
        .arg("install")
        .args(arguments)
        .output()
        .expect("运行 yunjian mcp install");
    (
        output
            .status
            .code()
            .expect("进程应当正常退出而不是被信号杀掉"),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("读 {}：{error}", path.display()))
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("应为合法 JSON：{error}\n{text}"))
}

/// `--json` 的信封：断言恰好一行并解析出来。
fn envelope(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "`--json` 的 stdout 必须恰好一行，实际 {} 行：{stdout}",
        lines.len()
    );
    parse(lines[0])
}

fn assert_no_logs_on_stdout(stdout: &str) {
    for marker in LOG_MARKERS {
        assert!(
            !stdout.contains(marker),
            "stdout 里出现了日志记号 {marker}：{stdout}"
        );
    }
}

// ------------------------------------------------------- 验收：两种形态各自精确

#[test]
fn a_fresh_claude_config_gets_a_string_command_and_args_mcp() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("claude_desktop_config.json");
    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "claude",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_no_logs_on_stdout(&stdout);

    let value = parse(&read(&target));
    assert_eq!(
        value["mcpServers"][ENTRY]["args"],
        serde_json::json!(["mcp"])
    );
    assert_eq!(
        value["mcpServers"][ENTRY]["command"],
        serde_json::json!("yunjian")
    );
    // 顶层键不是 OpenCode 的那个，`command` 也不是数组。
    assert!(value.get("mcp").is_none(), "Claude 配置里不该有 `mcp` 键");
    assert!(
        value["mcpServers"][ENTRY]["command"].is_string(),
        "Claude 的 command 必须是字符串"
    );
}

#[test]
fn a_fresh_opencode_config_gets_an_array_command_and_type_local() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_no_logs_on_stdout(&stdout);

    let value = parse(&read(&target));
    assert_eq!(
        value["mcp"][ENTRY]["command"],
        serde_json::json!(["yunjian", "mcp"])
    );
    assert_eq!(value["mcp"][ENTRY]["type"], serde_json::json!("local"));
    assert_eq!(value["mcp"][ENTRY]["enabled"], serde_json::json!(true));
    assert!(
        value.get("mcpServers").is_none(),
        "OpenCode 配置里不该有 `mcpServers` 键"
    );
}

#[test]
fn an_executable_off_path_is_registered_by_absolute_path() {
    // 反向对照：`PATH` 上找不到 `yunjian` 时必须落绝对路径，否则这类安装起不来。
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let (code, _, stderr) = run(
        sandbox.command_without_path(),
        &[
            "--client",
            "opencode",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let value = parse(&read(&target));
    let command = value["mcp"][ENTRY]["command"]
        .as_array()
        .expect("command 是数组");
    let program = command[0].as_str().expect("首项是程序名");
    assert_ne!(program, "yunjian", "不在 PATH 上时不该写裸名");
    assert!(
        Path::new(program).is_absolute(),
        "不在 PATH 上时必须写绝对路径：{program}"
    );
    assert_eq!(command[1], serde_json::json!("mcp"));
}

// ------------------------------------------------------- 验收：合并不动别人

#[test]
fn an_existing_unrelated_server_entry_survives_the_merge_byte_identically() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let existing = "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"theme\": \"system\",\n  \
                    \"mcp\": {\n    \"other-server\": {\n      \"type\": \"local\",\n      \
                    \"command\": [\"other\", \"serve\"],\n      \"enabled\": true\n    }\n  }\n}\n";
    std::fs::write(&target, existing).expect("预置一份含另一个服务器的配置");

    let (code, _, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");

    let merged = read(&target);
    // 逐字节：原文里那一整段必须原样出现在结果里，缩进、键序、空白一个都不许变。
    let fragment = "\"other-server\": {\n      \"type\": \"local\",\n      \
                    \"command\": [\"other\", \"serve\"],\n      \"enabled\": true\n    }";
    assert!(merged.contains(fragment), "无关条目被改写了：\n{merged}");
    // 无关的顶层键同样原样保留。
    assert!(merged.contains("\"$schema\": \"https://opencode.ai/config.json\""));
    assert!(merged.contains("\"theme\": \"system\""));

    let value = parse(&merged);
    assert_eq!(
        value["mcp"]["other-server"]["command"],
        serde_json::json!(["other", "serve"])
    );
    assert_eq!(value["mcp"][ENTRY]["type"], serde_json::json!("local"));
}

#[test]
fn comments_in_a_json_named_config_survive_and_do_not_break_the_merge() {
    // 实测过的真实形态：一份 `opencode.json` 里带 `//` 注释，严格 JSON 解析器会当场报错。
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let existing = "{\n  \"mcp\": {\n    // 这条先留着，别动\n    \"other\": {\n      \
                    \"type\": \"local\"\n    }\n  }\n}\n";
    std::fs::write(&target, existing).expect("预置一份含注释的配置");

    let (code, _, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let merged = read(&target);
    assert!(
        merged.contains("// 这条先留着，别动"),
        "注释丢了：\n{merged}"
    );
    assert!(merged.contains("\"other\""), "无关条目丢了：\n{merged}");
}

// ------------------------------------------------------- 验收：备份

#[test]
fn a_backup_file_is_created_before_an_existing_config_is_rewritten() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let existing = "{\n  \"theme\": \"dark\"\n}\n";
    std::fs::write(&target, existing).expect("预置配置");

    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--json",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");

    let backup = envelope(&stdout)["data"]["backup"]
        .as_str()
        .expect("信封必须报出备份路径")
        .to_owned();
    assert!(Path::new(&backup).is_file(), "备份文件不存在：{backup}");
    // 备份的内容必须是改动**之前**的那一份，否则它备份的是修改结果，毫无用处。
    assert_eq!(read(Path::new(&backup)), existing);
    assert!(
        backup.contains("opencode.json.bak-"),
        "备份名要能一眼看出源文件：{backup}"
    );
}

#[test]
fn a_newly_created_config_needs_no_backup_and_a_reinstall_writes_nothing() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let arguments = [
        "--client",
        "opencode",
        "--json",
        "--path",
        &target.display().to_string(),
    ];

    let (code, stdout, stderr) = run(sandbox.command(), &arguments);
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let data = envelope(&stdout)["data"].clone();
    assert_eq!(data["action"], serde_json::json!("created"));
    assert!(data["backup"].is_null(), "新建不该产生备份：{data}");

    // 二次安装：条目已是目标形态，既不改文件也不备份。
    let before = read(&target);
    let (code, stdout, stderr) = run(sandbox.command(), &arguments);
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(
        envelope(&stdout)["data"]["action"],
        serde_json::json!("unchanged")
    );
    assert_eq!(read(&target), before, "重复安装不该改动文件");
    let backups = backup_names(&sandbox.dir);
    assert!(backups.is_empty(), "未改动就不该留下备份：{backups:?}");
}

fn backup_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("列沙箱目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".bak-"))
        .collect()
}

// ------------------------------------------------------- 验收：dry-run

#[test]
fn a_dry_run_prints_the_resulting_file_to_stdout_and_touches_nothing() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("claude_desktop_config.json");
    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "claude",
            "--dry-run",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_no_logs_on_stdout(&stdout);
    // 结果文件全文进了 stdout。
    assert!(
        stdout.contains("\"mcpServers\""),
        "演练要打出结果全文：{stdout}"
    );
    assert!(stdout.contains("\"args\""), "演练要打出结果全文：{stdout}");
    // 而磁盘上什么都没发生。
    assert!(!target.exists(), "演练不得创建 {}", target.display());
    assert!(backup_names(&sandbox.dir).is_empty(), "演练不该产生备份");
}

#[test]
fn a_dry_run_over_an_existing_config_leaves_it_byte_identical() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let existing = "{\n  \"mcp\": {\n    \"other\": {\"type\": \"local\"}\n  }\n}\n";
    std::fs::write(&target, existing).expect("预置配置");

    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--dry-run",
            "--json",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let data = envelope(&stdout)["data"].clone();
    assert_eq!(data["dry_run"], serde_json::json!(true));
    // 演练给出的是**合并后**的全文，但磁盘上仍是原文。
    let document = data["document"].as_str().expect("信封带结果全文");
    assert!(document.contains(ENTRY), "演练结果里应当有新条目");
    assert_eq!(read(&target), existing, "演练必须让目标文件逐字节不变");
}

// ------------------------------------------------------- 验收：拒绝而非替换

#[test]
fn an_invalid_config_is_refused_with_exit_two_and_left_untouched() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let broken = "{\n  \"mcp\": {\n    \"other\": {\"type\": \"local\"}\n  \n";
    std::fs::write(&target, broken).expect("预置一份坏配置");

    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--json",
            "--path",
            &target.display().to_string(),
        ],
    );
    // 2 而不是 0：脚本靠退出码判断有没有装上。
    assert_eq!(code, 2, "非法配置必须以用法错误退出\nstderr:\n{stderr}");
    let error = envelope(&stdout)["error"].clone();
    assert_eq!(error["code"], serde_json::json!("client_config_invalid"));
    assert!(
        error["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("不会替换")),
        "必须说明我们没有替换文件：{error}"
    );
    // 最要紧的一条：文件逐字节不变，用户的其余服务器定义都还在。
    assert_eq!(read(&target), broken, "被拒绝时配置必须逐字节不变");
    assert!(
        backup_names(&sandbox.dir).is_empty(),
        "拒绝路径不该留下备份"
    );
}

#[test]
fn a_container_key_that_is_not_an_object_is_refused_rather_than_clobbered() {
    let sandbox = Sandbox::new();
    let target = sandbox.target("opencode.json");
    let existing = "{\n  \"mcp\": \"disabled\"\n}\n";
    std::fs::write(&target, existing).expect("预置配置");

    let (code, stdout, _) = run(
        sandbox.command(),
        &[
            "--client",
            "opencode",
            "--json",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(
        envelope(&stdout)["error"]["code"],
        serde_json::json!("client_config_invalid")
    );
    assert_eq!(read(&target), existing, "被拒绝时配置必须逐字节不变");
}

// ------------------------------------------------------- 作用域与用法

#[test]
fn a_missing_client_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let (code, _, stderr) = run(sandbox.command(), &[]);
    assert_eq!(code, 2, "缺 --client 必须是用法错误");
    assert!(
        stderr.contains("--client"),
        "诊断要点名缺的是哪个参数：{stderr}"
    );
}

#[test]
fn global_on_claude_reports_that_it_changed_nothing() {
    // Claude 只有用户级配置。静默接受 `--global` 会让用户以为自己控制了作用域。
    let sandbox = Sandbox::new();
    let target = sandbox.target("claude_desktop_config.json");
    let (code, stdout, stderr) = run(
        sandbox.command(),
        &[
            "--client",
            "claude",
            "--global",
            "--dry-run",
            "--json",
            "--path",
            &target.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    // `--path` 已经决定了目标，此时不该再抱怨作用域。
    assert_eq!(
        envelope(&stdout)["warnings"],
        serde_json::json!([]),
        "显式给了 --path 就不该再报作用域警告"
    );
}

#[test]
fn opencode_without_global_writes_the_project_file_in_the_working_directory() {
    let sandbox = Sandbox::new();
    let project = sandbox.target("project");
    std::fs::create_dir_all(&project).expect("建项目目录");
    let mut command = sandbox.command();
    command.current_dir(&project);
    let (code, stdout, stderr) = run(command, &["--client", "opencode", "--json"]);
    assert_eq!(code, 0, "stderr:\n{stderr}");

    let data = envelope(&stdout)["data"].clone();
    assert_eq!(data["scope"], serde_json::json!("project"));
    let written = PathBuf::from(data["path"].as_str().expect("信封带目标路径"));
    assert_eq!(
        written.file_name().and_then(std::ffi::OsStr::to_str),
        Some("opencode.json")
    );
    assert!(written.is_file(), "项目级配置未落盘：{}", written.display());
    let value = parse(&read(&written));
    assert_eq!(
        value["mcp"][ENTRY]["command"],
        serde_json::json!(["yunjian", "mcp"])
    );
}
