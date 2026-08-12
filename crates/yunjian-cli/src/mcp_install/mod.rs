//! 把 `yunjian mcp` 注册进 MCP 客户端的配置文件。
//!
//! # 两个客户端不共用一套 schema
//!
//! 这是本模块存在的全部理由。两种形态**顶层键不同、`command` 的类型也不同**：
//!
//! ```json
//! // Claude Desktop：claude_desktop_config.json
//! {"mcpServers": {"yunjian": {"command": "yunjian", "args": ["mcp"]}}}
//! ```
//!
//! ```json
//! // OpenCode：opencode.json
//! {"mcp": {"yunjian": {"type": "local", "command": ["yunjian", "mcp"], "enabled": true}}}
//! ```
//!
//! Claude 的 `command` 是**字符串**、参数另放 `args`；OpenCode 的 `command` 是
//! **含参数的数组**，另有 `type` 与 `enabled`。把其中一种套到另一种上，客户端读到的
//! 是一个语法合法而语义为空的条目——不会报错，只是永远连不上。
//!
//! # 合并而不是覆盖
//!
//! 用户的配置里通常已有别的 MCP 服务器。因此写入走「定位 + 文本拼接」而不是
//! 「反序列化 + 重新序列化」：见 [`jsonc`]。除 `yunjian` 那一段，其余字节逐字保留，
//! 注释也留着。
//!
//! # 拒绝而不是替换
//!
//! 目标文件解析失败时**不写**。一个被手改坏的配置里可能有用户十几条服务器定义，
//! 拿一份干净的新文件覆盖它，等于用「安装成功」换掉用户的全部配置。

pub mod jsonc;

use crate::envelope::{ErrorCode, Failure};
use crate::exit::Exit;
use crate::output::Renderable;
use clap::{Args, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 写进客户端配置的服务器条目名。
pub const ENTRY: &str = "yunjian";

/// 起 MCP stdio 服务的子命令名。
const SERVE_ARG: &str = "mcp";

/// 分发出去的可执行文件名，不带平台后缀。
const PROGRAM: &str = "yunjian";

/// `yunjian mcp install` 的参数。
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InstallArgs {
    /// 目标客户端。没有默认值：两种形态不通用，猜错等于写了一个连不上的条目。
    #[arg(long, value_name = "CLIENT", value_enum)]
    pub client: Client,
    /// 写用户级（全局）配置而不是当前目录的项目级配置。
    ///
    /// Claude Desktop 只有用户级配置，因此该参数对它无效并会给出警告。
    #[arg(long)]
    pub global: bool,
    /// 显式指定目标文件，压过按平台推导的路径。
    #[arg(long, value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// 只把结果文件打到 stdout，不碰磁盘。
    #[arg(long)]
    pub dry_run: bool,
}

/// 支持自动注册的客户端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "lower")]
pub enum Client {
    /// Claude Desktop。
    Claude,
    /// OpenCode。
    OpenCode,
}

impl Client {
    /// 稳定 ASCII 键，用于 `--json` 载荷。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
        }
    }

    /// 承载全部服务器条目的顶层键。**两个客户端在此处就已经不同。**
    #[must_use]
    pub const fn container_key(self) -> &'static str {
        match self {
            Self::Claude => "mcpServers",
            Self::OpenCode => "mcp",
        }
    }

    /// 客户端有没有项目级配置。
    ///
    /// Claude Desktop 只读用户级的 `claude_desktop_config.json`，没有「当前项目」这个
    /// 概念；OpenCode 既读 `~/.config/opencode/opencode.json` 也读工作目录下的
    /// `opencode.json`。
    #[must_use]
    pub const fn has_project_scope(self) -> bool {
        match self {
            Self::Claude => false,
            Self::OpenCode => true,
        }
    }

    /// 本客户端认的服务器条目。
    ///
    /// `program` 是要执行的程序：在 `PATH` 上时是裸名 `yunjian`，否则是绝对路径。
    #[must_use]
    pub fn entry(self, program: &str) -> Value {
        match self {
            // 字符串 `command` + 独立 `args`。
            Self::Claude => json!({ "command": program, "args": [SERVE_ARG] }),
            // 数组 `command`（含参数）+ `type` + `enabled`。
            Self::OpenCode => json!({
                "type": "local",
                "command": [program, SERVE_ARG],
                "enabled": true,
            }),
        }
    }
}

/// 目标文件的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// 用户级配置。
    Global,
    /// 当前工作目录下的项目级配置。
    Project,
    /// `--path` 显式指定。
    Explicit,
}

impl Scope {
    /// 稳定 ASCII 键。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Explicit => "explicit",
        }
    }
}

/// 这次安装对目标文件做了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// 目标文件原先不存在，新建。
    Created,
    /// 合并进既有文件。
    Updated,
    /// 既有条目已经是目标形态，未写盘。
    Unchanged,
}

impl Action {
    /// 稳定 ASCII 键。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

/// 推导目标路径所需的目录，注入而非现取。
///
/// **不能在测试里改环境变量来隔离**：`dirs` 在 Windows 上走 `SHGetKnownFolderPath`，
/// 完全不读 `%APPDATA%`，于是「重定向环境变量」这套隔离手段在 CI 的 Windows 作业上
/// 是无效的。把目录作为入参传进来，路径推导才能在三个平台上用同一组断言验证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// 平台配置目录：Linux `$XDG_CONFIG_HOME`、macOS `~/Library/Application Support`、
    /// Windows `%APPDATA%`。
    pub config: Option<PathBuf>,
    /// XDG 配置目录。OpenCode 在三个平台上都用它，不跟随平台惯例。
    pub xdg_config: Option<PathBuf>,
    /// 当前工作目录。
    pub cwd: PathBuf,
}

impl Dirs {
    /// 按本机实际情况取目录。
    ///
    /// # Errors
    ///
    /// 取不到当前工作目录时返回错误。
    pub fn discover() -> Result<Self, std::io::Error> {
        Ok(Self {
            config: dirs::config_dir(),
            xdg_config: xdg_config_dir(),
            cwd: std::env::current_dir()?,
        })
    }
}

/// XDG 语义的配置目录：`$XDG_CONFIG_HOME`，否则 `$HOME/.config`。
///
/// 刻意不用 `dirs::config_dir()`：它在 macOS 上给的是
/// `~/Library/Application Support`，而 OpenCode 在 macOS 上读的仍然是
/// `~/.config/opencode/opencode.json`。用平台惯例去猜一个不跟随平台惯例的客户端，
/// 结果是写进一个它永远不会读的文件。
fn xdg_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    dirs::home_dir().map(|home| home.join(".config"))
}

/// 一次安装的完整结果。
#[derive(Debug, Clone)]
pub struct Outcome {
    /// 目标客户端。
    pub client: Client,
    /// 目标文件。
    pub path: PathBuf,
    /// 路径来源。
    pub scope: Scope,
    /// 写进条目的程序路径或名字。
    pub program: String,
    /// 结果文件全文。
    pub document: String,
    /// 对目标文件做了什么。
    pub action: Action,
    /// 备份文件路径；未写盘时为 `None`。
    pub backup: Option<PathBuf>,
    /// 是否只是演练。
    pub dry_run: bool,
}

/// 拒绝安装的原因。
#[derive(Debug)]
pub enum Refusal {
    /// 目标文件不是合法 JSON / JSONC。
    Invalid {
        /// 目标文件。
        path: PathBuf,
        /// 解析器给出的原因。
        reason: String,
    },
    /// 顶层不是 JSON 对象。
    RootNotObject {
        /// 目标文件。
        path: PathBuf,
    },
    /// 承载服务器条目的顶层键存在，但它的值不是对象。
    ContainerNotObject {
        /// 目标文件。
        path: PathBuf,
        /// 该顶层键。
        key: &'static str,
    },
    /// 推导不出目标路径。
    Unresolved {
        /// 目标客户端。
        client: Client,
        /// 原因。
        reason: String,
    },
    /// 读写目标文件失败。
    Io {
        /// 目标文件。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
}

impl Refusal {
    /// 翻成退出码与信封里的失败描述。
    ///
    /// 前四种都是「调用方改」——修配置或换路径，故退出 2。I/O 失败是「本机状态不对，
    /// 命令本身没写错」，按 `exit` 模块记的分类归 3，但**错误码另立一档**：
    /// 建议去取语料对一次写文件失败是纯误导。
    #[must_use]
    pub fn describe(&self) -> (Exit, Failure) {
        match self {
            Self::Invalid { path, reason } => (
                Exit::Usage,
                Failure::new(
                    ErrorCode::ClientConfigInvalid,
                    format!("{} 不是合法的 JSON/JSONC：{reason}", path.display()),
                )
                .with_hint("先修好该文件再重试；云笺不会替换一份解析不了的配置"),
            ),
            Self::RootNotObject { path } => (
                Exit::Usage,
                Failure::new(
                    ErrorCode::ClientConfigInvalid,
                    format!("{} 的顶层不是 JSON 对象", path.display()),
                )
                .with_hint("客户端配置的顶层必须是对象；确认这个路径确实是客户端配置文件"),
            ),
            Self::ContainerNotObject { path, key } => (
                Exit::Usage,
                Failure::new(
                    ErrorCode::ClientConfigInvalid,
                    format!(
                        "{} 里的 `{key}` 不是对象，无法在其中登记服务器",
                        path.display()
                    ),
                )
                .with_hint("把该键改成对象后重试"),
            ),
            Self::Unresolved { client, reason } => (
                Exit::Usage,
                Failure::new(
                    ErrorCode::ClientConfigInvalid,
                    format!("推导不出 {} 的配置路径：{reason}", client.as_key()),
                )
                .with_hint("用 `--path` 显式指定配置文件"),
            ),
            Self::Io { path, reason } => (
                Exit::CorpusUnavailable,
                Failure::new(
                    ErrorCode::ClientConfigWriteFailed,
                    format!("读写 {} 失败：{reason}", path.display()),
                )
                .with_hint("检查该路径的所在目录是否存在且可写"),
            ),
        }
    }
}

/// 解析目标文件路径。
///
/// # Errors
///
/// 取不到所需的平台目录时返回 [`Refusal::Unresolved`]。
pub fn resolve(
    client: Client,
    global: bool,
    args_path: Option<&Path>,
    dirs: &Dirs,
) -> Result<(PathBuf, Scope), Refusal> {
    if let Some(path) = args_path {
        return Ok((path.to_path_buf(), Scope::Explicit));
    }
    match client {
        Client::Claude => {
            let base = dirs.config.clone().ok_or_else(|| Refusal::Unresolved {
                client,
                reason: "取不到平台配置目录".to_owned(),
            })?;
            Ok((
                base.join("Claude").join("claude_desktop_config.json"),
                Scope::Global,
            ))
        }
        Client::OpenCode if global => {
            let base = dirs.xdg_config.clone().ok_or_else(|| Refusal::Unresolved {
                client,
                reason: "取不到 XDG 配置目录".to_owned(),
            })?;
            Ok((base.join("opencode").join("opencode.json"), Scope::Global))
        }
        Client::OpenCode => {
            // 已有 `.jsonc` 就写它，否则用 `.json`：两个都建会让客户端读到两份配置。
            let jsonc = dirs.cwd.join("opencode.jsonc");
            let path = if jsonc.is_file() {
                jsonc
            } else {
                dirs.cwd.join("opencode.json")
            };
            Ok((path, Scope::Project))
        }
    }
}

/// 决定写进条目的程序：在 `PATH` 上就用裸名，否则用绝对路径。
///
/// 裸名优先不是偷懒。写绝对路径的配置在用户换个安装位置、或把配置同步到另一台机器
/// 之后就指向一个不存在的文件；而 `PATH` 上找得到时，裸名在两种情形下都还成立。
/// 反过来，装在 `PATH` 之外（`cargo install --git` 之前的手工放置、便携目录）时裸名
/// 根本起不来，所以那种情形必须落绝对路径——这两条互为对照，缺一条就有一类安装装不上。
#[must_use]
pub fn program(exe: Option<&Path>, path_var: Option<&OsStr>) -> String {
    let Some(exe) = exe else {
        return PROGRAM.to_owned();
    };
    let absolute = exe.display().to_string();
    if exe.file_stem().and_then(OsStr::to_str) != Some(PROGRAM) {
        return absolute;
    }
    let Some(file_name) = exe.file_name() else {
        return absolute;
    };
    let Some(path_var) = path_var else {
        return absolute;
    };
    // 按 `PATH` 顺序找第一个同名文件：命中的若不是本进程，裸名会起到另一个二进制。
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(file_name);
        if !candidate.is_file() {
            continue;
        }
        return if same_file(&candidate, exe) {
            PROGRAM.to_owned()
        } else {
            absolute
        };
    }
    absolute
}

/// 两个路径是否指向同一个文件。
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// 组出合并后的文件全文。
///
/// # Errors
///
/// 目标文件解析失败、顶层不是对象、或容器键的值不是对象时返回 [`Refusal`]，
/// **且此时一个字节都还没写出去**。
pub fn merge(
    client: Client,
    path: &Path,
    existing: Option<&str>,
    program: &str,
) -> Result<String, Refusal> {
    let entry = client.entry(program);
    let container_key = client.container_key();

    let Some(existing) = existing.filter(|text| !text.trim().is_empty()) else {
        // 新建：整份文档由我们生成，键序与缩进因此是确定的。
        let document = serde_json::to_string_pretty(&json!({
            container_key: { ENTRY: entry },
        }))
        .map_err(|error| Refusal::Io {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        return Ok(format!("{document}\n"));
    };

    let stripped = jsonc::strip_comments(existing);
    // 先证明它是合法 JSON，再动手。拒绝的分支必须在任何写入之前。
    serde_json::from_str::<Value>(&stripped).map_err(|error| Refusal::Invalid {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let root = jsonc::root_object(&stripped).ok_or_else(|| Refusal::RootNotObject {
        path: path.to_path_buf(),
    })?;

    match jsonc::find_member(&stripped, root, container_key) {
        Some(container) => {
            if !jsonc::is_object(&stripped, container.value_start) {
                return Err(Refusal::ContainerNotObject {
                    path: path.to_path_buf(),
                    key: container_key,
                });
            }
            let inner = jsonc::ObjectSpan {
                start: container.value_start,
                end: container.value_end,
            };
            Ok(jsonc::upsert_member(
                existing, &stripped, inner, ENTRY, &entry,
            ))
        }
        None => Ok(jsonc::upsert_member(
            existing,
            &stripped,
            root,
            container_key,
            &json!({ ENTRY: entry }),
        )),
    }
}

/// 执行一次安装。
///
/// # Errors
///
/// 见 [`Refusal`]。
pub fn install(args: &InstallArgs, dirs: &Dirs) -> Result<Outcome, Refusal> {
    let (path, scope) = resolve(args.client, args.global, args.path.as_deref(), dirs)?;
    let program = program(
        std::env::current_exe().ok().as_deref(),
        std::env::var_os("PATH").as_deref(),
    );

    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Refusal::Io {
                path,
                reason: error.to_string(),
            });
        }
    };
    let document = merge(args.client, &path, existing.as_deref(), &program)?;

    let action = match existing.as_deref() {
        None => Action::Created,
        Some(previous) if previous == document => Action::Unchanged,
        Some(_) => Action::Updated,
    };

    let mut backup = None;
    if !args.dry_run && action != Action::Unchanged {
        if let Some(previous) = existing.as_deref() {
            backup = Some(write_backup(&path, previous)?);
        }
        write_atomically(&path, &document)?;
    }

    Ok(Outcome {
        client: args.client,
        path,
        scope,
        program,
        document,
        action,
        backup,
        dry_run: args.dry_run,
    })
}

/// 备份既有文件。
///
/// 文件名带毫秒时间戳而不是固定的 `.bak`：装第二次时固定名会被第一次安装后的内容
/// 覆盖，于是**原始配置这份最值得留的备份反而最先丢**。
fn write_backup(path: &Path, previous: &str) -> Result<PathBuf, Refusal> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("config.json");
    let backup = path.with_file_name(format!("{name}.bak-{stamp}"));
    std::fs::write(&backup, previous).map_err(|error| Refusal::Io {
        path: backup.clone(),
        reason: error.to_string(),
    })?;
    Ok(backup)
}

/// 同目录临时文件 → fsync → 改名。
///
/// 直接覆写会在中途失败时留下一个被截断的配置——客户端读到的不是旧配置也不是新配置，
/// 而是一份坏文件。临时文件必须与目标同目录：跨文件系统的 `rename` 不是原子操作。
fn write_atomically(path: &Path, document: &str) -> Result<(), Refusal> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| Refusal::Io {
        path: parent.to_path_buf(),
        reason: error.to_string(),
    })?;
    let temp = path.with_extension(format!("yunjian-{}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&temp).map_err(|error| Refusal::Io {
        path: temp.clone(),
        reason: error.to_string(),
    })?;
    let written = file
        .write_all(document.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(Refusal::Io {
            path: temp,
            reason: error.to_string(),
        });
    }
    drop(file);
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        Refusal::Io {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }
    })
}

/// `install` 的 `--json` 载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallOut {
    /// 目标客户端。
    pub client: &'static str,
    /// 承载服务器条目的顶层键。
    pub container_key: &'static str,
    /// 服务器条目名。
    pub entry: &'static str,
    /// 目标文件。
    pub path: String,
    /// 路径来源。
    pub scope: &'static str,
    /// 写进条目的程序。
    pub program: String,
    /// 对目标文件做了什么。
    pub action: &'static str,
    /// 备份文件路径。
    pub backup: Option<String>,
    /// 是否只是演练。
    pub dry_run: bool,
    /// 结果文件全文。
    pub document: String,
}

impl InstallOut {
    /// 从安装结果组装载荷。
    #[must_use]
    pub fn new(outcome: &Outcome) -> Self {
        Self {
            client: outcome.client.as_key(),
            container_key: outcome.client.container_key(),
            entry: ENTRY,
            path: outcome.path.display().to_string(),
            scope: outcome.scope.as_key(),
            program: outcome.program.clone(),
            action: outcome.action.as_key(),
            backup: outcome
                .backup
                .as_ref()
                .map(|path| path.display().to_string()),
            dry_run: outcome.dry_run,
            document: outcome.document.clone(),
        }
    }
}

impl Renderable for InstallOut {
    fn render(&self) -> Vec<String> {
        // 演练时结果文件全文就是产物，直接铺到 stdout；其余情形只报做了什么。
        if self.dry_run {
            let mut lines = vec![format!("# {}（演练，未写盘）", self.path)];
            lines.extend(self.document.lines().map(str::to_owned));
            return lines;
        }
        let mut lines = vec![
            format!("{} 配置：{}（{}）", self.client, self.path, self.scope),
            format!("{}.{} → {}", self.container_key, self.entry, self.program),
        ];
        lines.push(
            match self.action {
                "created" => "已新建配置文件",
                "unchanged" => "条目已是目标形态，未改动文件",
                _ => "已合并进既有配置，其余条目原样保留",
            }
            .to_owned(),
        );
        if let Some(backup) = &self.backup {
            lines.push(format!("备份：{backup}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests;
