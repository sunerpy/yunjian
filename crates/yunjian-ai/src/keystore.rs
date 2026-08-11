//! 跨平台密钥存储与显式降级链。
//!
//! # 为什么是 `StorageReport` 而不是一个扁平的来源枚举
//!
//! 一个 `keychain` 取值会同时覆盖「Secret Service 里持久保存的条目」和「Linux keyutils
//! 里重启即失效的条目」——两者都是操作系统提供的钥匙串，但持久性完全不同。设置界面
//! 若只拿到 `keychain`，就会对一个重启就没的密钥宣称「系统钥匙串」，用户重启后发现要
//! 重新粘一次 key，而界面此前明确说过不用。因此本模块返回四个正交维度：后端、持久性、
//! 保护方式、可显示位置；界面文案由**持久性**派生，不看后端名字。
//!
//! # 降级链
//!
//! 1. 操作系统钥匙串（Linux 先试 Secret Service，失败退内核 keyutils）；
//! 2. 本进程会话内存（[`secrecy::SecretString`]，drop 时 zeroize）；
//! 3. 用户指定的明文文件，**仅在显式 opt-in 时**，Unix 上 mode `0600`、Windows 上收紧
//!    到当前用户的 ACL，并附明文警告。
//!
//! 第 2 层永不失败，所以第 3 层不会被「自动」走到——`allow_plaintext_file` 这个显式
//! opt-in 就是选择第 3 层的唯一机制，其语义是「我要它活过重启，并接受明文代价」。
//!
//! # keyutils 是内存型的
//!
//! 上游文档要求调用方「prepare for `Entry::get_password` to fail」。因此本模块把
//! 「密钥不存在」建模成 [`Lookup::Absent`] 这一**正常返回**而非错误，并由
//! [`Lookup::needs_reprompt`] 明确要求重新索要。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use keyring_core::api::CredentialPersistence;
use keyring_core::{CredentialStore, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use yunjian_core::{Error, Result, redact_credentials};
use zeroize::Zeroize;

/// 默认服务名。keyutils 会把它拼进 description，Secret Service 拿它当属性。
pub const DEFAULT_SERVICE: &str = "yunjian";

/// 明文降级文件的默认文件名。名字里带 `plaintext` 是刻意的：用户在文件管理器里看到它
/// 就该知道里面没有加密。
pub const PLAINTEXT_FILE_NAME: &str = "credentials.plaintext.toml";

/// 密钥实际所在的后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// Linux Secret Service（GNOME Keyring / KWallet），经 D-Bus。
    SecretService,
    /// Linux 内核 keyutils。纯内存，重启即失效。
    Keyutils,
    /// Windows 凭据管理器。
    WindowsCredential,
    /// macOS / iOS 钥匙串。
    AppleKeychain,
    /// Android Keystore 加 SharedPreferences。
    AndroidKeystore,
    /// 本进程内存，退出即失效。
    SessionMemory,
    /// 明文文件。仅在显式 opt-in 后使用。
    PlaintextFile,
    /// 没有存储任何东西——该密钥不存在。
    Absent,
}

impl Backend {
    /// 稳定的机器可读标识。界面与 FFI 两侧都以此为契约，不要改动取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecretService => "secret_service",
            Self::Keyutils => "keyutils",
            Self::WindowsCredential => "windows_credential",
            Self::AppleKeychain => "apple_keychain",
            Self::AndroidKeystore => "android_keystore",
            Self::SessionMemory => "session_memory",
            Self::PlaintextFile => "plaintext_file",
            Self::Absent => "absent",
        }
    }
}

/// 密钥能活多久。**界面文案只许从这里派生。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    /// 活到显式删除，跨重启。
    Persistent,
    /// 活到注销或重启。keyutils 属于这一档。
    LoginSession,
    /// 活到本进程退出。
    ProcessOnly,
    /// 什么都没存。
    None,
}

impl Persistence {
    /// 稳定的机器可读标识。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::LoginSession => "login_session",
            Self::ProcessOnly => "process_only",
            Self::None => "none",
        }
    }

    /// 把 `keyring-core` 的持久性描述映射到本模块的四档。
    ///
    /// 未知取值一律**保守**落到 [`Self::LoginSession`]：`CredentialPersistence` 是
    /// `#[non_exhaustive]` 的，上游将来新增一档时，把它当成 [`Self::Persistent`] 会让界面
    /// 替一个未知语义的后端做出持久承诺，那正是本模块要防的事。
    pub fn from_credential_persistence(value: CredentialPersistence) -> Self {
        match value {
            CredentialPersistence::UntilDelete => Self::Persistent,
            CredentialPersistence::UntilReboot | CredentialPersistence::UntilLogout => {
                Self::LoginSession
            }
            CredentialPersistence::ProcessOnly | CredentialPersistence::EntryOnly => {
                Self::ProcessOnly
            }
            _ => Self::LoginSession,
        }
    }
}

/// 密钥受什么保护。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    /// 由操作系统加密存储。
    OsEncrypted,
    /// 只在本进程内存里。
    ProcessMemory,
    /// 明文，任何能读到该文件的程序都能拿到。
    Plaintext,
}

impl Protection {
    /// 稳定的机器可读标识。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OsEncrypted => "os_encrypted",
            Self::ProcessMemory => "process_memory",
            Self::Plaintext => "plaintext",
        }
    }
}

/// 一次存储操作的诚实描述。**不含任何密钥材料。**
///
/// 当 `backend` 为 [`Backend::Absent`] 时，`protection` 与 `location` 描述的是**被查询的
/// 那一层**（即「若存在会在哪里、受什么保护」），因为此时没有「被保护的东西」可描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageReport {
    /// 实际后端。
    pub backend: Backend,
    /// 能活多久。
    pub persistence: Persistence,
    /// 受什么保护。
    pub protection: Protection,
    /// 可显示的非机密位置：路径或后端名。
    pub location: String,
}

impl StorageReport {
    /// 设置界面展示的那一句话。
    ///
    /// **由 `persistence` 与 `protection` 派生，刻意不看 `backend` 的名字**：keyutils 与
    /// Secret Service 都是「操作系统钥匙串」，但只有后者配得上「系统钥匙串」这个说法。
    /// 只有同时满足「持久」与「操作系统加密」才允许出现「系统钥匙串」四个字。
    pub fn settings_summary(&self) -> String {
        match (self.persistence, self.protection) {
            (Persistence::Persistent, Protection::OsEncrypted) => {
                format!("系统钥匙串（{}）：重启后依然有效", self.location)
            }
            (Persistence::LoginSession, _) => format!(
                "内核会话密钥环（{}）：**重启或注销后失效**，届时需要重新输入密钥",
                self.location
            ),
            (Persistence::ProcessOnly, _) => format!(
                "仅本次运行的内存（{}）：**退出即失效**，下次启动需要重新输入密钥",
                self.location
            ),
            (Persistence::None, _) => {
                format!("未存储（{}）：需要输入密钥", self.location)
            }
            (Persistence::Persistent, Protection::Plaintext) => format!(
                "明文文件（{}）：**未加密**，任何能读到该文件的程序都能拿到密钥",
                self.location
            ),
            (Persistence::Persistent, Protection::ProcessMemory) => format!(
                "保护方式与持久性不一致（{}）：按不可信处理，请重新输入密钥",
                self.location
            ),
        }
    }

    /// 明文存储警告。非明文时为 [`None`]，界面据此决定是否显示告警条。
    pub fn plaintext_warning(&self) -> Option<String> {
        (self.protection == Protection::Plaintext).then(|| {
            format!(
                "密钥以明文保存在 {}。该文件未加密，仅靠文件权限保护；\
                 一旦被备份、同步或打包带走，密钥即随之泄露。",
                self.location
            )
        })
    }
}

/// 一次读取的结果。「未找到」是正常返回，不是错误。
#[derive(Debug, Clone)]
pub enum Lookup {
    /// 找到了。
    Found {
        /// 存储描述。
        report: StorageReport,
        /// 密钥。
        secret: SecretString,
    },
    /// 没找到。keyutils 重启后、或用户从未设置过，都会走到这里。
    Absent {
        /// 存储描述，`backend` 为 [`Backend::Absent`]。
        report: StorageReport,
    },
}

impl Lookup {
    /// 是否需要重新向用户索要密钥。
    ///
    /// 这是 keyutils「内存型、不跨重启」这一事实在 API 上的落点：调用方不得假定持久，
    /// 每条读路径都要照这个信号走重新索要，而不是把它当异常抛出去。
    pub fn needs_reprompt(&self) -> bool {
        matches!(self, Self::Absent { .. })
    }

    /// 存储描述。
    pub fn report(&self) -> &StorageReport {
        match self {
            Self::Found { report, .. } | Self::Absent { report } => report,
        }
    }

    /// 取出密钥；未找到时为 [`None`]。
    pub fn secret(&self) -> Option<&SecretString> {
        match self {
            Self::Found { secret, .. } => Some(secret),
            Self::Absent { .. } => None,
        }
    }
}

/// 已选定的操作系统钥匙串。
pub struct OsKeychain {
    store: Arc<CredentialStore>,
    backend: Backend,
    persistence: Persistence,
    location: String,
}

impl std::fmt::Debug for OsKeychain {
    /// 手写而非 derive：`CredentialStore` 的 `Debug` 由各平台 store 自行实现，内容不在
    /// 本项目控制之下，不放进渲染结果。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsKeychain")
            .field("backend", &self.backend)
            .field("persistence", &self.persistence)
            .field("location", &self.location)
            .finish()
    }
}

impl OsKeychain {
    /// 接管一个已建好的凭据 store。
    ///
    /// `persistence` 一律向 store 本人问（`CredentialStoreApi::persistence`），不按后端名
    /// 硬编码：上游最清楚自己的凭据能活多久，硬编码只会在上游改语义时静默说谎。
    pub fn adopt(
        store: Arc<CredentialStore>,
        backend: Backend,
        location: impl Into<String>,
    ) -> Self {
        let persistence = Persistence::from_credential_persistence(store.persistence());
        Self {
            store,
            backend,
            persistence,
            location: location.into(),
        }
    }

    /// 后端。
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// 持久性。
    pub fn persistence(&self) -> Persistence {
        self.persistence
    }

    /// 可显示位置。
    pub fn location(&self) -> &str {
        &self.location
    }

    fn report(&self) -> StorageReport {
        StorageReport {
            backend: self.backend,
            persistence: self.persistence,
            protection: Protection::OsEncrypted,
            location: self.location.clone(),
        }
    }
}

/// 会话内存层：只在本进程内保存，drop 时由 [`SecretString`] 负责 zeroize。
#[derive(Debug, Default)]
struct SessionMemory {
    slots: Mutex<BTreeMap<String, SecretString>>,
}

/// 明文文件层。
#[derive(Debug)]
struct PlaintextFile {
    path: PathBuf,
    opt_in: bool,
}

enum Tier {
    Keychain(OsKeychain),
    Session(SessionMemory),
    File(PlaintextFile),
}

/// 打开密钥存储的参数。
#[derive(Debug, Clone)]
pub struct KeyStoreConfig {
    /// 服务名。
    pub service: String,
    /// **显式 opt-in**：允许把密钥写进明文文件。默认 `false`。
    pub allow_plaintext_file: bool,
    /// 明文文件路径。为 [`None`] 时取 `$XDG_CONFIG_HOME/yunjian/` 下的默认名。
    pub plaintext_path: Option<PathBuf>,
}

impl Default for KeyStoreConfig {
    fn default() -> Self {
        Self {
            service: DEFAULT_SERVICE.to_string(),
            allow_plaintext_file: false,
            plaintext_path: None,
        }
    }
}

/// 密钥存储。
pub struct KeyStore {
    service: String,
    tier: Tier,
}

impl std::fmt::Debug for KeyStore {
    /// 手写而非 derive：会话层持有 [`SecretString`]，虽然它自身的 `Debug` 已经打
    /// 星号，但把整个层结构暴露进渲染结果没有任何收益。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStore")
            .field("service", &self.service)
            .field("tier", &self.tier_report())
            .finish()
    }
}

impl KeyStore {
    /// 按降级链打开。
    ///
    /// 会在选定操作系统钥匙串时调用 [`keyring_core::set_default_store`]，让进程内其他
    /// 通过 `Entry::new` 建条目的代码走同一个 store。
    pub fn open(config: KeyStoreConfig) -> Result<Self> {
        let tier = match install_default_store() {
            Some(keychain) => Tier::Keychain(keychain),
            None if config.allow_plaintext_file => {
                let path = match config.plaintext_path.clone() {
                    Some(path) => path,
                    None => default_plaintext_path()?,
                };
                Tier::File(PlaintextFile { path, opt_in: true })
            }
            None => Tier::Session(SessionMemory::default()),
        };
        Ok(Self {
            service: config.service,
            tier,
        })
    }

    /// 直接用给定的凭据 store 建立存储，跳过平台探测。
    ///
    /// 测试用 `keyring_core::mock::Store` 走这条路；todo 37 需要指定 store 时也走这里。
    pub fn from_credential_store(
        store: Arc<CredentialStore>,
        service: impl Into<String>,
        backend: Backend,
        location: impl Into<String>,
    ) -> Self {
        Self {
            service: service.into(),
            tier: Tier::Keychain(OsKeychain::adopt(store, backend, location)),
        }
    }

    /// 建立只用本进程内存的存储。
    pub fn session_memory(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            tier: Tier::Session(SessionMemory::default()),
        }
    }

    /// 建立明文文件存储。
    ///
    /// `opt_in` 为 `false` 时，[`Self::set`] 会拒绝写入并报错——这一层的拒绝不依赖调用方
    /// 记得检查开关。
    pub fn plaintext_file(
        service: impl Into<String>,
        path: impl Into<PathBuf>,
        opt_in: bool,
    ) -> Self {
        Self {
            service: service.into(),
            tier: Tier::File(PlaintextFile {
                path: path.into(),
                opt_in,
            }),
        }
    }

    /// 当前所处层的描述，与任何具体密钥无关。
    pub fn tier_report(&self) -> StorageReport {
        match &self.tier {
            Tier::Keychain(keychain) => keychain.report(),
            Tier::Session(_) => session_report(),
            Tier::File(file) => file.report(),
        }
    }

    /// 读取密钥。未找到时返回 [`Lookup::Absent`]，不是错误。
    pub fn get(&self, account: &str) -> Result<Lookup> {
        match &self.tier {
            Tier::Keychain(keychain) => {
                let entry = self.entry(keychain, account)?;
                match entry.get_password() {
                    Ok(mut password) => {
                        let secret = SecretString::from(password.clone());
                        password.zeroize();
                        Ok(Lookup::Found {
                            report: keychain.report(),
                            secret,
                        })
                    }
                    Err(KeyringError::NoEntry) => Ok(Lookup::Absent {
                        report: absent_report(keychain.location(), Protection::OsEncrypted),
                    }),
                    Err(err) => Err(keystore_error("读取", &err)),
                }
            }
            Tier::Session(session) => {
                let slots = session.lock()?;
                match slots.get(account) {
                    Some(secret) => Ok(Lookup::Found {
                        report: session_report(),
                        secret: secret.clone(),
                    }),
                    None => Ok(Lookup::Absent {
                        report: absent_report(SESSION_LOCATION, Protection::ProcessMemory),
                    }),
                }
            }
            Tier::File(file) => {
                let mut entries = file.read()?;
                let found = entries.get(account).map(|value| {
                    let secret = SecretString::from(value.clone());
                    Lookup::Found {
                        report: file.report(),
                        secret,
                    }
                });
                zeroize_values(&mut entries);
                Ok(found.unwrap_or_else(|| Lookup::Absent {
                    report: absent_report(file.display_path(), Protection::Plaintext),
                }))
            }
        }
    }

    /// 写入密钥。
    pub fn set(&self, account: &str, secret: &SecretString) -> Result<StorageReport> {
        match &self.tier {
            Tier::Keychain(keychain) => {
                let entry = self.entry(keychain, account)?;
                entry
                    .set_password(secret.expose_secret())
                    .map_err(|err| keystore_error("写入", &err))?;
                Ok(keychain.report())
            }
            Tier::Session(session) => {
                let mut slots = session.lock()?;
                slots.insert(account.to_string(), secret.clone());
                Ok(session_report())
            }
            Tier::File(file) => {
                file.require_opt_in()?;
                let mut entries = file.read()?;
                entries.insert(account.to_string(), secret.expose_secret().to_string());
                let result = file.write(&entries);
                zeroize_values(&mut entries);
                result?;
                Ok(file.report())
            }
        }
    }

    /// 删除密钥。幂等：本来就不存在时返回 [`Backend::Absent`] 的报告而非报错。
    pub fn delete(&self, account: &str) -> Result<StorageReport> {
        match &self.tier {
            Tier::Keychain(keychain) => {
                let entry = self.entry(keychain, account)?;
                match entry.delete_credential() {
                    Ok(()) => Ok(keychain.report()),
                    Err(KeyringError::NoEntry) => {
                        Ok(absent_report(keychain.location(), Protection::OsEncrypted))
                    }
                    Err(err) => Err(keystore_error("删除", &err)),
                }
            }
            Tier::Session(session) => {
                let mut slots = session.lock()?;
                match slots.remove(account) {
                    Some(_) => Ok(session_report()),
                    None => Ok(absent_report(SESSION_LOCATION, Protection::ProcessMemory)),
                }
            }
            Tier::File(file) => {
                file.require_opt_in()?;
                let mut entries = file.read()?;
                let mut removed = entries.remove(account);
                let existed = removed.is_some();
                if let Some(value) = removed.as_mut() {
                    value.zeroize();
                }
                let result = file.write(&entries);
                zeroize_values(&mut entries);
                result?;
                Ok(if existed {
                    file.report()
                } else {
                    absent_report(file.display_path(), Protection::Plaintext)
                })
            }
        }
    }

    fn entry(&self, keychain: &OsKeychain, account: &str) -> Result<keyring_core::Entry> {
        keychain
            .store
            .build(&self.service, account, None)
            .map_err(|err| keystore_error("建立条目", &err))
    }
}

const SESSION_LOCATION: &str = "本进程内存（yunjian 会话）";

fn session_report() -> StorageReport {
    StorageReport {
        backend: Backend::SessionMemory,
        persistence: Persistence::ProcessOnly,
        protection: Protection::ProcessMemory,
        location: SESSION_LOCATION.to_string(),
    }
}

fn absent_report(location: impl Into<String>, protection: Protection) -> StorageReport {
    StorageReport {
        backend: Backend::Absent,
        persistence: Persistence::None,
        protection,
        location: location.into(),
    }
}

/// 把外部 keystore 错误收进 [`Error::Ai`]。
///
/// 走 `Error::ai` 而不是新造错误类型，是为了继承它构造期就跑一遍
/// [`redact_credentials`] 的那道防线：平台错误信息里可能夹带请求内容。
fn keystore_error(action: &str, err: &KeyringError) -> Error {
    Error::ai("keystore", format!("{action}凭据失败：{err}"))
}

impl SessionMemory {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, SecretString>>> {
        self.slots
            .lock()
            .map_err(|_| Error::ai("keystore", "会话内存锁已中毒"))
    }
}

fn zeroize_values(entries: &mut BTreeMap<String, String>) {
    for value in entries.values_mut() {
        value.zeroize();
    }
    entries.clear();
}

impl PlaintextFile {
    fn report(&self) -> StorageReport {
        StorageReport {
            backend: Backend::PlaintextFile,
            persistence: Persistence::Persistent,
            protection: Protection::Plaintext,
            location: self.display_path(),
        }
    }

    fn display_path(&self) -> String {
        self.path.display().to_string()
    }

    fn require_opt_in(&self) -> Result<()> {
        if self.opt_in {
            return Ok(());
        }
        Err(Error::ai(
            "keystore",
            format!(
                "拒绝把密钥写入明文文件 {}：需要显式开启 allow_plaintext_file",
                self.display_path()
            ),
        ))
    }

    fn read(&self) -> Result<BTreeMap<String, String>> {
        let mut text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(err) => {
                return Err(Error::ai(
                    "keystore",
                    format!("读取 {} 失败：{err}", self.display_path()),
                ));
            }
        };
        let parsed = toml::from_str::<BTreeMap<String, String>>(&text);
        text.zeroize();
        parsed.map_err(|err| {
            Error::ai(
                "keystore",
                format!("解析 {} 失败：{err}", self.display_path()),
            )
        })
    }

    fn write(&self, entries: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            create_private_dir(parent).map_err(|err| {
                Error::ai(
                    "keystore",
                    format!("创建目录 {} 失败：{err}", parent.display()),
                )
            })?;
        }
        let mut text = toml::to_string(entries).map_err(|err| {
            Error::ai(
                "keystore",
                format!("序列化 {} 失败：{err}", self.display_path()),
            )
        })?;
        let result = write_private_file(&self.path, text.as_bytes());
        text.zeroize();
        result.map_err(|err| {
            Error::ai(
                "keystore",
                format!("写入 {} 失败：{err}", self.display_path()),
            )
        })
    }
}

/// 建立存放明文密钥的目录。
///
/// **已存在的目录不改权限**：那通常是用户的配置目录，里面还有别的文件，静默把它收到
/// 0700 是越权。真正的保护落在文件本身的 0600 上（[`write_private_file`] 保证，含接管
/// 既存宽松文件的情形）；目录的 0700 只是新建时顺手加上的一层。
#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    if dir.is_dir() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

/// 建立（或截断）一个只有当前用户可读写的文件，然后写入内容。
///
/// 两个平台都遵守同一条时序：**先把权限收紧，再写入密钥**。顺序颠倒会留下一个「文件已
/// 含密钥、权限却还是继承来的宽松值」的时间窗，另一个用户在那个窗口里就能读到。
#[cfg(unix)]
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    // `mode` 只在本次调用真的创建了文件时生效；文件已存在时它被忽略，所以下面还要
    // 显式 `set_permissions` 一次，否则接管一个 0644 的旧文件会静默保持宽松权限。
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(windows)]
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    restrict_to_current_user(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// 把文件的 ACL 收紧到只有当前用户可读写。
///
/// POSIX mode bits 在 Windows 上没有意义（`set_permissions` 只能切只读位），所以这里必须
/// 重建 DACL：`/inheritance:r` 丢掉从父目录继承来的 ACE，`/grant:r` 把权限**重置**为只有
/// 当前用户可读写。走系统自带的 `icacls` 而不是 `windows-sys` 的
/// `SetNamedSecurityInfoW`，是因为后者需要一大段本项目 CI 无法运行验证的 `unsafe`。
///
/// stdout 必须丢弃：本进程的 stdout 是 MCP 协议流，子进程往那里写一行就会毁掉会话。
/// `CREATE_NO_WINDOW` 则避免 GUI 进程里闪出一个黑框。
#[cfg(windows)]
fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(name)) if !domain.is_empty() && !name.is_empty() => {
            format!("{domain}\\{name}")
        }
        (_, Ok(name)) if !name.is_empty() => name,
        _ => {
            return Err(std::io::Error::other(
                "无法确定当前用户，拒绝在未收紧 ACL 的情况下写入明文密钥",
            ));
        }
    };

    let status = Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{user}:(R,W)"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;

    if status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "icacls 收紧 {} 的 ACL 失败（退出码 {:?}）",
        path.display(),
        status.code()
    )))
}

#[cfg(not(any(unix, windows)))]
fn write_private_file(_path: &Path, _bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "本平台没有可用的文件权限收紧手段，拒绝写入明文密钥",
    ))
}

/// 明文降级文件的默认路径。
pub fn default_plaintext_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().ok_or_else(|| Error::ai("keystore", "无法确定用户配置目录"))?;
    Ok(dir.join("yunjian").join(PLAINTEXT_FILE_NAME))
}

/// 探测并安装本平台的操作系统钥匙串。
///
/// 返回 [`None`] 表示本机没有可用的操作系统钥匙串——这在 CI、容器与纯 SSH 会话里是
/// 常态，不是异常。调用方据此走降级链。
pub fn install_default_store() -> Option<OsKeychain> {
    let keychain = probe_os_keychain()?;
    keyring_core::set_default_store(keychain.store.clone());
    Some(keychain)
}

fn log_store_failure(backend: &str, err: &KeyringError) {
    tracing::info!(
        backend,
        reason = %redact_credentials(&err.to_string()),
        "凭据 store 不可用，继续走降级链"
    );
}

/// Linux：先试 Secret Service，失败退内核 keyutils。
///
/// Secret Service 失败是**预期路径**：它要求 D-Bus 会话加一个真实的 secrets 守护进程，
/// 二者在无头环境里通常都没有。keyutils 属于内核、总是可用，代价是纯内存。
#[cfg(target_os = "linux")]
fn probe_os_keychain() -> Option<OsKeychain> {
    match zbus_secret_service_keyring_store::Store::new() {
        Ok(store) => {
            return Some(OsKeychain::adopt(
                store,
                Backend::SecretService,
                "Secret Service（D-Bus org.freedesktop.secrets）",
            ));
        }
        Err(err) => log_store_failure("secret_service", &err),
    }
    match linux_keyutils_keyring_store::Store::new() {
        Ok(store) => Some(OsKeychain::adopt(
            store,
            Backend::Keyutils,
            "Linux 内核 keyutils",
        )),
        Err(err) => {
            log_store_failure("keyutils", &err);
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn probe_os_keychain() -> Option<OsKeychain> {
    match apple_native_keyring_store::keychain::Store::new() {
        Ok(store) => Some(OsKeychain::adopt(
            store,
            Backend::AppleKeychain,
            "macOS 钥匙串",
        )),
        Err(err) => {
            log_store_failure("apple_keychain", &err);
            None
        }
    }
}

#[cfg(target_os = "ios")]
fn probe_os_keychain() -> Option<OsKeychain> {
    match apple_native_keyring_store::protected::Store::new() {
        Ok(store) => Some(OsKeychain::adopt(
            store,
            Backend::AppleKeychain,
            "iOS 钥匙串（protected data）",
        )),
        Err(err) => {
            log_store_failure("apple_keychain", &err);
            None
        }
    }
}

#[cfg(target_os = "windows")]
fn probe_os_keychain() -> Option<OsKeychain> {
    match windows_native_keyring_store::Store::new() {
        Ok(store) => Some(OsKeychain::adopt(
            store,
            Backend::WindowsCredential,
            "Windows 凭据管理器",
        )),
        Err(err) => {
            log_store_failure("windows_credential", &err);
            None
        }
    }
}

/// Android：用新的命名 store（`by_store`），不用已废弃的 `by_service` legacy API。
///
/// 宿主应用必须**先**初始化 `ndk-context` 的 application context，否则这里必然失败——
/// 上游从那里取 JNI 环境。未初始化时返回 [`None`] 而不是 panic，降级链照常往下走。
#[cfg(target_os = "android")]
fn probe_os_keychain() -> Option<OsKeychain> {
    match android_native_keyring_store::Store::new() {
        Ok(store) => Some(OsKeychain::adopt(
            store,
            Backend::AndroidKeystore,
            "Android Keystore（命名 store）",
        )),
        Err(err) => {
            log_store_failure("android_keystore", &err);
            None
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "android"
)))]
fn probe_os_keychain() -> Option<OsKeychain> {
    None
}

#[cfg(test)]
mod tests;
