//! MCP HTTP 传输的 bearer token：生成、落盘、读回与比对。
//!
//! # token 是机密，因此这里没有任何把它取出来的入口
//!
//! [`BearerToken`] 不实现 `Display`、不实现 `Serialize`、没有 `as_str`，`Debug` 是手写的
//! 且只打印指纹。**这不是洁癖**：`yunjian mcp` 的 stderr 会被 MCP 客户端整条收进它自己的
//! 日志文件，一次 `tracing::info!(token = %token)` 就把认证凭据写进了一个用户根本不知道
//! 存在的文件。派生 `Debug` 也不行——它会把内部字段整个渲染出来（`yunjian-ai` 的
//! `Error::Ai` 已经因为同一个理由手写 `Debug`）。
//!
//! 因此调用方拿 token 的唯一通路是**读那个 0600 的文件**，与真实客户端完全同一条路径。
//!
//! # 为什么不接受 `--token` 或环境变量
//!
//! 命令行参数在 Linux 上是 `/proc/<pid>/cmdline`，同机任何用户 `ps` 一下就能读到；
//! 环境变量同理（`/proc/<pid>/environ` 虽然只对同一用户可读，但会被子进程继承、被崩溃
//! 报告与进程快照工具收集）。文件权限是这三者里唯一能表达「只有我能读」的机制。

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// token 文件名。
pub const TOKEN_FILE_NAME: &str = "mcp-token";

/// token 文件所在的子目录名。
pub const TOKEN_DIR_NAME: &str = "yunjian";

/// token 的随机字节数。256 位，取自操作系统 CSPRNG。
pub const TOKEN_BYTES: usize = 32;

/// 日志里那个指纹的十六进制字符数。
///
/// 64 位足以让两份不同 token 的指纹在实践中不撞，同时短到无法用于离线穷举——它的用途只是
/// 让用户确认「客户端配的是这个 token」，不是任何形式的凭据。
pub const FINGERPRINT_CHARS: usize = 16;

/// token 文件必须具备的权限位。
#[cfg(unix)]
pub const TOKEN_FILE_MODE: u32 = 0o600;

/// token 目录必须具备的权限位。
#[cfg(unix)]
pub const TOKEN_DIR_MODE: u32 = 0o700;

/// 一个 bearer token。
///
/// 内部同时留着明文（写文件用）与 BLAKE3 摘要（比对用）。摘要预先算好，因为每个请求都要
/// 比一次，而 token 在进程生命周期内不变。
pub struct BearerToken {
    secret: String,
    digest: blake3::Hash,
}

impl BearerToken {
    /// 向操作系统要 [`TOKEN_BYTES`] 字节随机数并编成小写十六进制。
    ///
    /// # Errors
    ///
    /// 操作系统熵源不可用时返回错误。**此时绝不能退化成时间戳或进程号**：那样的 token 是
    /// 可预测的，比没有认证更糟——它看起来像有认证。
    pub fn generate() -> anyhow::Result<Self> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|error| anyhow::anyhow!("向操作系统申请随机数失败：{error}"))?;
        Ok(Self::from_secret(hex_lower(&bytes)))
    }

    fn from_secret(secret: String) -> Self {
        let digest = blake3::hash(secret.as_bytes());
        Self { secret, digest }
    }

    /// 读取 `path` 处已有的 token，不存在则现生成一个并以 0600 写入。
    ///
    /// 两条路径都保证返回时文件权限**恰好**是 [`TOKEN_FILE_MODE`]：已存在但更宽松时会被
    /// 收紧并留一条告警。一个 0644 的 token 文件与没有认证等价，而这种文件通常来自用户
    /// 自己的 `echo ... > token`，静默接受它就等于静默取消了整套认证。
    ///
    /// # Errors
    ///
    /// 建目录、读写文件失败，或文件内容不是合法 token 时返回错误。
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        match create_new_private_file(path) {
            Ok(mut file) => {
                let token = Self::generate()?;
                file.write_all(token.secret.as_bytes())
                    .and_then(|()| file.write_all(b"\n"))
                    .map_err(|error| {
                        anyhow::anyhow!("写入 token 文件 {} 失败：{error}", path.display())
                    })?;
                file.sync_all().map_err(|error| {
                    anyhow::anyhow!("同步 token 文件 {} 失败：{error}", path.display())
                })?;
                Ok(token)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Self::read(path),
            Err(error) => Err(anyhow::anyhow!(
                "创建 token 文件 {} 失败：{error}",
                path.display()
            )),
        }
    }

    fn read(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("读取 token 文件 {} 失败：{error}", path.display()))?;
        let secret = raw.trim();
        anyhow::ensure!(
            !secret.is_empty(),
            "token 文件 {} 是空的；删掉它让服务重新生成",
            path.display()
        );
        anyhow::ensure!(
            secret
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'"'),
            "token 文件 {} 的内容含不可用于 HTTP 头的字符",
            path.display()
        );
        tighten_permissions(path)?;
        Ok(Self::from_secret(secret.to_owned()))
    }

    /// 不可逆指纹：BLAKE3 摘要的前 [`FINGERPRINT_CHARS`] 个十六进制字符。
    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.digest
            .to_hex()
            .chars()
            .take(FINGERPRINT_CHARS)
            .collect()
    }

    /// 判断请求里带的值是否就是这个 token。
    ///
    /// 比的是两侧的 BLAKE3 摘要，而不是两个字符串：`blake3::Hash` 的 `PartialEq` 被上游
    /// 标注为常数时间实现，而 `str` 的 `==` 在首个不等字节处短路，会把「前缀猜对了几位」
    /// 泄露成可测量的时间差。
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        blake3::hash(presented.as_bytes()) == self.digest
    }
}

/// 手写而非派生：派生实现会把 `secret` 整个渲染出来。
impl fmt::Debug for BearerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BearerToken")
            .field("secret", &"<redacted>")
            .field("fingerprint", &self.fingerprint())
            .finish()
    }
}

/// 默认 token 路径：优先 `$XDG_RUNTIME_DIR/yunjian/mcp-token`，回落到应用数据目录。
///
/// 优先运行时目录是因为它的语义正好对上：由登录会话持有、模式 0700、注销即清空。token 只在
/// 服务进程活着时有意义，把它留在数据目录里等于留下一个过期的凭据文件。
#[must_use]
pub fn default_token_path() -> Option<PathBuf> {
    token_path_from(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        dirs::data_dir(),
    )
}

/// [`default_token_path`] 的纯函数内核，便于在不改进程环境的前提下断言优先级。
#[must_use]
pub fn token_path_from(runtime_dir: Option<PathBuf>, data_dir: Option<PathBuf>) -> Option<PathBuf> {
    runtime_dir
        .filter(|path| path.is_absolute())
        .or(data_dir)
        .map(|base| base.join(TOKEN_DIR_NAME).join(TOKEN_FILE_NAME))
}

fn hex_lower(bytes: &[u8]) -> String {
    use fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    match fs::DirBuilder::new()
        .recursive(true)
        .mode(TOKEN_DIR_MODE)
        .create(dir)
    {
        Ok(()) => Ok(()),
        Err(error) => Err(anyhow::anyhow!(
            "创建 token 目录 {} 失败：{error}",
            dir.display()
        )),
    }
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dir)
        .map_err(|error| anyhow::anyhow!("创建 token 目录 {} 失败：{error}", dir.display()))
}

#[cfg(unix)]
fn create_new_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(TOKEN_FILE_MODE)
        .open(path)
}

#[cfg(not(unix))]
fn create_new_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
fn tighten_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::metadata(path).map_err(|error| {
        anyhow::anyhow!("读取 token 文件 {} 的权限失败：{error}", path.display())
    })?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode == TOKEN_FILE_MODE {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(TOKEN_FILE_MODE)).map_err(|error| {
        anyhow::anyhow!("收紧 token 文件 {} 的权限失败：{error}", path.display())
    })?;
    tracing::warn!(
        token_file = %path.display(),
        found_mode = format!("{mode:04o}"),
        "token 文件权限过宽，已收紧为 0600"
    );
    Ok(())
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BearerToken, FINGERPRINT_CHARS, TOKEN_BYTES, token_path_from};
    use std::path::PathBuf;

    #[test]
    fn a_generated_token_is_sixty_four_hex_characters() {
        let token = BearerToken::generate().expect("生成 token");
        let debug = format!("{token:?}");
        assert!(
            debug.contains("<redacted>"),
            "Debug 必须遮蔽明文，实为 {debug}"
        );
        assert_eq!(token.fingerprint().len(), FINGERPRINT_CHARS);
        assert_eq!(TOKEN_BYTES * 2, 64);
    }

    #[test]
    fn two_generated_tokens_differ() {
        let left = BearerToken::generate().expect("生成 token");
        let right = BearerToken::generate().expect("生成 token");
        assert_ne!(
            left.fingerprint(),
            right.fingerprint(),
            "两次生成不得给出同一个 token"
        );
    }

    #[test]
    fn matching_accepts_the_persisted_value_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("yunjian-token-match-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mcp-token");

        let token = BearerToken::load_or_create(&path).expect("落盘 token");
        let secret = std::fs::read_to_string(&path)
            .expect("读回 token")
            .trim()
            .to_owned();

        assert!(token.matches(&secret), "落盘的值必须被接受");
        assert!(!token.matches(""), "空串不得被接受");
        assert!(
            !token.matches(&format!("{secret}x")),
            "多一个字符不得被接受"
        );
        assert!(
            !token.matches(&secret[..secret.len() - 1]),
            "前缀不得被接受"
        );

        let reopened = BearerToken::load_or_create(&path).expect("重开 token 文件");
        assert_eq!(
            reopened.fingerprint(),
            token.fingerprint(),
            "文件已存在时必须读回原值而不是重新生成"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_runtime_directory_wins_over_the_data_directory() {
        let runtime = PathBuf::from("/run/user/1000");
        let data = PathBuf::from("/home/someone/.local/share");
        assert_eq!(
            token_path_from(Some(runtime), Some(data.clone())),
            Some(PathBuf::from("/run/user/1000/yunjian/mcp-token"))
        );
        assert_eq!(
            token_path_from(None, Some(data)),
            Some(PathBuf::from(
                "/home/someone/.local/share/yunjian/mcp-token"
            ))
        );
        assert_eq!(
            token_path_from(Some(PathBuf::from("relative")), None),
            None,
            "相对路径的 XDG_RUNTIME_DIR 不可用，且不得静默当成相对当前目录"
        );
    }
}
