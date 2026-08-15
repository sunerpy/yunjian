//! 落地与只读打开的用例。
//!
//! # fixture 为什么自己写 DDL
//!
//! 真实 schema 住在 `yunjian-corpus/schema.sql`，而依赖方向是 corpus -> core，本 crate
//! 拿不到它。这里的 fixture 只建本模块真正读的那几列，并保留它们在真实 schema 里的
//! CHECK 约束。**两份 DDL 的漂移由 `yunjian-corpus` 侧的
//! `a_real_built_corpus_opens_through_the_core_handle` 抓**：那条用例用真实 `SCHEMA_SQL`
//! 建库，再走本模块的 [`CorpusHandle::open`]，所以「core 读的列在随包 schema 里不存在」
//! 会立刻变红。

use super::*;
use rusqlite::params;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// 不引入 `tempfile` 依赖的最小临时目录，析构时递归删除。
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yunjian-corpus-open-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("建临时目录");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
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

const FIXTURE_POEMS: [(&str, &str); 3] = [
    ("fixture:0001", "床前明月光，疑是地上霜。"),
    ("fixture:0002", "海上生明月，天涯共此时。"),
    ("fixture:0003", "国破山河在，城春草木深。"),
];

/// 建一份能通过 [`open_corpus`] 与 [`CorpusMeta::read`] 的语料库文件。
fn write_fixture_corpus(path: &Path, schema_version: u32, corpus_version: &str) {
    let connection = Connection::open(path).expect("建 fixture 库");
    connection
        .execute_batch(
            "PRAGMA page_size = 4096;
             CREATE TABLE poem (
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE TABLE corpus_meta (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
                 schema_version INTEGER NOT NULL CHECK (schema_version > 0),
                 corpus_version TEXT NOT NULL,
                 built_at TEXT NOT NULL,
                 poem_count INTEGER NOT NULL CHECK (poem_count >= 0),
                 index_detail_mode TEXT NOT NULL
                     CHECK (index_detail_mode IN ('none', 'column', 'full')),
                 derived_indexes TEXT NOT NULL
                     CHECK (derived_indexes IN ('shipped', 'first_launch')),
                 shipped_scope TEXT NOT NULL CHECK (shipped_scope IN ('10k', 'tang-song', 'full')),
                 integrity_check TEXT NOT NULL
             );",
        )
        .expect("建 fixture schema");
    for (stable_id, body) in FIXTURE_POEMS {
        connection
            .execute(
                "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
                params![stable_id, body],
            )
            .expect("写 fixture 诗");
    }
    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, schema_version, corpus_version, built_at, \
                                     poem_count, index_detail_mode, derived_indexes, \
                                     shipped_scope, integrity_check) \
             VALUES (1, ?1, ?2, '2026-08-11T00:00:00Z', ?3, 'full', 'first_launch', '10k', 'ok')",
            params![schema_version, corpus_version, FIXTURE_POEMS.len() as i64],
        )
        .expect("写 fixture 元数据");
    connection.close().expect("关闭 fixture 库");
}

fn gzip(source: &Path, destination: &Path) {
    let input = std::fs::read(source).expect("读 fixture 库");
    let file = std::fs::File::create(destination).expect("建归档");
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::new(9));
    encoder.write_all(&input).expect("压缩");
    encoder.finish().expect("收尾").sync_all().expect("落盘");
}

/// 打包端产出的三件套：`.db.gz` + `manifest.json` + `.sha256`。
struct Archive {
    path: PathBuf,
    sha256: String,
    size_bytes: u64,
    uncompressed_bytes: u64,
}

fn stage_archive(dir: &TempDir, corpus_version: &str, schema_version: u32) -> Archive {
    let staging = dir.join("staging.db");
    write_fixture_corpus(&staging, schema_version, corpus_version);
    let uncompressed_bytes = std::fs::metadata(&staging).expect("量体积").len();
    let name = format!("yunjian-corpus-{corpus_version}.db.gz");
    let archive = dir.join(&name);
    gzip(&staging, &archive);
    std::fs::remove_file(&staging).expect("删掉暂存库");
    let sha256 = sha256_of_file(&archive).expect("算摘要");
    let size_bytes = std::fs::metadata(&archive).expect("量归档").len();
    Archive {
        path: archive,
        sha256,
        size_bytes,
        uncompressed_bytes,
    }
}

fn write_manifest(
    dir: &TempDir,
    archive: &Archive,
    corpus_version: &str,
    schema_version: u32,
    min_app_version: &str,
) {
    let name = archive
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .expect("归档文件名");
    let manifest = format!(
        "{{\n  \"schema_version\": {schema_version},\n  \
         \"corpus_version\": \"{corpus_version}\",\n  \
         \"min_app_version\": \"{min_app_version}\",\n  \
         \"record_count\": {records},\n  \
         \"artifact_name\": \"{name}\",\n  \
         \"size_bytes\": {size},\n  \
         \"sha256\": \"{sha}\",\n  \
         \"uncompressed_bytes\": {uncompressed}\n}}\n",
        records = FIXTURE_POEMS.len(),
        size = archive.size_bytes,
        sha = archive.sha256,
        uncompressed = archive.uncompressed_bytes,
    );
    std::fs::write(dir.join(CORPUS_MANIFEST_NAME), manifest).expect("写清单");
}

fn write_sidecar(archive: &Archive) {
    let name = archive
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .expect("归档文件名");
    std::fs::write(
        sidecar_path(&archive.path),
        format!("{}  {name}\n", archive.sha256),
    )
    .expect("写摘要旁文件");
}

fn config(dir: &TempDir) -> CorpusConfig {
    CorpusConfig {
        path: None,
        data_dir: dir.path().to_path_buf(),
        archive: None,
    }
}

/// 进度事件带借用字段，所以在回调里就折叠成标签，不留生命周期。
#[derive(Default)]
struct Events {
    labels: Vec<String>,
    derive_steps: Vec<DeriveStep>,
    decompress_reports: usize,
}

impl Events {
    fn record(&mut self, event: MaterializationProgress<'_>) {
        let label = match event {
            MaterializationProgress::AlreadyPresent { .. } => "already-present",
            MaterializationProgress::VerifyingArchive { .. } => "verifying",
            MaterializationProgress::ArchiveVerified { .. } => "verified",
            MaterializationProgress::Decompressing { .. } => {
                self.decompress_reports += 1;
                "decompressing"
            }
            MaterializationProgress::Materialized { .. } => "materialized",
            MaterializationProgress::Deriving(progress) => {
                self.derive_steps.push(progress.step);
                "deriving"
            }
            MaterializationProgress::DeriveFailed { .. } => "derive-failed",
            MaterializationProgress::Ready { .. } => "ready",
        };
        self.labels.push(label.to_owned());
    }

    fn saw(&self, label: &str) -> bool {
        self.labels.iter().any(|seen| seen == label)
    }
}

fn open_with_events(cfg: &CorpusConfig) -> (Result<CorpusHandle>, Events) {
    let mut events = Events::default();
    let outcome = CorpusHandle::open_with_progress(cfg, &mut |event| events.record(event));
    (outcome, events)
}

#[test]
fn an_explicit_configured_path_is_used_as_is_and_nothing_is_materialized() {
    let dir = TempDir::new("configured");
    let explicit = dir.join("elsewhere.db");
    write_fixture_corpus(&explicit, SCHEMA_VERSION, "1.2.3");
    let cfg = CorpusConfig {
        path: Some(explicit.clone()),
        data_dir: dir.join("data"),
        archive: None,
    };

    let (handle, events) = open_with_events(&cfg);
    let handle = handle.expect("显式路径应当直接可用");
    assert_eq!(handle.path(), explicit);
    assert_eq!(handle.origin(), &CorpusOrigin::Configured);
    assert_eq!(handle.meta().corpus_version, "1.2.3");
    assert!(events.saw("already-present"));
    assert!(!events.saw("verifying"), "显式路径不该触发归档校验");
    assert!(
        !cfg.data_dir.join(CORPUS_FILE_NAME).exists(),
        "显式路径不该在数据目录里落地任何副本"
    );
}

#[test]
fn a_configured_path_that_does_not_exist_is_an_error_not_a_silent_fallback() {
    let dir = TempDir::new("configured-missing");
    let archive = stage_archive(&dir, "1.2.3", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.2.3", SCHEMA_VERSION, "0.1.0");
    let cfg = CorpusConfig {
        path: Some(dir.join("does-not-exist.db")),
        data_dir: dir.path().to_path_buf(),
        archive: None,
    };

    let error = CorpusHandle::open(&cfg).expect_err("显式指向不存在的文件必须报错");
    let message = error.to_string();
    assert!(
        message.contains("does-not-exist.db"),
        "unexpected: {message}"
    );
    assert!(
        message.contains("不会被静默忽略"),
        "错误必须说明为什么不回退：{message}"
    );
    assert!(
        !dir.join(CORPUS_FILE_NAME).exists(),
        "报错的解析不该顺手落地一份语料"
    );
}

#[test]
fn a_verified_archive_is_materialized_atomically_and_recorded_with_its_corpus_version() {
    let dir = TempDir::new("materialize");
    let archive = stage_archive(&dir, "2.0.1", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "2.0.1", SCHEMA_VERSION, "0.1.0");
    let cfg = config(&dir);

    let (handle, events) = open_with_events(&cfg);
    let handle = handle.expect("校验通过的归档应当落地");
    assert_eq!(handle.path(), dir.join(CORPUS_FILE_NAME));
    assert_eq!(handle.meta().corpus_version, "2.0.1");
    assert_eq!(handle.meta().schema_version, SCHEMA_VERSION);
    assert_eq!(handle.index_detail_mode(), "full");
    match handle.origin() {
        CorpusOrigin::JustMaterialized {
            archive: recorded,
            sha256,
        } => {
            assert_eq!(recorded, &archive.path);
            assert_eq!(sha256, &archive.sha256);
        }
        other => panic!("应当记录为刚落地：{other:?}"),
    }
    for label in [
        "verifying",
        "verified",
        "decompressing",
        "materialized",
        "ready",
    ] {
        assert!(events.saw(label), "缺进度事件 {label}");
    }
    assert!(
        !events.saw("derive-failed"),
        "可写目录下的首启派生不该失败：{:?}",
        events.labels
    );
    assert!(handle.derived().is_ready(), "首启派生应当就绪");
    assert!(
        std::fs::read_dir(dir.path())
            .expect("列目录")
            .filter_map(std::result::Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(TEMP_SUFFIX)),
        "落地成功后不该留临时文件"
    );
    assert!(
        !deriving_marker(handle.path()).exists(),
        "派生成功后标记必须被删除"
    );
}

#[test]
fn an_already_materialized_copy_is_reused_without_reading_the_archive() {
    let dir = TempDir::new("reuse");
    write_fixture_corpus(&dir.join(CORPUS_FILE_NAME), SCHEMA_VERSION, "3.0.0");
    // 一个必然校验失败的归档：只要解析顺序把已落地的副本排在归档之前，它就不会被读。
    std::fs::write(dir.join(CORPUS_ARCHIVE_NAME), b"not a gzip at all").expect("写坏归档");

    let (handle, events) = open_with_events(&config(&dir));
    let handle = handle.expect("已落地的副本应当直接可用");
    assert_eq!(handle.origin(), &CorpusOrigin::Materialized);
    assert_eq!(handle.meta().corpus_version, "3.0.0");
    assert!(!events.saw("verifying"), "已落地时不该去碰归档");
}

#[test]
fn a_checksum_mismatch_aborts_materialization_and_leaves_no_file() {
    let dir = TempDir::new("checksum");
    let archive = stage_archive(&dir, "1.0.0", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.0.0", SCHEMA_VERSION, "0.1.0");
    // 翻转归档中间的一个比特：字节数不变，所以只有摘要能抓到它。
    let mut bytes = std::fs::read(&archive.path).expect("读归档");
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 0b0000_0001;
    std::fs::write(&archive.path, &bytes).expect("写回被篡改的归档");

    let error = CorpusHandle::open(&config(&dir)).expect_err("摘要不符必须中止落地");
    let message = error.to_string();
    assert!(message.contains("摘要"), "unexpected: {message}");
    assert!(
        message.contains(&archive.sha256),
        "错误应给出期望摘要：{message}"
    );
    assert!(
        !dir.join(CORPUS_FILE_NAME).exists(),
        "摘要不符时目标目录不得留下 {CORPUS_FILE_NAME}"
    );
    let leftovers = std::fs::read_dir(dir.path())
        .expect("列目录")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(TEMP_SUFFIX) || name == CORPUS_FILE_NAME)
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "不该留下任何半成品：{leftovers:?}");
}

#[test]
fn a_truncated_archive_is_diagnosed_as_truncated_rather_than_as_a_bad_digest() {
    let dir = TempDir::new("truncated");
    let archive = stage_archive(&dir, "1.0.0", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.0.0", SCHEMA_VERSION, "0.1.0");
    let mut bytes = std::fs::read(&archive.path).expect("读归档");
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&archive.path, &bytes).expect("写回被截断的归档");

    let error = CorpusHandle::open(&config(&dir)).expect_err("字节数不符必须中止");
    let message = error.to_string();
    assert!(
        message.contains("截断"),
        "字节数不符要给出比「摘要不符」更具体的诊断：{message}"
    );
    assert!(!dir.join(CORPUS_FILE_NAME).exists());
}

#[test]
fn an_archive_with_no_expected_digest_is_refused_rather_than_trusted() {
    let dir = TempDir::new("unverifiable");
    let staging = dir.join("staging.db");
    write_fixture_corpus(&staging, SCHEMA_VERSION, "1.0.0");
    gzip(&staging, &dir.join(CORPUS_ARCHIVE_NAME));
    std::fs::remove_file(&staging).expect("删暂存");

    let error = CorpusHandle::open(&config(&dir)).expect_err("无从校验的归档不得落地");
    let message = error.to_string();
    assert!(message.contains("期望摘要"), "unexpected: {message}");
    assert!(!dir.join(CORPUS_FILE_NAME).exists());
}

#[test]
fn a_sha256_sidecar_is_accepted_when_no_manifest_describes_the_archive() {
    let dir = TempDir::new("sidecar");
    let archive = stage_archive(&dir, "1.4.0", SCHEMA_VERSION);
    write_sidecar(&archive);
    let cfg = CorpusConfig {
        path: None,
        data_dir: dir.path().to_path_buf(),
        archive: Some(archive.path.clone()),
    };

    let handle = CorpusHandle::open(&cfg).expect("旁文件也应当能作为期望摘要");
    assert_eq!(handle.meta().corpus_version, "1.4.0");
}

#[test]
fn an_interrupted_materialization_restarts_cleanly_and_produces_a_valid_corpus() {
    let dir = TempDir::new("interrupted");
    let archive = stage_archive(&dir, "1.0.0", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.0.0", SCHEMA_VERSION, "0.1.0");
    // 模拟被杀掉的上一次落地：一个半写的临时文件躺在目标目录里。
    let stale = dir.join(&format!("{CORPUS_FILE_NAME}.999999.0{TEMP_SUFFIX}"));
    std::fs::write(&stale, b"half-written garbage from a killed run").expect("写残留临时文件");

    let handle = CorpusHandle::open(&config(&dir)).expect("残留临时文件不该妨碍重跑");
    assert_eq!(handle.path(), dir.join(CORPUS_FILE_NAME));
    assert_eq!(handle.meta().corpus_version, "1.0.0");
    assert!(!stale.exists(), "残留的临时文件应当被清扫");
    let connection = handle.connect().expect("落地后的语料库必须可读");
    let poems: i64 = connection
        .query_row("SELECT count(*) FROM poem", [], |row| row.get(0))
        .expect("数诗");
    assert_eq!(poems, FIXTURE_POEMS.len() as i64);
}

#[test]
fn a_partial_temp_file_is_never_mistaken_for_the_materialized_corpus() {
    let dir = TempDir::new("temp-naming");
    let target = dir.join(CORPUS_FILE_NAME);
    let temp = temp_path(&target);
    assert_ne!(temp, target);
    let name = temp
        .file_name()
        .and_then(|value| value.to_str())
        .expect("名字");
    assert!(
        name.starts_with(CORPUS_FILE_NAME),
        "temp 必须与目标同前缀：{name}"
    );
    assert!(name.ends_with(TEMP_SUFFIX), "temp 必须带临时后缀：{name}");
    assert_eq!(temp.parent(), target.parent(), "temp 必须与目标同目录");
    assert_ne!(temp_path(&target), temp_path(&target), "temp 名字必须唯一");
}

#[test]
fn any_insert_against_the_opened_handle_is_a_read_only_error() {
    let dir = TempDir::new("readonly");
    write_fixture_corpus(&dir.join(CORPUS_FILE_NAME), SCHEMA_VERSION, "1.0.0");
    let handle = CorpusHandle::open(&config(&dir)).expect("打开");
    let connection = handle.connect().expect("取连接");

    let query_only: i64 = connection
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .expect("读 query_only");
    assert_eq!(query_only, 1);
    let error = connection
        .execute(
            "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
            params!["fixture:9999", "不应写入"],
        )
        .expect_err("只读句柄必须拒绝 INSERT");
    assert!(
        error.to_string().contains("readonly"),
        "unexpected: {error}"
    );
    let update = connection
        .execute("UPDATE corpus_meta SET corpus_version='9.9.9'", [])
        .expect_err("只读句柄必须拒绝 UPDATE");
    assert!(
        update.to_string().contains("readonly"),
        "unexpected: {update}"
    );
}

#[test]
fn an_out_of_range_schema_version_is_rejected_with_an_actionable_message() {
    let dir = TempDir::new("schema");
    let unsupported = SUPPORTED_SCHEMA.end() + 1;
    write_fixture_corpus(&dir.join(CORPUS_FILE_NAME), unsupported, "1.0.0");

    let error = CorpusHandle::open(&config(&dir)).expect_err("越界 schema 必须被拒");
    let message = error.to_string();
    assert!(
        message.contains(&unsupported.to_string()),
        "unexpected: {message}"
    );
    assert!(
        message.contains("yunjian corpus fetch"),
        "unexpected: {message}"
    );
}

#[test]
fn a_manifest_demanding_a_newer_app_is_refused_before_anything_is_written() {
    let dir = TempDir::new("min-app");
    let archive = stage_archive(&dir, "1.0.0", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.0.0", SCHEMA_VERSION, "99.0.0");

    let error = CorpusHandle::open(&config(&dir)).expect_err("要求更新的应用必须被拒");
    let message = error.to_string();
    assert!(message.contains("99.0.0"), "unexpected: {message}");
    assert!(message.contains("升级应用"), "unexpected: {message}");
    assert!(!dir.join(CORPUS_FILE_NAME).exists());
}

#[test]
fn a_manifest_declaring_an_unsupported_schema_is_refused_before_decompression() {
    let dir = TempDir::new("manifest-schema");
    let unsupported = SUPPORTED_SCHEMA.end() + 1;
    let archive = stage_archive(&dir, "1.0.0", unsupported);
    write_manifest(&dir, &archive, "1.0.0", unsupported, "0.1.0");

    let error = CorpusHandle::open(&config(&dir)).expect_err("清单声明的越界 schema 必须被拒");
    let message = error.to_string();
    assert!(
        message.contains("yunjian corpus fetch"),
        "unexpected: {message}"
    );
    assert!(!dir.join(CORPUS_FILE_NAME).exists());
}

#[test]
fn first_launch_derivation_runs_once_and_reports_every_step() {
    let dir = TempDir::new("derive");
    write_fixture_corpus(&dir.join(CORPUS_FILE_NAME), SCHEMA_VERSION, "1.0.0");

    let (handle, events) = open_with_events(&config(&dir));
    let handle = handle.expect("首启派生应当成功");
    match handle.derived() {
        DerivedState::Ready { stats } => {
            let stats = stats.expect("本次运行构建过，应当带实测量");
            assert_eq!(stats.poems, FIXTURE_POEMS.len() as u64);
            assert!(stats.grams > 0);
        }
        other => panic!("派生应当就绪：{other:?}"),
    }
    for step in [
        DeriveStep::Scan,
        DeriveStep::Ngram,
        DeriveStep::LastChar,
        DeriveStep::Fts,
    ] {
        assert!(
            events.derive_steps.contains(&step),
            "缺 {step:?} 的进度事件：{:?}",
            events.derive_steps
        );
    }

    // 第二次打开不该再派生一遍：571.8 s 的代价只付一次。
    let (second, second_events) = open_with_events(&config(&dir));
    let second = second.expect("第二次打开");
    assert_eq!(second.derived(), &DerivedState::Ready { stats: None });
    assert!(
        second_events.derive_steps.is_empty(),
        "已就绪时不该重跑派生：{:?}",
        second_events.derive_steps
    );
}

#[test]
fn an_interrupted_derivation_is_detected_by_the_marker_and_redone() {
    let dir = TempDir::new("derive-marker");
    let corpus = dir.join(CORPUS_FILE_NAME);
    write_fixture_corpus(&corpus, SCHEMA_VERSION, "1.0.0");
    CorpusHandle::open(&config(&dir)).expect("先建出派生结构");
    {
        let connection = connect_read_only(&corpus).expect("只读连接");
        assert!(
            derived_indexes_present(&connection).expect("查结构"),
            "前置条件：三张结构都在"
        );
    }
    // 表与索引都在，光看它们会判为已完成——标记是唯一能说出「上次被打断」的东西。
    std::fs::write(deriving_marker(&corpus), b"interrupted").expect("写标记");

    let (handle, events) = open_with_events(&config(&dir));
    let handle = handle.expect("带标记时应当重建而不是报错");
    assert!(handle.derived().is_ready());
    assert!(
        !events.derive_steps.is_empty(),
        "标记在就必须重建：{:?}",
        events.labels
    );
    assert!(!deriving_marker(&corpus).exists(), "重建成功后标记必须被删");
}

#[test]
fn a_failed_derivation_leaves_the_dictionary_usable() {
    let dir = TempDir::new("derive-failure");
    let corpus = dir.join(CORPUS_FILE_NAME);
    write_fixture_corpus(&corpus, SCHEMA_VERSION, "1.0.0");
    // 拿一个**目录**占住标记的位置：`exists()` 为真所以判定「需要派生」，
    // 而写它必然失败。任何平台上都确定失败，且不依赖文件权限。
    std::fs::create_dir(deriving_marker(&corpus)).expect("占住标记位置");

    let (handle, events) = open_with_events(&config(&dir));
    let handle = handle.expect("派生失败不该让整个打开失败");
    match handle.derived() {
        DerivedState::Unavailable { reason } => assert!(!reason.is_empty(), "必须带原因"),
        other => panic!("应当报告不可用：{other:?}"),
    }
    assert!(events.saw("derive-failed"));
    assert!(events.saw("ready"), "字典仍然可用，因此仍要报 ready");

    let connection = handle.connect().expect("字典仍可查");
    let poems: i64 = connection
        .query_row("SELECT count(*) FROM poem", [], |row| row.get(0))
        .expect("数诗");
    assert_eq!(poems, FIXTURE_POEMS.len() as i64);
}

#[test]
fn the_handle_is_cheap_to_clone_and_usable_from_several_threads() {
    fn assert_shareable<T: Send + Sync + Clone + 'static>() {}
    assert_shareable::<CorpusHandle>();

    let dir = TempDir::new("threads");
    write_fixture_corpus(&dir.join(CORPUS_FILE_NAME), SCHEMA_VERSION, "1.0.0");
    let handle = CorpusHandle::open(&config(&dir)).expect("打开");
    let clone = handle.clone();
    assert!(
        Arc::ptr_eq(&handle.inner, &clone.inner),
        "克隆必须共享同一份内部状态，否则不是 Arc 级别的开销"
    );

    let workers = (0..4)
        .map(|_| {
            let handle = handle.clone();
            std::thread::spawn(move || {
                // 每个 worker 自己开连接：Connection 不是 Sync，共用一个是错的。
                let connection = handle.connect().expect("worker 取连接");
                connection
                    .query_row("SELECT count(*) FROM poem", [], |row| row.get::<_, i64>(0))
                    .expect("worker 查询")
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        assert_eq!(
            worker.join().expect("worker 未 panic"),
            FIXTURE_POEMS.len() as i64
        );
    }
}

#[test]
fn a_missing_corpus_names_every_place_that_was_looked_at() {
    let dir = TempDir::new("missing");
    let error = CorpusHandle::open(&config(&dir)).expect_err("什么都没有时必须报错");
    let message = error.to_string();
    for expected in [CORPUS_FILE_NAME, CORPUS_MANIFEST_NAME, CORPUS_ARCHIVE_NAME] {
        assert!(
            message.contains(expected),
            "错误应提到 {expected}：{message}"
        );
    }
}

#[test]
fn a_manifest_naming_an_absent_artifact_is_an_error_not_a_fallback() {
    let dir = TempDir::new("manifest-orphan");
    let archive = stage_archive(&dir, "1.0.0", SCHEMA_VERSION);
    write_manifest(&dir, &archive, "1.0.0", SCHEMA_VERSION, "0.1.0");
    std::fs::remove_file(&archive.path).expect("删掉归档");
    // 同时放一个约定名的归档：清单指名的那个不在时，不该悄悄改用另一个文件。
    std::fs::write(dir.join(CORPUS_ARCHIVE_NAME), b"another file").expect("写约定名归档");

    let error = CorpusHandle::open(&config(&dir)).expect_err("清单与归档必须成对");
    assert!(
        error.to_string().contains("成对存在"),
        "unexpected: {error}"
    );
}

#[test]
fn the_sidecar_parser_requires_a_line_naming_this_archive() {
    let sidecar = Path::new("/tmp/yunjian-corpus-1.0.0.db.gz.sha256");
    let digest = "a".repeat(64);
    assert_eq!(
        parse_sha256_sidecar(
            &format!("{digest}  wanted.db.gz\n"),
            "wanted.db.gz",
            sidecar
        )
        .expect("标准两空格格式"),
        digest
    );
    assert_eq!(
        parse_sha256_sidecar(
            &format!("{digest}  *wanted.db.gz\n"),
            "wanted.db.gz",
            sidecar
        )
        .expect("二进制模式的星号前缀"),
        digest
    );
    assert_eq!(
        parse_sha256_sidecar(
            &format!("{}\n", digest.to_uppercase()),
            "wanted.db.gz",
            sidecar
        )
        .expect("只有摘要、没有文件名"),
        digest,
        "大写十六进制必须折叠成小写，否则与自己算出的摘要永远不相等"
    );
    let error = parse_sha256_sidecar(&format!("{digest}  other.db.gz\n"), "wanted.db.gz", sidecar)
        .expect_err("描述别的工件的旁文件证明不了这个归档");
    assert!(error.to_string().contains("wanted.db.gz"));
    let malformed = parse_sha256_sidecar("nothex  wanted.db.gz\n", "wanted.db.gz", sidecar)
        .expect_err("非十六进制必须被拒");
    assert!(malformed.to_string().contains("SHA-256"));
}

#[test]
fn version_comparison_ignores_prerelease_and_build_metadata() {
    assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
    assert_eq!(parse_version("0.2.0-rc.1"), Some((0, 2, 0)));
    assert_eq!(parse_version("0.2.0+build.7"), Some((0, 2, 0)));
    assert_eq!(parse_version("1.2"), None);
    assert_eq!(parse_version("1.2.3.4"), None);
    assert_eq!(parse_version("v1.2.3"), None);
    // 预发布不该被判为「不满足 min_app_version = 0.2.0」——它读得动同一份语料。
    assert!(parse_version("0.2.0-rc.1") >= parse_version("0.2.0"));
}

/// 「core 永不感知 Tauri」这条决定要有牙齿。
///
/// 之前它只是 `lib.rs` 顶上的一句话加人工 review。落地模块是最容易破这条线的地方——
/// 「随包资产在哪」在桌面外壳的框架里有现成 API，顺手 import 它的路径解析器就完事，
/// 而那一行会把移动端的实现空间焊死。所以这里改成读自己的 `Cargo.toml` 断言。
///
/// 判的是**依赖**而不是源码里的字符串：`lib.rs` 与本文件都要能写出这个框架的名字来
/// 解释为什么不用它，禁掉字符串会把说明也一起禁掉。而不给依赖，`use` 它的任何类型都
/// 根本编译不过——依赖才是那条真正的边界。
#[test]
fn this_crate_declares_no_shell_framework_dependency() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("读自己的 Cargo.toml");
    for line in manifest.lines() {
        let code = line.split('#').next().unwrap_or_default();
        assert!(
            !code.contains(concat!("tau", "ri")),
            "yunjian-core 不得依赖 Tauri（决定 2：core 永不感知外壳），命中：{line}"
        );
    }
}

#[test]
fn a_sibling_suffix_appends_rather_than_replacing_the_extension() {
    let corpus = Path::new("/data/corpus.db");
    assert_eq!(
        deriving_marker(corpus),
        Path::new("/data/corpus.db.deriving"),
        "标记必须是 corpus.db 的旁文件，而不是把 .db 换成 .deriving"
    );
    assert_eq!(
        sidecar_path(Path::new("/data/yunjian-corpus-1.0.0.db.gz")),
        Path::new("/data/yunjian-corpus-1.0.0.db.gz.sha256")
    );
}
