//! 语料与随包赏析种子的统一获取生命周期。

use crate::{CORPUS_ARCHIVE_NAME, CorpusConfig, CorpusHandle, Error, Result, SUPPORTED_SCHEMA};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 用户数据目录中保留的统一资产清单文件名。
pub const ASSETS_MANIFEST_FILE_NAME: &str = "assets_manifest.json";

/// 已校验并成功导入的赏析种子文件名。
pub const APPRECIATION_SEED_FILE_NAME: &str = "appreciations.json";

/// 覆盖统一资产清单来源的环境变量。
pub const ENV_ASSETS_MANIFEST: &str = "YUNJIAN_ASSETS_MANIFEST";

/// 尚未显式配置清单时使用的发布地址。
pub const DEFAULT_ASSETS_MANIFEST_URL: &str =
    "https://github.com/sunerpy/yunjian/releases/latest/download/assets_manifest.json";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const BUFFER_BYTES: usize = 1 << 16;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// 统一清单中描述的语料工件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusAssetManifest {
    /// 下载地址，支持 HTTPS、`file://` 与本地路径。
    pub url: String,
    /// 工件的 SHA-256。
    pub sha256: String,
    /// 工件内应声明的语料版本。
    pub corpus_version: String,
    /// 工件内应声明的 schema 版本。
    pub schema_version: u32,
}

/// 统一清单中描述的随包赏析种子。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppreciationSeedManifest {
    /// 下载地址，支持 HTTPS、`file://` 与本地路径。
    pub url: String,
    /// 种子 JSON 的 SHA-256。
    pub sha256: String,
    /// 生成种子时使用的提示词模板版本。
    pub template_version: String,
    /// 生成种子时使用的语料版本。
    pub corpus_version: String,
    /// 种子应包含的记录数。
    pub record_count: usize,
}

/// 同一次数据发布中两件独立工件的声明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetsManifest {
    /// 语料归档。
    pub corpus: CorpusAssetManifest,
    /// 随包赏析种子。
    pub appreciation_seed: AppreciationSeedManifest,
}

impl AssetsManifest {
    /// 从 JSON 字节解析并执行不依赖下载内容的基础校验。
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| asset_error(format!("解析统一资产清单失败：{error}")))?;
        validate_manifest(&manifest)?;
        Ok(manifest)
    }
}

/// 清单或工件的本地/远程来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetLocation(String);

impl AssetLocation {
    /// 使用原始 URL 或路径构造来源。
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for AssetLocation {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AssetLocation {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<PathBuf> for AssetLocation {
    fn from(value: PathBuf) -> Self {
        Self(value.to_string_lossy().into_owned())
    }
}

impl From<&Path> for AssetLocation {
    fn from(value: &Path) -> Self {
        Self(value.to_string_lossy().into_owned())
    }
}

impl From<&PathBuf> for AssetLocation {
    fn from(value: &PathBuf) -> Self {
        Self::from(value.as_path())
    }
}

/// 一份已校验、尚未发布的下载工件。
#[derive(Debug)]
pub struct VerifiedAsset {
    path: PathBuf,
    sha256: String,
}

impl VerifiedAsset {
    /// 临时文件路径；仅在同步回调期间有效。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 实际校验得到的 SHA-256。
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// 一次统一资产同步的结果。
#[derive(Debug)]
pub struct AssetSync {
    /// 已就绪的语料库。
    pub corpus: CorpusHandle,
    /// 本次使用的清单。
    pub manifest: AssetsManifest,
    /// 已发布的种子文件。
    pub seed_path: PathBuf,
}

/// 统一下载、校验、语料落地与种子交付的解析器。
#[derive(Debug, Clone)]
pub struct AssetResolver {
    manifest: AssetLocation,
    corpus: CorpusConfig,
    app_data_dir: PathBuf,
}

impl AssetResolver {
    /// 绑定清单来源、语料配置与应用数据目录。
    #[must_use]
    pub fn new(
        manifest: impl Into<AssetLocation>,
        corpus: CorpusConfig,
        app_data_dir: impl AsRef<Path>,
    ) -> Self {
        Self {
            manifest: manifest.into(),
            corpus,
            app_data_dir: app_data_dir.as_ref().to_path_buf(),
        }
    }

    /// 从环境覆盖或默认发布地址构造解析器。
    #[must_use]
    pub fn discover(corpus: CorpusConfig, app_data_dir: impl AsRef<Path>) -> Self {
        let source = std::env::var(ENV_ASSETS_MANIFEST)
            .unwrap_or_else(|_| DEFAULT_ASSETS_MANIFEST_URL.to_owned());
        Self::new(source, corpus, app_data_dir)
    }

    /// 同步两件工件；导入器成功后才发布新种子文件。
    ///
    /// 不报进度。要在界面上显示物化进度请用 [`Self::sync_with_progress`]——本方法只是
    /// 给它传一个空回调，两者走的是**同一条**代码路径，不存在「带进度的那条另有实现」。
    pub fn sync<F>(&self, import: F) -> Result<AssetSync>
    where
        F: FnOnce(&VerifiedAsset, &AppreciationSeedManifest, &CorpusHandle) -> Result<()>,
    {
        self.sync_with_progress(import, &mut |_| {})
    }

    /// 同步两件工件，并把语料校验、解压与首启派生的进度逐条报出来。
    ///
    /// # 为什么要有这个重载
    ///
    /// 移动端首启要下载 212 MiB 归档并解压出数 GiB 语料，中间还有首启派生。桌面走
    /// `fetch_corpus` -> [`CorpusHandle::open_with_progress`] 已经能报这些阶段，而
    /// [`Self::sync`] 内部调的是无回调的 [`CorpusHandle::open`]，于是移动端只能盯着
    /// 一个不动的转圈。给它加一个可选回调比让调用方旁观文件系统里程碑更诚实：旁观拿到的
    /// 是「文件出现了」，报不出「正在核对归档摘要 / 已解压 40%」。
    ///
    /// 回调在**调用线程**上同步执行，不得在其中阻塞或再次进入本解析器。
    pub fn sync_with_progress<F>(
        &self,
        import: F,
        progress: &mut dyn FnMut(crate::MaterializationProgress<'_>),
    ) -> Result<AssetSync>
    where
        F: FnOnce(&VerifiedAsset, &AppreciationSeedManifest, &CorpusHandle) -> Result<()>,
    {
        std::fs::create_dir_all(&self.app_data_dir)?;
        std::fs::create_dir_all(&self.corpus.data_dir)?;

        let manifest_bytes = read_location(&self.manifest)?;
        let manifest = AssetsManifest::parse(&manifest_bytes)?;
        let manifest_path = self.app_data_dir.join(ASSETS_MANIFEST_FILE_NAME);
        write_atomic(&manifest_path, &manifest_bytes)?;

        let mut corpus_config = self.corpus.clone();
        if corpus_config.path.is_none()
            && !corpus_config
                .data_dir
                .join(crate::CORPUS_FILE_NAME)
                .is_file()
        {
            let archive = corpus_config.data_dir.join(CORPUS_ARCHIVE_NAME);
            ensure_verified_download(
                &AssetLocation::new(manifest.corpus.url.clone()),
                &manifest.corpus.sha256,
                &archive,
                "语料归档",
            )?;
            write_atomic(
                &suffix_path(&archive, ".sha256"),
                format!("{}  {CORPUS_ARCHIVE_NAME}\n", manifest.corpus.sha256).as_bytes(),
            )?;
            corpus_config.archive = Some(archive);
        }

        let corpus = CorpusHandle::open_with_progress(&corpus_config, progress)?;
        if corpus.meta().corpus_version != manifest.corpus.corpus_version
            || corpus.meta().schema_version != manifest.corpus.schema_version
        {
            return Err(asset_error(format!(
                "统一清单声明语料版本 {} / schema {}，实际为 {} / schema {}",
                manifest.corpus.corpus_version,
                manifest.corpus.schema_version,
                corpus.meta().corpus_version,
                corpus.meta().schema_version
            )));
        }

        let published_seed = self.app_data_dir.join(APPRECIATION_SEED_FILE_NAME);
        let seed_temp = temp_path(&published_seed);
        let downloaded = download_to_temp(
            &AssetLocation::new(manifest.appreciation_seed.url.clone()),
            &manifest.appreciation_seed.sha256,
            &seed_temp,
            "赏析种子",
        )?;
        let outcome = import(&downloaded, &manifest.appreciation_seed, &corpus)
            .and_then(|()| publish_replacing(&seed_temp, &published_seed));
        if let Err(error) = outcome {
            let _ = std::fs::remove_file(&seed_temp);
            return Err(error);
        }

        Ok(AssetSync {
            corpus,
            manifest,
            seed_path: published_seed,
        })
    }
}

fn validate_manifest(manifest: &AssetsManifest) -> Result<()> {
    validate_digest(&manifest.corpus.sha256, "语料归档")?;
    validate_digest(&manifest.appreciation_seed.sha256, "赏析种子")?;
    if manifest.corpus.url.trim().is_empty() || manifest.appreciation_seed.url.trim().is_empty() {
        return Err(asset_error("统一资产清单里的下载地址不能为空"));
    }
    if manifest.corpus.corpus_version.trim().is_empty()
        || manifest.appreciation_seed.corpus_version.trim().is_empty()
        || manifest
            .appreciation_seed
            .template_version
            .trim()
            .is_empty()
    {
        return Err(asset_error("统一资产清单里的版本字段不能为空"));
    }
    if !SUPPORTED_SCHEMA.contains(&manifest.corpus.schema_version) {
        return Err(asset_error(format!(
            "统一资产清单声明语料 schema {}，应用仅支持 {}..={}",
            manifest.corpus.schema_version,
            SUPPORTED_SCHEMA.start(),
            SUPPORTED_SCHEMA.end()
        )));
    }
    if manifest.appreciation_seed.record_count == 0 {
        return Err(asset_error("赏析种子 record_count 必须大于 0"));
    }
    Ok(())
}

fn validate_digest(digest: &str, label: &str) -> Result<()> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(asset_error(format!(
            "{label} 摘要必须是 64 位十六进制 SHA-256"
        )));
    }
    Ok(())
}

fn read_location(location: &AssetLocation) -> Result<Vec<u8>> {
    if is_http(location.as_str()) {
        let agent = http_agent();
        let mut response = agent
            .get(location.as_str())
            .call()
            .map_err(|error| asset_error(format!("下载 {} 失败：{error}", location.as_str())))?;
        let mut bytes = Vec::new();
        response.body_mut().as_reader().read_to_end(&mut bytes)?;
        return Ok(bytes);
    }
    std::fs::read(local_path(location.as_str()))
        .map_err(|error| asset_error(format!("读取 {} 失败：{error}", location.as_str())))
}

fn ensure_verified_download(
    source: &AssetLocation,
    expected: &str,
    target: &Path,
    label: &str,
) -> Result<()> {
    if target.is_file() && sha256_file(target)? == expected.to_ascii_lowercase() {
        return Ok(());
    }
    let temp = temp_path(target);
    let downloaded = download_to_temp(source, expected, &temp, label)?;
    let outcome = publish_replacing(downloaded.path(), target);
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    outcome
}

fn download_to_temp(
    source: &AssetLocation,
    expected: &str,
    temp: &Path,
    label: &str,
) -> Result<VerifiedAsset> {
    if let Some(parent) = temp.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let result = stream_location(source, temp).and_then(|()| {
        let actual = sha256_file(temp)?;
        if actual != expected.to_ascii_lowercase() {
            return Err(asset_error(format!(
                "{label} {} 摘要 {actual} 与清单记录的 {expected} 不符；未执行任何导入",
                source.as_str()
            )));
        }
        Ok(VerifiedAsset {
            path: temp.to_path_buf(),
            sha256: actual,
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

fn stream_location(source: &AssetLocation, target: &Path) -> Result<()> {
    let file = std::fs::File::create(target)?;
    let mut writer = std::io::BufWriter::with_capacity(BUFFER_BYTES, file);
    if is_http(source.as_str()) {
        let agent = http_agent();
        let mut response = agent
            .get(source.as_str())
            .call()
            .map_err(|error| asset_error(format!("下载 {} 失败：{error}", source.as_str())))?;
        std::io::copy(&mut response.body_mut().as_reader(), &mut writer)?;
    } else {
        let mut reader = std::io::BufReader::with_capacity(
            BUFFER_BYTES,
            std::fs::File::open(local_path(source.as_str()))?,
        );
        std::io::copy(&mut reader, &mut writer)?;
    }
    writer.flush()?;
    writer
        .into_inner()
        .map_err(|error| Error::Io(error.into_error()))?
        .sync_all()?;
    Ok(())
}

fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .user_agent(concat!("yunjian/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    let temp = temp_path(target);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        publish_replacing(&temp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn publish_replacing(temp: &Path, target: &Path) -> Result<()> {
    if std::fs::rename(temp, target).is_ok() {
        sync_parent(target);
        return Ok(());
    }
    let backup = suffix_path(target, ".previous");
    let had_previous = target.is_file();
    if had_previous {
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup)?;
    }
    match std::fs::rename(temp, target) {
        Ok(()) => {
            let _ = std::fs::remove_file(backup);
            sync_parent(target);
            Ok(())
        }
        Err(error) => {
            if had_previous {
                let _ = std::fs::rename(&backup, target);
            }
            Err(Error::Io(error))
        }
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut reader = std::io::BufReader::with_capacity(BUFFER_BYTES, std::fs::File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
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

fn local_path(source: &str) -> &Path {
    Path::new(source.strip_prefix("file://").unwrap_or(source))
}

fn is_http(source: &str) -> bool {
    source.starts_with("https://") || source.starts_with("http://")
}

fn temp_path(target: &Path) -> PathBuf {
    suffix_path(
        target,
        &format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ),
    )
}

fn suffix_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(directory) = std::fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
}

fn asset_error(message: impl Into<String>) -> Error {
    Error::Corpus(format!("资产同步失败：{}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::{APPRECIATION_SEED_FILE_NAME, AssetResolver};
    use crate::{CorpusConfig, CorpusHandle, Error, SCHEMA_VERSION};
    use rusqlite::{Connection, params};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "yunjian-assets-{label}-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("建临时目录");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        _root: TempDir,
        manifest: PathBuf,
        corpus: CorpusConfig,
        app_data: PathBuf,
        seed_sha256: String,
    }

    fn fixture(corrupt_seed_digest: bool) -> Fixture {
        let root = TempDir::new("fixture");
        let source = root.join("source");
        let corpus_data = root.join("corpus-data");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&source).expect("建来源目录");

        let corpus_db = source.join("corpus.db");
        write_corpus(&corpus_db, "corpus-v1");
        let corpus_archive = source.join("corpus.db.gz");
        gzip(&corpus_db, &corpus_archive);
        let corpus_sha256 = sha256(&std::fs::read(&corpus_archive).expect("读语料归档"));

        let seed = source.join("appreciations.json");
        let seed_bytes =
            r#"[{"stable_id":"fixture:1","template_version":"1.0.0","text":"内置赏析"}]"#
                .as_bytes();
        std::fs::write(&seed, seed_bytes).expect("写种子");
        let seed_sha256 = sha256(seed_bytes);
        let declared_seed_sha256 = if corrupt_seed_digest {
            "0".repeat(64)
        } else {
            seed_sha256.clone()
        };

        let manifest = root.join("assets_manifest.json");
        std::fs::write(
            &manifest,
            serde_json::to_vec_pretty(&json!({
                "corpus": {
                    "url": corpus_archive.to_string_lossy(),
                    "sha256": corpus_sha256,
                    "corpus_version": "corpus-v1",
                    "schema_version": SCHEMA_VERSION
                },
                "appreciation_seed": {
                    "url": seed.to_string_lossy(),
                    "sha256": declared_seed_sha256,
                    "template_version": "1.0.0",
                    "corpus_version": "corpus-v1",
                    "record_count": 1
                }
            }))
            .expect("序列化统一清单"),
        )
        .expect("写统一清单");

        Fixture {
            _root: root,
            manifest,
            corpus: CorpusConfig {
                path: None,
                data_dir: corpus_data,
                archive: None,
            },
            app_data,
            seed_sha256,
        }
    }

    #[test]
    fn a_fresh_profile_installs_the_corpus_and_imports_a_nonempty_seed_idempotently() {
        let fixture = fixture(false);
        let imports = AtomicUsize::new(0);
        let resolver =
            AssetResolver::new(&fixture.manifest, fixture.corpus.clone(), &fixture.app_data);

        for _ in 0..2 {
            resolver
                .sync(|seed, manifest, corpus| {
                    assert_eq!(corpus.meta().corpus_version, "corpus-v1");
                    assert_eq!(manifest.record_count, 1);
                    let records: Vec<serde_json::Value> =
                        serde_json::from_slice(&std::fs::read(seed.path()).expect("读已校验种子"))
                            .expect("解析种子");
                    assert!(!records.is_empty());
                    let database = fixture.app_data.join("appreciation.db");
                    let connection = Connection::open(database)?;
                    connection.execute_batch(
                        "CREATE TABLE IF NOT EXISTS appreciation_shipped(\
                         stable_id TEXT PRIMARY KEY NOT NULL, text TEXT NOT NULL);",
                    )?;
                    for record in records {
                        connection.execute(
                            "INSERT OR IGNORE INTO appreciation_shipped(stable_id, text) VALUES (?1, ?2)",
                            params![record["stable_id"].as_str(), record["text"].as_str()],
                        )?;
                    }
                    imports.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                })
                .expect("统一资产应可重复同步");
        }

        let connection =
            Connection::open(fixture.app_data.join("appreciation.db")).expect("开赏析库");
        let shipped: i64 = connection
            .query_row("SELECT count(*) FROM appreciation_shipped", [], |row| {
                row.get(0)
            })
            .expect("数随包赏析");
        assert!(shipped > 0, "净机同步后随包赏析必须非空");
        assert_eq!(
            imports.load(Ordering::Relaxed),
            2,
            "重跑仍须让导入器确认数据库状态"
        );
        assert!(
            fixture
                .corpus
                .data_dir
                .join(crate::CORPUS_FILE_NAME)
                .is_file()
        );
        assert_eq!(
            sha256(
                &std::fs::read(fixture.app_data.join(APPRECIATION_SEED_FILE_NAME))
                    .expect("读已发布种子")
            ),
            fixture.seed_sha256
        );
    }

    #[test]
    fn a_corrupted_seed_aborts_before_import_but_leaves_the_dictionary_usable() {
        let fixture = fixture(true);
        let imports = AtomicUsize::new(0);
        let resolver =
            AssetResolver::new(&fixture.manifest, fixture.corpus.clone(), &fixture.app_data);

        let error = resolver
            .sync(|_, _, _| {
                imports.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .expect_err("损坏种子必须中止");

        assert!(error.to_string().contains("赏析种子"));
        assert!(error.to_string().contains("摘要"));
        assert_eq!(imports.load(Ordering::Relaxed), 0, "摘要不符时不得开始写库");
        let handle = CorpusHandle::open(&fixture.corpus).expect("种子失败后字典仍须可用");
        assert_eq!(handle.meta().corpus_version, "corpus-v1");
    }

    #[test]
    fn an_interrupted_import_keeps_the_previous_seed_and_the_corpus() {
        let fixture = fixture(false);
        std::fs::create_dir_all(&fixture.app_data).expect("建应用数据目录");
        let published = fixture.app_data.join(APPRECIATION_SEED_FILE_NAME);
        std::fs::write(&published, b"previous-seed").expect("写旧种子");
        let resolver =
            AssetResolver::new(&fixture.manifest, fixture.corpus.clone(), &fixture.app_data);

        let error = resolver
            .sync(|_, _, _| Err(Error::Corpus("模拟事务中断".to_owned())))
            .expect_err("导入中断必须失败");

        assert!(error.to_string().contains("模拟事务中断"));
        assert_eq!(
            std::fs::read(&published).expect("旧种子仍在"),
            b"previous-seed"
        );
        assert!(
            CorpusHandle::open(&fixture.corpus).is_ok(),
            "导入失败不能破坏字典"
        );
    }

    fn write_corpus(path: &Path, corpus_version: &str) {
        let connection = Connection::open(path).expect("建语料 fixture");
        connection
            .execute_batch(
                "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);\
                 CREATE TABLE corpus_meta(\
                   singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),\
                   schema_version INTEGER NOT NULL, corpus_version TEXT NOT NULL,\
                   built_at TEXT NOT NULL, poem_count INTEGER NOT NULL,\
                   index_detail_mode TEXT NOT NULL, derived_indexes TEXT NOT NULL,\
                   shipped_scope TEXT NOT NULL, integrity_check TEXT NOT NULL);",
            )
            .expect("建 schema");
        connection
            .execute(
                "INSERT INTO poem(stable_id, body) VALUES ('fixture:1', '床前明月光，疑是地上霜。')",
                [],
            )
            .expect("写诗");
        connection
            .execute(
                "INSERT INTO corpus_meta VALUES (1, ?1, ?2, '2026-08-13T00:00:00Z', 1,\
                 'full', 'first_launch', '10k', 'ok')",
                params![SCHEMA_VERSION, corpus_version],
            )
            .expect("写元数据");
    }

    fn gzip(source: &Path, target: &Path) {
        let file = std::fs::File::create(target).expect("建 gzip");
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        encoder
            .write_all(&std::fs::read(source).expect("读待压缩语料"))
            .expect("压缩语料");
        encoder.finish().expect("结束压缩");
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
