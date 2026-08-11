//! 语料库的解析、校验落地与只读打开。
//!
//! # 为什么必须「落地」而不是就地打开
//!
//! Android 的资产不是文件：「An asset is not a file. It is an entry in the ZIP archive
//! that makes up an APK」。SQLite 打不开它，实测的失败形态是
//! `unable to open database file`。所以随包语料**一定**要先复制出来，那么随包压缩就是
//! 纯赚的——既省安装包空间，也绕开平台对某些已知扩展名的自动变换。
//!
//! # 解析顺序
//!
//! 1. **显式配置路径**（`corpus.path`，可被 `YUNJIAN_CORPUS_PATH` 覆盖）。指到不存在的
//!    文件必须报错，不能静默降级——这与 `config` 模块对 `--config` / `APP_CONFIG` 的处理
//!    一致：显式意图不该被悄悄忽略。
//! 2. **应用数据目录里已落地的副本**（`corpus.data_dir/corpus.db`）。
//! 3. **随包或已下载的 `.db.gz`**，校验期望 SHA-256 后原子落地到第 2 级的位置。
//!
//! 全程不看可执行文件旁边的目录：安装目录通常不可写（Windows 的 `Program Files`、
//! macOS 的 `.app` 内部、Linux 的 `/usr/bin`），把用户数据写在那里在真机上就是失败。
//!
//! # 原子性与幂等
//!
//! 落地写**同目录**的临时文件 → `fsync` → `rename` → `fsync` 目录。同目录是硬要求：
//! 跨文件系统的 `rename` 不是原子操作，`/tmp` 与应用数据目录很可能不在一个卷上。
//!
//! 于是任何一次中断留下的都是 `corpus.db.<pid>.<序号>.tmp`，而下一次运行只认
//! `corpus.db` 这个名字，**不可能**把半成品当成完整语料。残留的临时文件在下一次解析时
//! 被清扫（[`CORPUS_FILE_NAME`] 前缀 + [`TEMP_SUFFIX`] 后缀），所以重跑既干净也不累积垃圾。
//!
//! 摘要在**写出任何字节之前**校验：先比字节数（不符就是下载被截断，这个诊断具体得多），
//! 再比 SHA-256。代价是多读一遍归档；换来的是「校验失败时目标目录里一个文件都没有」，
//! 而不是「留下一个已经生成、只是不该用的文件」。
//!
//! # 首启派生的完成标记
//!
//! 三张检索结构不随包（见 [`crate::derive`]），首启在本机构建，唐宋规模实测 571.8 s。
//! 「是否已建完」**不能**只看表在不在：`schema-derived.sql` 先建表和索引再灌数据，
//! 而 `poem_fts` 的 rebuild 被打断时表已存在、内容不全——[`crate::derived_indexes_present`]
//! 会把它判为已完成。所以另立一个旁文件 `corpus.db.deriving` 当权威标记：开工前创建、
//! 成功后删除。标记在，就一定重建。
//!
//! 派生失败**不是致命错误**：字典仍然完整可用，只是两字查询退化。
//! [`CorpusHandle::derived`] 把这件事变成调用方必须处理的类型，而不是一个静默的空结果。
//!
//! # 为什么 handle 里没有连接
//!
//! `rusqlite::Connection` 不是 `Sync`。把它放进 handle 会让 handle 也不是 `Sync`，
//! 那么「`Arc` 廉价克隆、跨线程与 FFI 友好」这个目标就落空了。所以 handle 只持有
//! 路径与元数据，连接由 [`CorpusHandle::connect`] 现开——**每个 worker 一个连接**，
//! 这既是 SQLite 的正确用法，也是这里唯一能同时满足只读与并发的形状。
//!
//! # 本模块永不感知 Tauri
//!
//! 「随包资产怎么到 `data_dir` 里」是外壳的事（Android 从 APK 里 copy、桌面端从安装
//! 目录 copy、CI 直接放）。本模块只认[`CORPUS_ARCHIVE_NAME`]这个约定名字与 `manifest.json`，
//! 因此同一份实现在桌面端、移动端与命令行下逐字相同。

use crate::config::CorpusConfig;
#[cfg(test)]
use crate::derive::DeriveStep;
use crate::derive::{
    DeriveProgress, DerivedBuildStats, build_derived_indexes_with_progress, derived_indexes_present,
};
use crate::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// schema v2 相对 v1 的变化：`defect` / `disposition` 移进审计库，`ngram` 改为首启
/// 派生，`corpus_meta` 增加 `derived_indexes` 与 `shipped_scope` 两列。
///
/// 之所以升版号而不是留在 1：`corpus_meta` 新增了 NOT NULL 列，拿 v1 的文件跑
/// v2 的查询会在 SQL 层报 `no such column`。升版号把它变成
/// [`OpenCorpusError::IncompatibleSchema`] 这条带下一步指引的类型化错误。
/// 尚无已发布工件，因此不需要迁移代码。
///
/// 这两个常量住在 `yunjian-core` 而不是 `yunjian-corpus`：构建期与运行期必须用**同一个**
/// 兼容范围，而依赖方向是 corpus -> core，只有放在 core 才可能是一份。
/// `yunjian_corpus::db` 原样重导出它们。
pub const SCHEMA_VERSION: u32 = 2;
pub const SUPPORTED_SCHEMA: RangeInclusive<u32> = 2..=2;

/// 落地后的语料库文件名。解析顺序第 2 级只认这一个名字。
pub const CORPUS_FILE_NAME: &str = "corpus.db";

/// 外壳把随包资产复制到 `corpus.data_dir` 时应当使用的文件名。
///
/// 刻意不带版本号：本模块无法枚举目录里可能存在的多个版本并挑一个「最新」——那需要
/// 解析文件名里的版本号，而文件名不是可信来源。版本从 `manifest.json` 与工件内的
/// `corpus_meta` 里读，文件名只是一个约定的入口。
pub const CORPUS_ARCHIVE_NAME: &str = "yunjian-corpus.db.gz";

/// `xtask corpus-package` 与归档一同产出的清单名。
pub const CORPUS_MANIFEST_NAME: &str = "manifest.json";

/// 落地中的临时文件后缀。
pub const TEMP_SUFFIX: &str = ".tmp";

/// 首启派生进行中的标记后缀，接在语料库文件名之后。
pub const DERIVING_MARKER_SUFFIX: &str = ".deriving";

/// 解压时每写出这么多字节汇报一次。
///
/// 16 MiB 让唐宋规模（解压后 633 MiB）产生约 40 次事件：够画出进度，又不至于让回调
/// 本身成为热点。
const DECOMPRESS_REPORT_BYTES: u64 = 16 * 1024 * 1024;

/// 读归档与写落地时的缓冲区大小。
const IO_BUFFER_BYTES: usize = 1 << 20;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 打开一个既有语料库文件时可能失败的方式。
///
/// [`OpenCorpusError::IncompatibleSchema`] 独立成变体而不是一句字符串，因为外壳要能
/// 区分「文件坏了」与「文件没坏、但需要换一份语料」——后者有明确的下一步动作。
#[derive(Debug, thiserror::Error)]
pub enum OpenCorpusError {
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("语料库元数据错误：{0}")]
    InvalidMetadata(String),
    #[error(
        "语料库 schema 版本 {corpus_schema_version} 与应用 {app_version} 不兼容；应用支持 {supported_min}..={supported_max}。请运行 `yunjian corpus fetch` 获取兼容语料库"
    )]
    IncompatibleSchema {
        corpus_schema_version: u32,
        app_version: &'static str,
        supported_min: u32,
        supported_max: u32,
    },
}

/// 以只读方式打开语料库，并在**任何领域查询之前**校验 schema 版本。
///
/// 两道只读保险都要：`SQLITE_OPEN_READ_ONLY` 让文件描述符本身不可写，
/// `PRAGMA query_only` 连同一进程内后续可能的 `ATTACH` 一起封住。
pub fn open_corpus(path: impl AsRef<Path>) -> std::result::Result<Connection, OpenCorpusError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (rows, schema_version): (i64, Option<u32>) = connection.query_row(
        "SELECT count(*), min(schema_version) FROM corpus_meta",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if rows != 1 {
        return Err(OpenCorpusError::InvalidMetadata(format!(
            "corpus_meta 必须恰有一行，实际为 {rows} 行"
        )));
    }
    let schema_version = schema_version.ok_or_else(|| {
        OpenCorpusError::InvalidMetadata("corpus_meta.schema_version 不能为空".to_owned())
    })?;
    if !SUPPORTED_SCHEMA.contains(&schema_version) {
        return Err(OpenCorpusError::IncompatibleSchema {
            corpus_schema_version: schema_version,
            app_version: app_version(),
            supported_min: *SUPPORTED_SCHEMA.start(),
            supported_max: *SUPPORTED_SCHEMA.end(),
        });
    }
    connection.pragma_update(None, "query_only", true)?;
    Ok(connection)
}

/// 语料库自述的身份与形态，来自 `corpus_meta` 唯一那一行。
///
/// `index_detail_mode` 必须在运行期从这里读、永不假设：查询路由（todo 24）按它决定
/// 长查询能不能用 `MATCH`，而它是构建期把实测裁决刻进工件的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusMeta {
    pub schema_version: u32,
    pub corpus_version: String,
    pub built_at: String,
    pub poem_count: i64,
    pub index_detail_mode: String,
    pub derived_indexes: String,
    pub shipped_scope: String,
}

impl CorpusMeta {
    fn read(connection: &Connection) -> Result<Self> {
        let (meta, integrity_check) = connection.query_row(
            "SELECT schema_version, corpus_version, built_at, poem_count, \
                    index_detail_mode, derived_indexes, shipped_scope, integrity_check \
             FROM corpus_meta WHERE singleton=1",
            [],
            |row| {
                Ok((
                    Self {
                        schema_version: row.get(0)?,
                        corpus_version: row.get(1)?,
                        built_at: row.get(2)?,
                        poem_count: row.get(3)?,
                        index_detail_mode: row.get(4)?,
                        derived_indexes: row.get(5)?,
                        shipped_scope: row.get(6)?,
                    },
                    row.get::<_, String>(7)?,
                ))
            },
        )?;
        if integrity_check != "ok" {
            return Err(corpus_error(format!(
                "语料库记录 integrity_check = `{integrity_check}` 而不是 `ok`；\
                 完整性未通过的语料库不得使用"
            )));
        }
        Ok(meta)
    }
}

/// 当前这份语料库是从哪一级解析出来的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusOrigin {
    /// 第 1 级：`corpus.path` 显式指定。
    Configured,
    /// 第 2 级：应用数据目录里已落地的副本。
    Materialized,
    /// 第 3 级：本次运行刚从归档校验并落地。
    JustMaterialized { archive: PathBuf, sha256: String },
}

/// 首启派生结构的状态。
///
/// `Unavailable` 带原因而不是一个 `bool`：调用方要能告诉用户「两字查询暂时慢」以及
/// 为什么，而「为什么」在 `false` 里表达不出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedState {
    /// 结构就绪。`stats` 仅在本次运行构建过时有值。
    Ready { stats: Option<DerivedBuildStats> },
    /// 结构不可用：两字查询没有候选表可走，调用方须降级并告知用户。
    Unavailable { reason: String },
}

impl DerivedState {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// 落地与首启派生的进度事件。
///
/// 借用而不是持有字符串：事件在唐宋规模上会发出上千次，而消费方只是格式化一下。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializationProgress<'a> {
    /// 已有可用语料库，本次不需要落地。
    AlreadyPresent { path: &'a Path },
    /// 开始校验归档；`bytes` 是归档的实际字节数。
    VerifyingArchive { archive: &'a Path, bytes: u64 },
    /// 归档摘要与期望一致。
    ArchiveVerified { sha256: &'a str },
    /// 正在解压。`bytes_total == 0` 表示清单没给出解压后大小。
    Decompressing { bytes_done: u64, bytes_total: u64 },
    /// 已原子落地。
    Materialized {
        path: &'a Path,
        corpus_version: &'a str,
    },
    /// 首启派生进度。唐宋规模实测总计 571.8 s。
    Deriving(DeriveProgress),
    /// 首启派生失败。字典仍可用，两字查询退化；下次运行会重来。
    DeriveFailed { reason: &'a str },
    /// 语料库已就绪，可以开始查询。
    Ready {
        path: &'a Path,
        corpus_version: &'a str,
        derived: bool,
    },
}

/// 一份已就绪的只读语料库。
///
/// 克隆是 `Arc` 级别的开销，因此可以随手发给每个 worker、每个 FFI 调用；跨线程共享的
/// 是路径与元数据，连接各自 [`CorpusHandle::connect`] 现开。
#[derive(Debug, Clone)]
pub struct CorpusHandle {
    inner: Arc<CorpusInner>,
}

#[derive(Debug)]
struct CorpusInner {
    path: PathBuf,
    meta: CorpusMeta,
    origin: CorpusOrigin,
    derived: DerivedState,
}

impl CorpusHandle {
    /// 按解析顺序找到语料库，必要时校验并落地，然后以只读方式打开。
    pub fn open(cfg: &CorpusConfig) -> Result<Self> {
        Self::open_with_progress(cfg, &mut |_| {})
    }

    /// 同 [`CorpusHandle::open`]，但逐步汇报进度。
    ///
    /// 首启路径上这个回调是唯一的反馈来源：校验 + 解压 + 派生在唐宋规模上合计约十分钟，
    /// 不汇报就与卡死无法区分。
    pub fn open_with_progress(
        cfg: &CorpusConfig,
        progress: &mut dyn FnMut(MaterializationProgress<'_>),
    ) -> Result<Self> {
        let (path, origin) = resolve(cfg, progress)?;
        let meta = {
            let connection = connect_read_only(&path)?;
            CorpusMeta::read(&connection)?
        };
        let derived = ensure_derived(&path, progress);
        if let DerivedState::Unavailable { reason } = &derived {
            tracing::warn!(
                corpus = %path.display(),
                reason = %reason,
                "首启派生结构不可用：两字查询将退化，下次启动会重试"
            );
            progress(MaterializationProgress::DeriveFailed { reason });
        }
        progress(MaterializationProgress::Ready {
            path: &path,
            corpus_version: &meta.corpus_version,
            derived: derived.is_ready(),
        });
        tracing::info!(
            corpus = %path.display(),
            corpus_version = %meta.corpus_version,
            schema_version = meta.schema_version,
            poem_count = meta.poem_count,
            index_detail_mode = %meta.index_detail_mode,
            derived = derived.is_ready(),
            "语料库已就绪"
        );
        Ok(Self {
            inner: Arc::new(CorpusInner {
                path,
                meta,
                origin,
                derived,
            }),
        })
    }

    /// 新开一个只读连接。**每个 worker 一个**，不要跨线程共用。
    pub fn connect(&self) -> Result<Connection> {
        connect_read_only(&self.inner.path)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    #[must_use]
    pub fn meta(&self) -> &CorpusMeta {
        &self.inner.meta
    }

    #[must_use]
    pub fn origin(&self) -> &CorpusOrigin {
        &self.inner.origin
    }

    /// 首启派生结构的状态。两字查询路径必须先看这里。
    #[must_use]
    pub fn derived(&self) -> &DerivedState {
        &self.inner.derived
    }

    /// 运行期索引形态，来自 `corpus_meta.index_detail_mode`。
    #[must_use]
    pub fn index_detail_mode(&self) -> &str {
        &self.inner.meta.index_detail_mode
    }
}

fn connect_read_only(path: &Path) -> Result<Connection> {
    open_corpus(path).map_err(|error| match error {
        OpenCorpusError::Database(inner) => Error::Db(inner),
        other => corpus_error(format!("打开 {} 失败：{other}", path.display())),
    })
}

/// 按三级顺序定位语料库；第 3 级会真的落地。
fn resolve(
    cfg: &CorpusConfig,
    progress: &mut dyn FnMut(MaterializationProgress<'_>),
) -> Result<(PathBuf, CorpusOrigin)> {
    if let Some(path) = &cfg.path {
        if !path.is_file() {
            return Err(corpus_error(format!(
                "配置指定的语料库 {} 不存在；显式指定的路径不会被静默忽略，\
                 要走随包语料请清空 `corpus.path` 与 `YUNJIAN_CORPUS_PATH`",
                path.display()
            )));
        }
        progress(MaterializationProgress::AlreadyPresent { path });
        return Ok((path.clone(), CorpusOrigin::Configured));
    }

    let target = cfg.data_dir.join(CORPUS_FILE_NAME);
    let marker = deriving_marker(&target);
    if target.is_file() {
        progress(MaterializationProgress::AlreadyPresent { path: &target });
        return Ok((target, CorpusOrigin::Materialized));
    }
    // 目标不在但标记在：上一次是在落地成功、派生途中被打断，而落地的产物又被外部删了。
    // 标记留着会让下一次「已落地」判断说谎，所以在这里一并清掉。
    let _ = std::fs::remove_file(&marker);

    let archive = archive_candidate(cfg)?;
    let expectation = ArchiveExpectation::load(&archive)?;
    sweep_stale_temps(&cfg.data_dir)?;
    let sha256 = materialize(&archive, &expectation, &target, progress)?;
    Ok((target, CorpusOrigin::JustMaterialized { archive, sha256 }))
}

/// 找出该用哪个 `.db.gz`。
fn archive_candidate(cfg: &CorpusConfig) -> Result<PathBuf> {
    if let Some(archive) = &cfg.archive {
        if !archive.is_file() {
            return Err(corpus_error(format!(
                "配置指定的语料归档 {} 不存在",
                archive.display()
            )));
        }
        return Ok(archive.clone());
    }

    // 清单里的 `artifact_name` 是唯一不需要猜的来源：打包时它与归档在同一次运行里写出。
    let manifest_path = cfg.data_dir.join(CORPUS_MANIFEST_NAME);
    if let Some(manifest) = ArtifactManifest::read(&manifest_path)? {
        let named = cfg.data_dir.join(&manifest.artifact_name);
        if named.is_file() {
            return Ok(named);
        }
        return Err(corpus_error(format!(
            "{} 声明工件 `{}`，但 {} 不存在；清单与归档必须成对存在",
            manifest_path.display(),
            manifest.artifact_name,
            named.display()
        )));
    }

    let conventional = cfg.data_dir.join(CORPUS_ARCHIVE_NAME);
    if conventional.is_file() {
        return Ok(conventional);
    }
    Err(corpus_error(format!(
        "找不到语料库：{} 里既没有已落地的 {CORPUS_FILE_NAME}，也没有 {CORPUS_MANIFEST_NAME} \
         或 {CORPUS_ARCHIVE_NAME}。请让外壳把随包语料放到该目录，\
         或用 `corpus.path` / `corpus.archive` 显式指定",
        cfg.data_dir.display()
    )))
}

/// 运行期消费的清单字段。
///
/// **刻意不开 `deny_unknown_fields`**：将来给清单加字段不该让旧应用连解析都过不去。
/// 兼容性由 `schema_version` 与 `min_app_version` 表达，那是两个可以给出确切下一步的
/// 判据；而「多了个没见过的键」给不出任何有用指引。
#[derive(Debug, Clone, Deserialize)]
struct ArtifactManifest {
    schema_version: u32,
    corpus_version: String,
    min_app_version: String,
    artifact_name: String,
    size_bytes: u64,
    sha256: String,
    uncompressed_bytes: u64,
}

impl ArtifactManifest {
    fn read(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(path)?;
        let manifest: Self = serde_json::from_slice(&bytes)
            .map_err(|error| corpus_error(format!("解析 {} 失败：{error}", path.display())))?;
        Ok(Some(manifest))
    }
}

/// 落地之前必须成立的期望。
#[derive(Debug, Clone)]
struct ArchiveExpectation {
    sha256: String,
    size_bytes: Option<u64>,
    uncompressed_bytes: Option<u64>,
    source: PathBuf,
}

impl ArchiveExpectation {
    /// 期望摘要的来源：同目录的 `manifest.json`（更全）优先，其次 `<归档>.sha256` 旁文件。
    ///
    /// 两者都没有就**报错而不是放行**。一个无从校验的归档与一个校验失败的归档，
    /// 对「这份语料是不是我们发出去的那份」这个问题给出的答案完全相同。
    fn load(archive: &Path) -> Result<Self> {
        let directory = archive.parent().unwrap_or_else(|| Path::new("."));
        let file_name = archive
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| corpus_error(format!("归档路径没有文件名：{}", archive.display())))?;

        let manifest_path = directory.join(CORPUS_MANIFEST_NAME);
        if let Some(manifest) = ArtifactManifest::read(&manifest_path)?
            && manifest.artifact_name == file_name
        {
            check_compatibility(&manifest, &manifest_path)?;
            return Ok(Self {
                sha256: manifest.sha256.to_ascii_lowercase(),
                size_bytes: Some(manifest.size_bytes),
                uncompressed_bytes: Some(manifest.uncompressed_bytes),
                source: manifest_path,
            });
        }

        let sidecar = sidecar_path(archive);
        if sidecar.is_file() {
            let text = std::fs::read_to_string(&sidecar)?;
            let digest = parse_sha256_sidecar(&text, file_name, &sidecar)?;
            return Ok(Self {
                sha256: digest,
                size_bytes: None,
                uncompressed_bytes: None,
                source: sidecar,
            });
        }

        Err(corpus_error(format!(
            "{} 没有可用的期望摘要：同目录既没有描述它的 {CORPUS_MANIFEST_NAME}，\
             也没有 {}。未经校验的语料库不会被落地",
            archive.display(),
            sidecar.display()
        )))
    }
}

/// 校验清单声明的兼容范围。
///
/// 两条都要：`schema_version` 说「这个文件的表长什么样」，`min_app_version` 说
/// 「读它需要多新的应用」。两者可以独立变化——同一个 schema 下新增一列可选语义时，
/// 老应用读得出表但读不懂内容。
fn check_compatibility(manifest: &ArtifactManifest, manifest_path: &Path) -> Result<()> {
    if !SUPPORTED_SCHEMA.contains(&manifest.schema_version) {
        return Err(corpus_error(format!(
            "{} 声明 schema 版本 {}，应用 {} 支持 {}..={}；\
             请运行 `yunjian corpus fetch` 获取兼容语料库",
            manifest_path.display(),
            manifest.schema_version,
            app_version(),
            SUPPORTED_SCHEMA.start(),
            SUPPORTED_SCHEMA.end()
        )));
    }
    let required = parse_version(&manifest.min_app_version).ok_or_else(|| {
        corpus_error(format!(
            "{} 的 min_app_version `{}` 不是 semver",
            manifest_path.display(),
            manifest.min_app_version
        ))
    })?;
    let current = parse_version(app_version())
        .ok_or_else(|| corpus_error(format!("应用版本 `{}` 不是 semver", app_version())))?;
    if current < required {
        return Err(corpus_error(format!(
            "语料 {} 要求应用至少 {}，当前 {}；请先升级应用",
            manifest.corpus_version,
            manifest.min_app_version,
            app_version()
        )));
    }
    Ok(())
}

/// 校验归档并原子落地，回传实测摘要。
fn materialize(
    archive: &Path,
    expectation: &ArchiveExpectation,
    target: &Path,
    progress: &mut dyn FnMut(MaterializationProgress<'_>),
) -> Result<String> {
    let directory = target
        .parent()
        .ok_or_else(|| corpus_error(format!("落地目标没有父目录：{}", target.display())))?;
    std::fs::create_dir_all(directory)?;

    let actual_size = std::fs::metadata(archive)?.len();
    progress(MaterializationProgress::VerifyingArchive {
        archive,
        bytes: actual_size,
    });
    // 先比字节数：不符几乎一定是下载被截断或写盘被打断，这个诊断比「摘要不符」具体得多，
    // 而且省掉一次整文件哈希。
    if let Some(expected) = expectation.size_bytes
        && expected != actual_size
    {
        return Err(corpus_error(format!(
            "语料归档 {} 有 {actual_size} 字节，{} 记录 {expected} 字节；\
             下载可能被截断，请重新获取",
            archive.display(),
            expectation.source.display()
        )));
    }

    let actual_digest = sha256_of_file(archive)?;
    if actual_digest != expectation.sha256 {
        return Err(corpus_error(format!(
            "语料归档 {} 摘要 {actual_digest} 与 {} 记录的 {} 不符；\
             未写出任何文件",
            archive.display(),
            expectation.source.display(),
            expectation.sha256
        )));
    }
    progress(MaterializationProgress::ArchiveVerified {
        sha256: &actual_digest,
    });

    let temp = temp_path(target);
    let outcome = decompress_into(archive, &temp, expectation.uncompressed_bytes, progress)
        .and_then(|()| validate_materialized(&temp))
        .and_then(|version| publish(&temp, target, directory).map(|()| version));
    match outcome {
        Ok(corpus_version) => {
            progress(MaterializationProgress::Materialized {
                path: target,
                corpus_version: &corpus_version,
            });
            tracing::info!(
                archive = %archive.display(),
                corpus = %target.display(),
                corpus_version = %corpus_version,
                sha256 = %actual_digest,
                "语料库已校验并原子落地"
            );
            Ok(actual_digest)
        }
        Err(error) => {
            // 临时文件永远不叫最终名字，所以删不掉也不会被后续运行误认；但正常路径上
            // 还是要删干净，否则失败重试会在目录里堆垃圾。
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn decompress_into(
    archive: &Path,
    temp: &Path,
    bytes_total: Option<u64>,
    progress: &mut dyn FnMut(MaterializationProgress<'_>),
) -> Result<()> {
    let source = std::fs::File::open(archive)?;
    let mut decoder =
        flate2::read::GzDecoder::new(std::io::BufReader::with_capacity(IO_BUFFER_BYTES, source));
    let file = std::fs::File::create(temp)?;
    let mut writer = std::io::BufWriter::with_capacity(IO_BUFFER_BYTES, file);

    let total = bytes_total.unwrap_or(0);
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];
    let mut written: u64 = 0;
    let mut reported: u64 = 0;
    loop {
        let read = decoder.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        written += read as u64;
        if written - reported >= DECOMPRESS_REPORT_BYTES {
            reported = written;
            progress(MaterializationProgress::Decompressing {
                bytes_done: written,
                bytes_total: total,
            });
        }
    }
    progress(MaterializationProgress::Decompressing {
        bytes_done: written,
        bytes_total: total,
    });

    if let Some(expected) = bytes_total
        && expected != written
    {
        return Err(corpus_error(format!(
            "解压得到 {written} 字节，清单记录 {expected} 字节；归档内容与清单不符"
        )));
    }

    writer.flush()?;
    // `sync_all` 而不是只 `flush`：flush 只把字节交给操作系统，掉电后仍可能丢。
    // 这是「rename 之后一定是一份完整文件」这条保证的前半段。
    writer
        .into_inner()
        .map_err(|error| Error::Io(error.into_error()))?
        .sync_all()?;
    Ok(())
}

/// 改名之前先证明这份文件真的能当语料库用，回传它的 `corpus_version`。
///
/// 摘要相符只说明字节与发布者一致；它不能说明发布者给的是一份可用的语料库。
/// 把校验放在 rename 之前，`corpus.db` 这个名字就只会出现在「已验证可用」的文件上。
fn validate_materialized(temp: &Path) -> Result<String> {
    let connection = connect_read_only(temp)?;
    let meta = CorpusMeta::read(&connection)?;
    Ok(meta.corpus_version)
}

/// 原子发布：`rename` + 父目录 `fsync`。
fn publish(temp: &Path, target: &Path, directory: &Path) -> Result<()> {
    match std::fs::rename(temp, target) {
        Ok(()) => {}
        Err(error) => {
            // Windows 的 rename 在目标已存在时失败。走到这里说明另一个进程刚刚落地成功，
            // 那份与我们的字节相同（同一个摘要），保留它、丢掉我们的临时文件即可。
            if target.is_file() {
                let _ = std::fs::remove_file(temp);
                return Ok(());
            }
            return Err(Error::Io(error));
        }
    }
    // 目录项本身也要落盘，否则掉电后可能「文件在、目录里看不见」。
    // 不是所有平台都允许把目录当文件打开，因此这一步尽力而为。
    if let Ok(handle) = std::fs::File::open(directory) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// 清掉上一次被打断的落地留下的临时文件。
///
/// 它们不可能被误认为完整语料（名字不同），但会占掉与语料库同量级的空间，
/// 所以每次要落地之前扫一遍。
fn sweep_stale_temps(directory: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::Io(error)),
    };
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(CORPUS_FILE_NAME) && name.ends_with(TEMP_SUFFIX) {
            let path = entry.path();
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!(
                    stale = %path.display(),
                    "清掉上一次被打断的落地留下的临时文件"
                ),
                Err(error) => tracing::warn!(
                    stale = %path.display(),
                    error = %error,
                    "清理临时文件失败，继续落地"
                ),
            }
        }
    }
    Ok(())
}

/// 确保首启派生结构就绪。失败不致命，回传原因即可。
///
/// 派生写的是**当前解析出来的那个文件**，包括 `corpus.path` 显式指定的那一份。
/// 换成「显式路径一律不派生」的话，显式路径就永远得不到可用的两字查询——而那是最常见的
/// 查询形状。文件不可写（只读挂载、系统级安装目录）时这里失败并降级，不会中断打开。
///
/// 两个进程同时首启时，SQLite 的写锁会让其中一个拿到 `SQLITE_BUSY` 并降级为
/// `Unavailable`，下次启动重来；标记文件让这种竞争不会留下被误认为完整的索引。
fn ensure_derived(
    path: &Path,
    progress: &mut dyn FnMut(MaterializationProgress<'_>),
) -> DerivedState {
    match derived_status(path) {
        Ok(true) => return DerivedState::Ready { stats: None },
        Ok(false) => {}
        Err(error) => {
            return DerivedState::Unavailable {
                reason: error.to_string(),
            };
        }
    }
    match derive_now(path, progress) {
        Ok(stats) => {
            tracing::info!(
                corpus = %path.display(),
                poems = stats.poems,
                grams = stats.grams,
                elapsed_secs = stats.elapsed.as_secs_f64(),
                "首启派生完成"
            );
            DerivedState::Ready { stats: Some(stats) }
        }
        Err(error) => DerivedState::Unavailable {
            reason: error.to_string(),
        },
    }
}

/// 便宜的就绪判断：标记文件 + 表与索引是否都在。
///
/// **刻意不在这里跑 `verify_derived_indexes`**：那个函数要读完每一首诗的正文（唐宋规模
/// 519 MiB）才能算出覆盖数，放在每次启动的路径上等于给每次启动加几秒 I/O。
/// 它的位置在派生结束时（`build_derived_indexes` 内部已经调用），那里才值得付这个代价。
fn derived_status(path: &Path) -> Result<bool> {
    if deriving_marker(path).exists() {
        return Ok(false);
    }
    let connection = connect_read_only(path)?;
    derived_indexes_present(&connection)
}

fn derive_now(
    path: &Path,
    progress: &mut dyn FnMut(MaterializationProgress<'_>),
) -> Result<DerivedBuildStats> {
    let marker = deriving_marker(path);
    std::fs::write(
        &marker,
        b"yunjian: derived indexes are being rebuilt; delete nothing by hand\n",
    )?;
    let mut connection = Connection::open(path)?;
    let stats = build_derived_indexes_with_progress(&mut connection, &mut |event| {
        progress(MaterializationProgress::Deriving(event));
    })?;
    // 标记只在派生**成功**后才删。删失败要当成失败：留着标记只是多跑一次派生，
    // 而当成成功会让一份不完整的候选表被当作可信索引使用，那是静默的错误结果。
    std::fs::remove_file(&marker)?;
    Ok(stats)
}

fn deriving_marker(corpus: &Path) -> PathBuf {
    sibling_with_suffix(corpus, DERIVING_MARKER_SUFFIX)
}

fn sidecar_path(archive: &Path) -> PathBuf {
    sibling_with_suffix(archive, ".sha256")
}

/// 在文件名**后面**接后缀，而不是替换扩展名。
///
/// `Path::with_extension` 会把 `corpus.db` 变成 `corpus.deriving`，那与
/// 「`corpus.db` 的旁文件」不是一回事，也会和别的 `corpus.*` 撞名。
fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// 同目录、唯一命名的临时路径。
///
/// 同目录是硬要求（跨卷 rename 不原子）；pid + 计数器让同一进程的多个线程与
/// 同时运行的多个进程不会撞到同一个临时文件。
fn temp_path(target: &Path) -> PathBuf {
    let suffix = format!(
        ".{}.{}{TEMP_SUFFIX}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    );
    sibling_with_suffix(target, &suffix)
}

/// `sha256sum` 旁文件的格式是 `<64 位十六进制>  <文件名>`（两个空格）。
///
/// 文件名要核对：一个描述别的工件的旁文件与没有旁文件同样无法证明这个归档。
fn parse_sha256_sidecar(text: &str, expected_name: &str, sidecar: &Path) -> Result<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (digest, name) = line.split_once("  ").unwrap_or((line, expected_name));
        let digest = digest.trim().to_ascii_lowercase();
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(corpus_error(format!(
                "{} 里的 `{digest}` 不是 64 位十六进制 SHA-256",
                sidecar.display()
            )));
        }
        if name.trim().trim_start_matches('*') == expected_name {
            return Ok(digest);
        }
    }
    Err(corpus_error(format!(
        "{} 没有描述 `{expected_name}` 的摘要行",
        sidecar.display()
    )))
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
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

/// 只取 semver 的三段主版本用于比较；预发布与构建元数据忽略。
///
/// 忽略预发布段是刻意的：`0.2.0-rc.1` 与 `0.2.0` 在「能不能读这份语料」上没有区别，
/// 而按 semver 完整规则排序会让 rc 版本被判为不满足 `min_app_version = 0.2.0`。
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

#[cfg(test)]
mod tests;
