//! 语音模型的按需下载、许可门禁与本地缓存。
//!
//! # 为什么不随安装包分发
//!
//! 许可干净的两把中文音色是 163 MB 与 349 MB，识别侧最小的也有 116 MB。打进安装包
//! 它们会主导下载体积，而且会把各自的署名义务拖进**每一个**分发件——包括根本不开
//! 语音的那些。所以权重按需下载，安装包里一个字节都没有（`.gitignore` 已排除
//! `/models/cache/`）。
//!
//! # 分层：判定与传输分开
//!
//! 与 [`crate::audio`] 同一条缝，理由也相同——**让载重逻辑能在没有网络的机器上被验证**：
//!
//! - **判定层，不带任何特性开关**：[`Registry`]、[`ModelError`]、许可与拒绝名单门禁、
//!   缓存路径解析、摘要校验、原子落地、缺失时的降级信号。全部由 `make ci` 的默认构建
//!   编译并跑测试。
//! - **传输层，`download` 特性**：[`HttpTransport`] 与 [`Bz2TarUnpacker`]。只有这两个
//!   薄壳需要 HTTP 客户端与解压库。
//!
//! [`ensure_model_with`] 把传输与解包都收成 `Option<&dyn _>`，于是「没有下载能力」
//! 不是另一条代码路径，而是同一条路径上的一个 `None`——那正是**模型缺失且无网络时
//! 降级到打字练习**这条要求的落点，`Option` 为 `None` 时它必然走到
//! [`ModelError::Absent`]，而 [`ModelError::practice`] 对每个变体都给打字练习。
//!
//! # 清单为什么在编译期内联
//!
//! `models.toml` 与 `models/DENYLIST.md` 用 [`include_str!`] 读进二进制，不在运行时读盘。
//! 决定性理由是发布形态：装好的二进制旁边没有仓库检出，运行时去找 `models.toml`
//! 必然失败。顺带的好处是许可门禁没有可被替换的外部输入——想放行一个被拒的模型，
//! 只能改仓库里的文件并重新构建，改不动已经发出去的那一份。
//!
//! # 与 `xtask verify-models` 的分工
//!
//! 那条命令校验的是**清单本身是否可信**：打开随仓的许可证据文件、核对 SPDX 标记与
//! SHA-256、断言拒绝名单没被删条目。本模块信任已经过那道门禁的清单，只做运行时该做
//! 的三件事：拒绝名单命中即拒、许可不在允许列表即拒、下载的字节摘要不符即拒。

use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::permission::{DegradeReason, Practice, explain};

/// 签入的权重清单，编译期内联。
const MANIFEST_SOURCE: &str = include_str!("../../../models.toml");

/// 签入的拒绝清单，编译期内联。
const DENYLIST_SOURCE: &str = include_str!("../../../models/DENYLIST.md");

/// 拒绝清单里唯一被解析的小节标题。与 `xtask verify-models` 逐字相同。
const DENYLIST_SECTION: &str = "## 拒绝清单";

/// 本模块能读的清单版本。未知版本一律拒绝而不是尽力解析——静默接受等于放弃校验。
pub const SCHEMA_VERSION: u32 = 1;

/// 权重许可的允许列表。比语料那边窄，且不设例外。
pub const ALLOWED_LICENSES: [&str; 2] = ["MIT", "Apache-2.0"];

/// 方案点名必须永远在拒绝名单里的标识符。
///
/// 存在的理由与 `xtask verify-models` 的 `REQUIRED_DENYLIST` 一致：**只维护一份清单
/// 不够，删掉一行就能放行**。这里的每一条都由测试断言仍出现在 `models/DENYLIST.md`
/// 里，删条目直接让测试失败。
pub const REQUIRED_DENIED: [&str; 5] = [
    "matcha-icefall-zh-baker",
    "vits-zh-hf-",
    "aishell3",
    "MCGA",
    "edge-tts",
];

/// 发布包的归档扩展名。清单里的 `sha256` 与 `size_bytes` 都是**压缩包**的。
const ARCHIVE_SUFFIX: &str = ".tar.bz2";

/// 已校验归档在缓存里的子目录。
const ARCHIVE_DIR: &str = "archives";

/// 落地中的临时名后缀。临时名永远不等于最终名，所以半成品不会被后续运行误认。
const TEMP_SUFFIX: &str = ".partial";

/// 读写归档与摘要用的缓冲区大小。
const IO_BUFFER_BYTES: usize = 1 << 16;

/// 进度上报的字节间隔。太密会让日志刷屏，太疏会让 600 MB 的下载看起来卡死。
const REPORT_BYTES: u64 = 4 << 20;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 模型用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// 语音识别。
    Asr,
    /// 语音合成。
    Tts,
}

impl ModelKind {
    /// 清单与输出里用的键名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asr => "asr",
            Self::Tts => "tts",
        }
    }
}

/// 模型是否进产品路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// 投产候选。
    Production,
    /// 仅用于构建冒烟，不进产品路径。
    Smoke,
}

impl ModelRole {
    /// 清单与输出里用的键名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Smoke => "smoke",
        }
    }
}

/// 发布包里夹带的第三方产物。
///
/// 逐项声明是必需的：`kokoro` 与 `kitten` 都夹带 GPL-3.0 的 espeak-ng 数据，
/// 那件事对分发形态的影响不能只活在某个人的记忆里。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledWork {
    /// 包内路径。
    pub path: String,
    /// 是谁的产物。
    pub work: String,
    /// SPDX。**不受 [`ALLOWED_LICENSES`] 约束**——夹带的 espeak-ng 数据是既成事实。
    pub license: String,
    /// 分发影响。非允许列表的许可必须写明，由 `xtask verify-models` 强制。
    #[serde(default)]
    pub distribution_impact: Option<String>,
}

/// 清单里的一个发布包。
///
/// 字段与 `models.toml` 逐一对应，且开了 `deny_unknown_fields`：清单新增字段时这里
/// 编译不过，于是「加了个字段但运行时不知道」不可能发生。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    /// 发布包名，全局唯一，同时是缓存目录名。
    pub name: String,
    /// 用途。
    pub kind: ModelKind,
    /// 是否进产品路径。
    pub role: ModelRole,
    /// 下载地址。
    pub url: String,
    /// **压缩包**的 SHA-256。按需下载校验的就是它。
    pub sha256: String,
    /// **压缩包**的字节数。
    pub size_bytes: u64,
    /// SPDX。只有 [`ALLOWED_LICENSES`] 里的两个会被放行。
    pub license: String,
    /// 许可证据的锁定 URL。
    pub license_url: String,
    /// 证据所在的 commit SHA。
    pub license_rev: String,
    /// 随仓保存的证据副本路径。
    pub license_file: String,
    /// 上一项的 SHA-256。
    pub license_sha256: String,
    /// 证据形态。**取值域由 `xtask verify-models` 判定**，运行时不据此做决定，
    /// 因此这里是字符串而不是枚举——再抄一份枚举只会多一处可漂移的定义。
    pub license_evidence: String,
    /// 原始权重是谁训练的、许可链怎么走过来的。
    pub underlying_work: String,
    /// 本行的核实日期。
    pub verified_at: String,
    /// 必要的限定说明。
    #[serde(default)]
    pub note: Option<String>,
    /// 夹带的第三方产物。
    #[serde(default)]
    pub bundled: Vec<BundledWork>,
}

impl ModelEntry {
    /// 该模型的许可是否在允许列表内。
    #[must_use]
    pub fn license_allowed(&self) -> bool {
        ALLOWED_LICENSES.contains(&self.license.as_str())
    }

    /// 随仓许可原文在 `licenses/` 下的文件名。
    ///
    /// 扩展名跟着证据形态走：模型卡是 Markdown，其余是 LICENSE 原文。
    #[must_use]
    pub fn attribution_file(&self) -> String {
        let ext = if self.license_evidence == "model_card" {
            ".CARD.md"
        } else {
            ".LICENSE"
        };
        format!("{}{ext}", self.name)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    #[serde(rename = "model", default)]
    models: Vec<ModelEntry>,
}

/// 拒绝名单的一条：标识符加它的理由原文。
///
/// 带上理由不是修饰。命中时报「详见 DENYLIST.md」等于让读者再翻一次文件，而理由本来
/// 就在同一行，顺手带出来的成本为零——`matcha-icefall-zh-baker` 被拒的理由是它的训练
/// 数据集写明非商用，那句话必须出现在报错里，否则拒绝看起来像是随意的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyEntry {
    /// 标识符。对 `name` 与 `url` 做大小写无关的子串匹配。
    pub id: String,
    /// 理由原文。
    pub reason: String,
}

/// 模型路径的失败面。**每一条都能降级到打字练习**，见 [`ModelError::practice`]。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    /// 清单里没有这个名字。
    #[error("清单里没有名为 `{name}` 的模型；可用的是：{}", known.join("、"))]
    Unknown {
        /// 请求的名字。
        name: String,
        /// 清单里实际有的名字。
        known: Vec<String>,
    },

    /// 命中拒绝名单。
    #[error(
        "模型 `{name}` 命中 models/DENYLIST.md 的拒绝条目 `{matched}`，不得加载。理由：{reason}"
    )]
    Denied {
        /// 请求的名字。
        name: String,
        /// 命中的拒绝条目标识符。
        matched: String,
        /// 拒绝理由原文。
        reason: String,
    },

    /// 许可不在允许列表内。
    #[error(
        "模型 `{name}` 的许可 `{license}` 不在允许列表（{}）内，不得加载",
        ALLOWED_LICENSES.join(" / ")
    )]
    LicenseRefused {
        /// 请求的名字。
        name: String,
        /// 清单记录的 SPDX。
        license: String,
    },

    /// 本地没有，且这次拿不到。
    #[error("模型 `{name}` 本地不可用：{} 不存在；{next}", dir.display())]
    Absent {
        /// 请求的名字。
        name: String,
        /// 期望的模型目录。报目录名不够用，调用方拿到的报错要能直接 `ls`。
        dir: PathBuf,
        /// 下一步该做什么。
        next: String,
    },

    /// 摘要不符。
    #[error(
        "模型 `{name}` 的归档摘要 {actual} 与清单记录的 {expected} 不符；已中止，未留下任何文件"
    )]
    ChecksumMismatch {
        /// 请求的名字。
        name: String,
        /// 清单记录的摘要。
        expected: String,
        /// 实测摘要。
        actual: String,
    },

    /// 字节数不符。先比字节数是因为它几乎总意味着下载被截断，这个诊断比「摘要不符」具体。
    #[error("模型 `{name}` 的归档有 {actual} 字节，清单记录 {expected} 字节；下载可能被截断")]
    SizeMismatch {
        /// 请求的名字。
        name: String,
        /// 清单记录的字节数。
        expected: u64,
        /// 实测字节数。
        actual: u64,
    },

    /// 传输失败。
    #[error("下载模型 `{name}` 失败（{url}）：{detail}")]
    Download {
        /// 请求的名字。
        name: String,
        /// 下载地址。公开 CDN 地址，不含任何凭据。
        url: String,
        /// 底层原因。
        detail: String,
    },

    /// 解包失败。
    #[error("解包模型 `{name}` 失败：{detail}")]
    Unpack {
        /// 请求的名字。
        name: String,
        /// 底层原因。
        detail: String,
    },

    /// 缓存目录读写失败。
    #[error("模型缓存读写失败（{}）：{detail}", path.display())]
    Io {
        /// 出错的路径。
        path: PathBuf,
        /// 底层原因。
        detail: String,
    },

    /// 编译进来的清单本身解析不了。正常构建里到不了，留着是为了不用 panic 表达它。
    #[error("签入的模型清单无法解析：{detail}")]
    Manifest {
        /// 底层原因。
        detail: String,
    },
}

impl ModelError {
    /// 该错误对应的降级原因。
    ///
    /// **穷尽匹配**，与 [`crate::audio::AudioError::degrade_reason`] 同一个理由：新增
    /// 变体时编译器在这里报缺分支，于是「模型侧的任何失败都退回打字练习」由类型系统
    /// 保证而不是靠约定。全部落在同一个原因上是刻意的——原因决定界面归类，而具体
    /// 该做什么由 [`Display`] 说，那才是分得清十种失败的地方。
    ///
    /// [`Display`]: std::fmt::Display
    #[must_use]
    pub const fn degrade_reason(&self) -> DegradeReason {
        match self {
            Self::Unknown { .. }
            | Self::Denied { .. }
            | Self::LicenseRefused { .. }
            | Self::Absent { .. }
            | Self::ChecksumMismatch { .. }
            | Self::SizeMismatch { .. }
            | Self::Download { .. }
            | Self::Unpack { .. }
            | Self::Io { .. }
            | Self::Manifest { .. } => DegradeReason::ModelUnavailable,
        }
    }

    /// 这次失败之后该走哪条练习路径。**永远是打字练习，不是错误对话框，也不是零分。**
    ///
    /// 消息刻意由两段拼成：先是本错误的原文（哪个模型、哪条许可、缺在哪个目录），
    /// 再接 [`explain`] 给的通用引导。只给前者说不清怎么恢复，只给后者说不清是什么坏了。
    #[must_use]
    pub fn practice(&self) -> Practice {
        Practice::Typed {
            reason: self.degrade_reason(),
            message: format!("{self}。{}", explain(self.degrade_reason(), None)),
        }
    }
}

/// 清单与拒绝名单合起来的运行时视图。
#[derive(Debug, Clone)]
pub struct Registry {
    models: Vec<ModelEntry>,
    denied: Vec<DenyEntry>,
}

impl Registry {
    /// 编译进本二进制的那一份。只解析一次。
    ///
    /// 返回 `Result` 而不是 `panic`：清单是编译期常量，解析失败属于作者错误而非运行时
    /// 状况，但一个库不该用 panic 表达它。有一条测试断言真清单能解析，所以这条错误路径
    /// 在实际构建里到不了。
    pub fn shipped() -> Result<&'static Self, ModelError> {
        static SHIPPED: OnceLock<Result<Registry, String>> = OnceLock::new();
        SHIPPED
            .get_or_init(|| {
                Self::parse(MANIFEST_SOURCE, DENYLIST_SOURCE).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|detail| ModelError::Manifest {
                detail: detail.clone(),
            })
    }

    /// 由清单与拒绝名单的文本构造。
    ///
    /// # Errors
    ///
    /// 清单不是合法 TOML、`schema_version` 不是 [`SCHEMA_VERSION`]、名字重复，
    /// 或拒绝名单一条都解析不出来时返回 [`ModelError::Manifest`]。
    pub fn parse(manifest: &str, denylist: &str) -> Result<Self, ModelError> {
        let parsed: Manifest = toml::from_str(manifest).map_err(|error| ModelError::Manifest {
            detail: error.to_string(),
        })?;
        if parsed.schema_version != SCHEMA_VERSION {
            return Err(ModelError::Manifest {
                detail: format!(
                    "schema_version = {} 不是 {SCHEMA_VERSION}；本模块只认版本 \
                     {SCHEMA_VERSION}，静默接受未知版本等于放弃校验",
                    parsed.schema_version
                ),
            });
        }
        for (index, entry) in parsed.models.iter().enumerate() {
            if parsed.models[..index].iter().any(|m| m.name == entry.name) {
                return Err(ModelError::Manifest {
                    detail: format!("清单里出现重名的 model `{}`", entry.name),
                });
            }
        }

        let denied = parse_denylist(denylist);
        if denied.is_empty() {
            return Err(ModelError::Manifest {
                detail: format!(
                    "拒绝名单的 `{DENYLIST_SECTION}` 一节里没有解析出任何条目；\
                     空名单会让每个模型都通过拒绝检查"
                ),
            });
        }

        Ok(Self {
            models: parsed.models,
            denied,
        })
    }

    /// 清单里的全部条目，顺序与文件一致。
    #[must_use]
    pub fn entries(&self) -> &[ModelEntry] {
        &self.models
    }

    /// 拒绝名单里的全部条目。
    #[must_use]
    pub fn denied(&self) -> &[DenyEntry] {
        &self.denied
    }

    /// 按名字取条目，**不做许可与拒绝判定**。
    ///
    /// # Errors
    ///
    /// 名字不在清单里时返回 [`ModelError::Unknown`]。
    pub fn find(&self, name: &str) -> Result<&ModelEntry, ModelError> {
        self.models
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| ModelError::Unknown {
                name: name.to_owned(),
                known: self.models.iter().map(|m| m.name.clone()).collect(),
            })
    }

    /// 取条目并过许可门禁。**任何加载路径都必须经这里，不能用 [`Self::find`] 绕过。**
    ///
    /// # Errors
    ///
    /// 名字未知、命中拒绝名单，或许可不在 [`ALLOWED_LICENSES`] 内。
    pub fn admit(&self, name: &str) -> Result<&ModelEntry, ModelError> {
        let entry = self.find(name)?;
        self.gate(entry)?;
        Ok(entry)
    }

    /// 许可门禁本体。
    ///
    /// **顺序是先拒绝名单再许可**，不能反：被拒的包完全可能在清单里写着 `MIT`
    /// （`vits-zh-hf-*` 的声音取自游戏角色，来源授权与包上写的 SPDX 是两回事），
    /// 先判许可就会让它通过，而拒绝理由才是那个更具体、更该报出来的诊断。
    ///
    /// # Errors
    ///
    /// 命中拒绝名单返回 [`ModelError::Denied`]；许可不在允许列表返回
    /// [`ModelError::LicenseRefused`]。
    pub fn gate(&self, entry: &ModelEntry) -> Result<(), ModelError> {
        if let Some(hit) = self.deny_hit(entry) {
            return Err(ModelError::Denied {
                name: entry.name.clone(),
                matched: hit.id.clone(),
                reason: hit.reason.clone(),
            });
        }
        if !entry.license_allowed() {
            return Err(ModelError::LicenseRefused {
                name: entry.name.clone(),
                license: entry.license.clone(),
            });
        }
        Ok(())
    }

    /// 命中的拒绝条目；没命中为 `None`。
    ///
    /// 同时匹配 `name` 与 `url`：换个名字重新挂上同一个 URL 也要被拦住。
    #[must_use]
    pub fn deny_hit(&self, entry: &ModelEntry) -> Option<&DenyEntry> {
        let name = entry.name.to_lowercase();
        let url = entry.url.to_lowercase();
        self.denied.iter().find(|deny| {
            let needle = deny.id.to_lowercase();
            name.contains(&needle) || url.contains(&needle)
        })
    }
}

/// 解析 ``- `标识符` —— 理由`` 形式的列表项，只在 [`DENYLIST_SECTION`] 一节内。
///
/// 与 `xtask verify-models` 的 `parse_denylist` 是同一套语法，刻意重写而非共用：
/// `xtask` 不是本 crate 的依赖（也不该是，它 `publish = false`），而这条语法只有
/// 十几行。两处由测试保持一致——有一条用例断言真拒绝名单在这里解析出的条目数不为零，
/// 且 [`REQUIRED_DENIED`] 每一条都在其中。
fn parse_denylist(text: &str) -> Vec<DenyEntry> {
    let mut entries = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim() == DENYLIST_SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(entry) = backticked_list_item(line) {
            entries.push(entry);
        }
    }
    entries
}

fn backticked_list_item(line: &str) -> Option<DenyEntry> {
    let rest = line.strip_prefix("- `")?;
    let end = rest.find('`')?;
    let id = &rest[..end];
    if id.is_empty() {
        return None;
    }
    let reason = rest[end + 1..]
        .trim_start()
        .trim_start_matches("——")
        .trim_start_matches('—')
        .trim();
    Some(DenyEntry {
        id: id.to_owned(),
        reason: reason.to_owned(),
    })
}

/// 模型缓存根目录。`YUNJIAN_MODEL_DIR` 覆盖，默认为仓库内 `models/cache`。
///
/// 默认值在编译期由 `CARGO_MANIFEST_DIR` 推成绝对路径而不是写相对路径：
/// `cargo test -p yunjian-voice` 的工作目录是 crate 目录而非工作区根，相对路径会找错
/// 地方。`.gitignore` 已排除该路径，权重因此不会被误提交。
#[must_use]
pub fn cache_root() -> PathBuf {
    std::env::var_os("YUNJIAN_MODEL_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("models")
                .join("cache")
        },
        PathBuf::from,
    )
}

/// 一个模型缓存目录上的全部操作。
///
/// **根目录是构造时的入参，不是每次调用时去读环境变量。** 这不是为了灵活：
/// `std::env::set_var` 在 Rust 2024 里是 `unsafe`，而且是进程全局的，于是「靠环境变量
/// 指向临时目录」的测试既要写 `unsafe`（本工作区 `unsafe_code = "warn"`，`make ci` 带
/// `-D warnings`），又必须串行执行，否则两条用例会互删对方的缓存。把根目录变成入参之后
/// 这两个问题一起消失，测试可以并行，也不需要任何 `unsafe`。
#[derive(Debug, Clone)]
pub struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    /// 按 [`cache_root`] 解析根目录。
    #[must_use]
    pub fn discover() -> Self {
        Self { root: cache_root() }
    }

    /// 指定根目录。
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 缓存根目录。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 解包后的模型目录。这是 [`Self::ensure`] 返回的东西，也是识别器与合成器要的东西。
    #[must_use]
    pub fn model_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 已校验归档的落地路径。留在缓存内，于是 `.gitignore` 的
    /// `/models/cache/` 一条就把权重与归档都盖住了。
    #[must_use]
    pub fn archive_path(&self, name: &str) -> PathBuf {
        self.root
            .join(ARCHIVE_DIR)
            .join(format!("{name}{ARCHIVE_SUFFIX}"))
    }

    /// 模型目录是否就位。
    #[must_use]
    pub fn is_present(&self, name: &str) -> bool {
        is_populated(&self.model_dir(name))
    }
}

/// 目录存在且至少有一个条目。
///
/// 空目录不算就位：解包中途失败或有人手工 `mkdir` 都会留下一个空目录，把它当作
/// 「已下载」会让识别器在打开模型文件时才失败，那时报错离原因已经很远了。
fn is_populated(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// 一个模型此刻的本地状态，`yunjian models list` 的行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelStatus {
    /// 发布包名。
    pub name: String,
    /// 用途。
    pub kind: ModelKind,
    /// 是否进产品路径。
    pub role: ModelRole,
    /// SPDX。
    pub license: String,
    /// 归档字节数。
    pub size_bytes: u64,
    /// 解包后的模型目录是否就位。
    pub unpacked: bool,
    /// 已校验归档是否还在本地。
    pub archived: bool,
    /// 许可门禁的判定结果；通过为 `None`，被拒时是给用户看的原因。
    pub refused: Option<String>,
    /// 随仓许可原文的文件名。
    pub attribution: String,
}

impl ModelCache {
    /// 清单里每个模型在本缓存里的状态。
    ///
    /// **被拒的模型也列出来**，只是带上 `refused`。清单里此刻没有被拒的条目，但一旦有人
    /// 加进来，`list` 必须让它显形而不是把它藏起来——藏起来只会让人以为清单没被改过。
    ///
    /// # Errors
    ///
    /// 编译进来的清单解析不了时返回 [`ModelError::Manifest`]。
    pub fn statuses(&self) -> Result<Vec<ModelStatus>, ModelError> {
        let registry = Registry::shipped()?;
        Ok(registry
            .entries()
            .iter()
            .map(|entry| ModelStatus {
                name: entry.name.clone(),
                kind: entry.kind,
                role: entry.role,
                license: entry.license.clone(),
                size_bytes: entry.size_bytes,
                unpacked: self.is_present(&entry.name),
                archived: self.archive_path(&entry.name).is_file(),
                refused: registry.gate(entry).err().map(|error| error.to_string()),
                attribution: entry.attribution_file(),
            })
            .collect())
    }
}

/// 下载与落地的进度。
///
/// 刻意可 `Copy` 且不借用任何东西：调用方（CLI）要把它塞进
/// `yunjian_core::operation` 的有界队列跨线程送走，借调用栈上的路径送不出去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchProgress {
    /// 正在下载。`bytes_total == 0` 表示服务端没给长度。
    Downloading {
        /// 已写出的字节数。
        bytes_done: u64,
        /// 预期总字节数；未知时为零。
        bytes_total: u64,
    },
    /// 正在核对已落地归档的摘要。
    Verifying {
        /// 归档字节数。
        bytes: u64,
    },
    /// 摘要已核对通过。
    Verified,
    /// 正在解包。
    Unpacking,
}

/// 归档传输。
///
/// 抽象出来的理由是**让下载链在不带任何 HTTP 客户端的构建里也能被完整验证**：
/// 「写到同目录临时文件 → 核对摘要 → 原子改名 → 失败不留文件」这段逻辑与传输方式
/// 无关，而它才是这个模块最该被测到的部分。生产实现是 [`HttpTransport`]。
pub trait Transport {
    /// 把 `url` 的字节写进 `sink`，每写一段调用一次 `progress(已写字节, 总字节)`。
    ///
    /// 总字节未知时传 0。返回实际写出的字节数。
    ///
    /// # Errors
    ///
    /// 任何传输或写入失败，返回可直接给用户看的中文原因。
    fn fetch(
        &self,
        url: &str,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u64, String>;
}

/// 写入上限守卫。
///
/// 清单记录了归档的确切字节数，所以任何多出来的字节都已经是错的。没有这道闸，一个坏掉
/// 或恶意的服务端可以一直发下去，把磁盘填满之后才被摘要校验发现——那时错误已经造成。
/// 上限放在判定层而不是某个具体传输里，于是它对**每个** [`Transport`] 实现都成立，
/// 并且由默认构建的测试覆盖。
struct CappedWriter<'a> {
    inner: &'a mut dyn Write,
    limit: u64,
    written: u64,
}

impl Write for CappedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.written.saturating_add(buf.len() as u64);
        if next > self.limit {
            return Err(std::io::Error::other(format!(
                "服务端发出的字节数超过清单记录的 {} 字节，已在第 {next} 字节处中止",
                self.limit
            )));
        }
        let wrote = self.inner.write(buf)?;
        self.written = self.written.saturating_add(wrote as u64);
        Ok(wrote)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// 归档解包。生产实现是 [`Bz2TarUnpacker`]。
pub trait Unpacker {
    /// 把 `archive` 解到 `into` 目录下。`into` 由调用方建好，且是个临时目录。
    ///
    /// # Errors
    ///
    /// 解压失败、归档结构异常，或条目路径试图逃出 `into`。
    fn unpack(&self, archive: &Path, into: &Path) -> Result<(), String>;
}

/// 确保模型在本地就位，返回它的目录。
///
/// 解析顺序：已解包的目录 → 已校验的归档 → 下载。**第一级命中时不发起任何网络请求。**
///
/// 没有编译 `download` 特性时没有传输与解包能力，于是本地缺失必然落到
/// [`ModelError::Absent`]，调用方拿它的 [`ModelError::practice`] 得到打字练习。
///
/// # Errors
///
/// 见 [`ModelError`]。任何一种都可以降级到打字练习。
pub fn ensure_model(name: &str) -> Result<PathBuf, ModelError> {
    ModelCache::discover().ensure(name, &mut |_| {})
}

impl ModelCache {
    /// 确保模型在本缓存里就位，返回它的目录。用编译进来的默认传输与解包器。
    ///
    /// # Errors
    ///
    /// 见 [`ModelError`]。
    pub fn ensure(
        &self,
        name: &str,
        progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PathBuf, ModelError> {
        self.ensure_with(
            name,
            default_transport().as_deref(),
            default_unpacker().as_deref(),
            progress,
        )
    }

    /// [`Self::ensure`] 的可注入版本。
    ///
    /// `transport` 与 `unpacker` 为 `None` 表示本次没有该能力——不是另一条代码路径，
    /// 而是同一条路径上的 `None`，于是「没有下载能力」必然落到 [`ModelError::Absent`]。
    ///
    /// # Errors
    ///
    /// 见 [`ModelError`]。
    pub fn ensure_with(
        &self,
        name: &str,
        transport: Option<&dyn Transport>,
        unpacker: Option<&dyn Unpacker>,
        progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PathBuf, ModelError> {
        let registry = Registry::shipped()?;
        let entry = registry.admit(name)?;

        let dir = self.model_dir(name);
        if is_populated(&dir) {
            return Ok(dir);
        }

        let archive = self.archive_path(name);
        if archive.is_file() {
            // 已有归档仍然要核对摘要。**绝不加载未校验的下载**：上一次运行可能在改名
            // 之后、解包之前被杀，也可能有人手工放了一个文件进来。
            let bytes = file_len(&archive)?;
            progress(FetchProgress::Verifying { bytes });
            verify_archive_bytes(entry, &archive)?;
            progress(FetchProgress::Verified);
        } else {
            let Some(transport) = transport else {
                return Err(ModelError::Absent {
                    name: name.to_owned(),
                    dir,
                    next: format!(
                        "本构建未编译模型下载能力（`download` 特性）；请在带该特性的构建里运行 \
                         `yunjian models fetch {name}`，或按 docs/VOICE-BUILD.zh.md 手工放置"
                    ),
                });
            };
            download_verified(entry, &archive, transport, progress)?;
        }

        let Some(unpacker) = unpacker else {
            return Err(ModelError::Absent {
                name: name.to_owned(),
                dir,
                next: format!(
                    "已校验的归档在 {}，但本构建未编译解包能力（`download` 特性）；\
                     可手工 `tar xf` 到 {}",
                    archive.display(),
                    self.root.display()
                ),
            });
        };
        progress(FetchProgress::Unpacking);
        unpack_atomically(name, unpacker, &archive, &dir)?;
        tracing::info!(
            model = name,
            dir = %dir.display(),
            license = entry.license,
            "模型已校验并解包就位"
        );
        Ok(dir)
    }

    /// 核对本缓存里归档的摘要。返回 `None` 表示本地没有归档。
    ///
    /// # Errors
    ///
    /// 名字未知或被拒、摘要或字节数不符、读文件失败。
    pub fn verify_archive(&self, name: &str) -> Result<Option<String>, ModelError> {
        let entry = Registry::shipped()?.admit(name)?;
        let archive = self.archive_path(name);
        if !archive.is_file() {
            return Ok(None);
        }
        verify_archive_bytes(entry, &archive)?;
        Ok(Some(entry.sha256.clone()))
    }
}

/// [`remove_cached`] 删掉了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Removed {
    /// 解包目录被删了。
    pub dir: bool,
    /// 归档被删了。
    pub archive: bool,
}

impl Removed {
    /// 是否什么都没删。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        !self.dir && !self.archive
    }
}

impl ModelCache {
    /// 删掉一个模型的本地缓存。
    ///
    /// **不过许可门禁**，走 [`Registry::find`] 而不是 [`Registry::admit`]：删除是缩小
    /// 攻击面的动作，一个被拒的模型如果已经躺在缓存里，最该允许的操作就是把它删掉。
    ///
    /// # Errors
    ///
    /// 名字不在清单里，或删除失败。
    pub fn remove(&self, name: &str) -> Result<Removed, ModelError> {
        Registry::shipped()?.find(name)?;
        let mut removed = Removed::default();

        let dir = self.model_dir(name);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|error| ModelError::Io {
                path: dir.clone(),
                detail: error.to_string(),
            })?;
            removed.dir = true;
        }

        let archive = self.archive_path(name);
        if archive.is_file() {
            std::fs::remove_file(&archive).map_err(|error| ModelError::Io {
                path: archive.clone(),
                detail: error.to_string(),
            })?;
            removed.archive = true;
        }

        Ok(removed)
    }
}

/// 下载到同目录临时文件，核对摘要，然后原子改名。
///
/// 与 `yunjian_core::corpus` 的落地形态一致，这不是巧合而是同一条不变量：
/// **最终名字只会出现在已校验的字节上**。摘要不符时删掉临时文件，一个字节都不留。
fn download_verified(
    entry: &ModelEntry,
    archive: &Path,
    transport: &dyn Transport,
    progress: &mut dyn FnMut(FetchProgress),
) -> Result<(), ModelError> {
    let directory = archive.parent().ok_or_else(|| ModelError::Io {
        path: archive.to_path_buf(),
        detail: "归档路径没有父目录".to_owned(),
    })?;
    create_dir(directory)?;

    let temp = temp_path(archive);
    let outcome = stream_to_file(entry, &temp, transport, progress)
        .and_then(|()| {
            let bytes = file_len(&temp)?;
            progress(FetchProgress::Verifying { bytes });
            verify_archive_bytes(entry, &temp)
        })
        .and_then(|()| {
            progress(FetchProgress::Verified);
            rename(&temp, archive)
        });

    if let Err(error) = outcome {
        // 临时文件永远不叫最终名字，所以留下来也不会被误认；但失败重试不该在目录里堆
        // 垃圾，而「摘要不符时不留文件」这条验收判据要求的正是这一步。
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    fsync_dir(directory);
    Ok(())
}

fn stream_to_file(
    entry: &ModelEntry,
    temp: &Path,
    transport: &dyn Transport,
    progress: &mut dyn FnMut(FetchProgress),
) -> Result<(), ModelError> {
    let file = std::fs::File::create(temp).map_err(|error| ModelError::Io {
        path: temp.to_path_buf(),
        detail: error.to_string(),
    })?;
    let mut writer = std::io::BufWriter::with_capacity(IO_BUFFER_BYTES, file);

    let mut reported = 0_u64;
    let mut on_bytes = |done: u64, total: u64| {
        if done.saturating_sub(reported) >= REPORT_BYTES || done == total {
            reported = done;
            progress(FetchProgress::Downloading {
                bytes_done: done,
                bytes_total: total,
            });
        }
    };
    {
        let mut capped = CappedWriter {
            inner: &mut writer,
            limit: entry.size_bytes,
            written: 0,
        };
        transport
            .fetch(&entry.url, &mut capped, &mut on_bytes)
            .map_err(|detail| ModelError::Download {
                name: entry.name.clone(),
                url: entry.url.clone(),
                detail,
            })?;
    }

    // `sync_all` 而不是只 `flush`：flush 只把字节交给操作系统，掉电后仍可能丢。
    // 这是「改名之后一定是一份完整文件」这条保证的前半段。
    writer
        .into_inner()
        .map_err(|error| ModelError::Io {
            path: temp.to_path_buf(),
            detail: error.to_string(),
        })?
        .sync_all()
        .map_err(|error| ModelError::Io {
            path: temp.to_path_buf(),
            detail: error.to_string(),
        })
}

/// 先比字节数再比摘要。字节数不符几乎一定是下载被截断，那个诊断比「摘要不符」具体得多，
/// 而且省掉一次整文件哈希。
fn verify_archive_bytes(entry: &ModelEntry, path: &Path) -> Result<(), ModelError> {
    let actual_size = file_len(path)?;
    if actual_size != entry.size_bytes {
        return Err(ModelError::SizeMismatch {
            name: entry.name.clone(),
            expected: entry.size_bytes,
            actual: actual_size,
        });
    }
    let actual = sha256_of_file(path)?;
    if actual != entry.sha256 {
        return Err(ModelError::ChecksumMismatch {
            name: entry.name.clone(),
            expected: entry.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

/// 解到同目录临时目录，再整体改名。
///
/// 归档内含一个与包同名的顶层目录（`tar xf sherpa-onnx-whisper-tiny.tar.bz2` 得到
/// `sherpa-onnx-whisper-tiny/`），因此解完要往里下一层；万一某个包没有那层，就用临时
/// 目录本身。两种都处理是因为「上游打包形态一致」这个假设在这个项目里已经被推翻过。
fn unpack_atomically(
    name: &str,
    unpacker: &dyn Unpacker,
    archive: &Path,
    dir: &Path,
) -> Result<(), ModelError> {
    let parent = dir.parent().ok_or_else(|| ModelError::Io {
        path: dir.to_path_buf(),
        detail: "模型目录没有父目录".to_owned(),
    })?;
    create_dir(parent)?;

    let staging = temp_path(dir);
    let _ = std::fs::remove_dir_all(&staging);
    create_dir(&staging)?;

    let outcome = unpacker
        .unpack(archive, &staging)
        .map_err(|detail| ModelError::Unpack {
            name: name.to_owned(),
            detail,
        })
        .and_then(|()| {
            let nested = staging.join(name);
            let produced = if nested.is_dir() {
                nested
            } else {
                staging.clone()
            };
            if !is_populated(&produced) {
                return Err(ModelError::Unpack {
                    name: name.to_owned(),
                    detail: format!("归档解开后 {} 是空的", produced.display()),
                });
            }
            rename(&produced, dir)
        });

    let _ = std::fs::remove_dir_all(&staging);
    outcome?;
    fsync_dir(parent);
    Ok(())
}

/// 同目录、唯一命名的临时路径。
///
/// 同目录是硬要求（跨卷 rename 不原子）；pid 加计数器让同一进程的多个线程与同时运行的
/// 多个进程不会撞到同一个临时名。后缀接在文件名**后面**而不是替换扩展名——
/// `Path::with_extension` 会把 `x.tar.bz2` 变成 `x.tar.partial`，那和「x.tar.bz2 的
/// 旁文件」不是一回事。
fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}{TEMP_SUFFIX}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    target.with_file_name(name)
}

fn create_dir(path: &Path) -> Result<(), ModelError> {
    std::fs::create_dir_all(path).map_err(|error| ModelError::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}

fn file_len(path: &Path) -> Result<u64, ModelError> {
    std::fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| ModelError::Io {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })
}

fn rename(from: &Path, to: &Path) -> Result<(), ModelError> {
    std::fs::rename(from, to).map_err(|error| ModelError::Io {
        path: to.to_path_buf(),
        detail: format!("由 {} 改名失败：{error}", from.display()),
    })
}

/// 目录项本身也要落盘，否则掉电后可能「文件在、目录里看不见」。
/// 不是所有平台都允许把目录当文件打开，因此这一步尽力而为。
fn fsync_dir(directory: &Path) {
    if let Ok(handle) = std::fs::File::open(directory) {
        let _ = handle.sync_all();
    }
}

fn sha256_of_file(path: &Path) -> Result<String, ModelError> {
    let mut file = std::fs::File::open(path).map_err(|error| ModelError::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(|error| ModelError::Io {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(feature = "download")]
mod transport;

#[cfg(feature = "download")]
pub use transport::{Bz2TarUnpacker, HttpTransport};

#[cfg(feature = "download")]
fn default_transport() -> Option<Box<dyn Transport>> {
    Some(Box::new(HttpTransport::default()))
}

#[cfg(not(feature = "download"))]
fn default_transport() -> Option<Box<dyn Transport>> {
    None
}

#[cfg(feature = "download")]
fn default_unpacker() -> Option<Box<dyn Unpacker>> {
    Some(Box::new(Bz2TarUnpacker))
}

#[cfg(not(feature = "download"))]
fn default_unpacker() -> Option<Box<dyn Unpacker>> {
    None
}

#[cfg(test)]
mod tests;
