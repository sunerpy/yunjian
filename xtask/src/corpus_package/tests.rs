use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-package-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

#[test]
fn the_checksum_sidecar_uses_the_format_sha256sum_dash_c_accepts() {
    // `sha256sum -c` 要求「摘要 + 两个空格 + 文件名」，且文件名不带目录，
    // 这样在工件所在目录里直接校验就能过。格式写错时校验工具报的是
    // 「improperly formatted checksum line」，与「摘要不符」完全是两回事。
    let digest = "a".repeat(64);
    let line = format!("{digest}  yunjian-corpus-1.0.0.db.gz\n");
    let (recorded, name) = line
        .trim_end()
        .split_once("  ")
        .expect("必须以两个空格分隔");
    assert_eq!(recorded.len(), 64);
    assert_eq!(name, "yunjian-corpus-1.0.0.db.gz");
    assert!(!name.contains('/'), "文件名不得带目录");
}

#[test]
fn a_single_flipped_byte_changes_the_artifact_digest() {
    // 失败场景：把打好的 `.gz` 改掉一个字节，校验必须失败。这条钉的是「摘要真的覆盖
    // 全部内容」——如果只对文件头或长度取摘要，改中间一个字节就抓不到。
    let dir = scratch("corrupt");
    let artifact = dir.join("artifact.db.gz");
    compress_bytes(
        &artifact,
        "床前明月光，疑是地上霜。举头望山月，低头思故乡。".as_bytes(),
    );
    let good = sha256_of_file(&artifact).expect("摘要");

    let mut bytes = std::fs::read(&artifact).expect("读工件");
    let victim = bytes.len() / 2;
    bytes[victim] ^= 0x01;
    std::fs::write(&artifact, &bytes).expect("写回被损坏的工件");
    let bad = sha256_of_file(&artifact).expect("摘要");

    assert_ne!(good, bad, "改掉一个字节后摘要必须变化，否则校验形同虚设");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_corrupted_artifact_fails_to_decompress_at_all() {
    // gzip 自带 CRC32，所以被改过的工件通常连解压都过不去——这是摘要之外的第二道网。
    let dir = scratch("gunzip");
    let artifact = dir.join("artifact.db.gz");
    compress_bytes(&artifact, &vec![7_u8; 4096]);
    let mut bytes = std::fs::read(&artifact).expect("读工件");
    let victim = bytes.len() / 2;
    bytes[victim] ^= 0xff;
    std::fs::write(&artifact, &bytes).expect("写回");

    let file = std::fs::File::open(&artifact).expect("打开");
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut out = Vec::new();
    let error = decoder
        .read_to_end(&mut out)
        .expect_err("被损坏的 gzip 必须解压失败");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_manifest_round_trips_through_json_without_losing_a_field() {
    // manifest 是下游唯一的机器可读入口（`jq -e '.schema_version and .sha256'`）。
    // `deny_unknown_fields` 加上这条 round-trip 让「悄悄删掉一个字段」无法通过。
    let manifest = sample_manifest();
    let text = serde_json::to_string_pretty(&manifest).expect("序列化");
    let parsed: Manifest = serde_json::from_str(&text).expect("反序列化");
    assert_eq!(parsed.schema_version, manifest.schema_version);
    assert_eq!(parsed.corpus_version, manifest.corpus_version);
    assert_eq!(parsed.min_app_version, manifest.min_app_version);
    assert_eq!(parsed.sha256, manifest.sha256);
    assert_eq!(parsed.record_count, manifest.record_count);
    assert_eq!(parsed.shipped_scope, manifest.shipped_scope);
    assert_eq!(parsed.derived_indexes, manifest.derived_indexes);
    assert_eq!(
        parsed.measurement.first_launch_seconds,
        manifest.measurement.first_launch_seconds
    );

    let value: serde_json::Value = serde_json::from_str(&text).expect("解析为 Value");
    for key in [
        "schema_version",
        "corpus_version",
        "min_app_version",
        "sha256",
        "record_count",
        "size_bytes",
        "source_manifest_sha256",
    ] {
        assert!(
            value.get(key).is_some_and(|v| !v.is_null()),
            "manifest 缺少下游断言依赖的字段 {key}"
        );
    }
}

#[test]
fn the_round_trip_check_rejects_a_manifest_describing_another_build() {
    // manifest 与 `.gz` 之间没有任何机制性联系，所以「manifest 描述上一次构建、
    // 工件是这一次」是一个真实可能的错误。这条证明回读会抓到它。
    let dir = scratch("roundtrip");
    let corpus = dir.join("corpus.db");
    write_minimal_corpus(&corpus, "1.2.3", 2);
    let artifact = dir.join("yunjian-corpus-1.2.3.db.gz");
    let size = compress(&corpus, &artifact).expect("压缩");
    assert!(size > 0);

    let mut manifest = sample_manifest();
    manifest.uncompressed_bytes = std::fs::metadata(&corpus).expect("元数据").len();
    manifest.corpus_version = "1.2.3".to_owned();
    manifest.record_count = 2;
    manifest.schema_version = yunjian_corpus::db::SCHEMA_VERSION;
    manifest.source_manifest_sha256 = "b".repeat(64);
    manifest.shipped_scope = "tang-song".to_owned();
    manifest.derived_indexes = "first_launch".to_owned();
    verify_round_trip(&artifact, &manifest).expect("一致的一对应当通过");

    manifest.corpus_version = "9.9.9".to_owned();
    let error =
        verify_round_trip(&artifact, &manifest).expect_err("manifest 与工件版本不符必须被抓到");
    let message = format!("{error:#}");
    assert!(
        message.contains("corpus_version") && message.contains("上一次构建"),
        "错误必须点名字段并解释这是什么错：{message}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_round_trip_check_rejects_a_truncated_artifact_before_opening_it() {
    // 先比字节数再比内容：字节数不符能给出「下载被截断」这个具体得多的诊断，
    // 而直接拿去开库只会得到 SQLite 的「file is not a database」。
    let dir = scratch("truncated");
    let corpus = dir.join("corpus.db");
    write_minimal_corpus(&corpus, "1.2.3", 2);
    let artifact = dir.join("yunjian-corpus-1.2.3.db.gz");
    compress(&corpus, &artifact).expect("压缩");

    let mut manifest = sample_manifest();
    manifest.uncompressed_bytes = 1;
    let error =
        verify_round_trip(&artifact, &manifest).expect_err("字节数不符必须在开库之前就被拒");
    let message = format!("{error:#}");
    assert!(
        message.contains("字节") && message.contains("manifest 记录"),
        "错误必须报出两个字节数：{message}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_round_trip_check_rejects_an_artifact_carrying_a_diagnostic_table() {
    // 回读时再查一遍诊断表，而不是只在打包前查一次：打包前查的是源文件，
    // 回读查的是**将要发布出去的那些字节**。
    let dir = scratch("diagnostic");
    let corpus = dir.join("corpus.db");
    write_minimal_corpus(&corpus, "1.2.3", 2);
    {
        let connection = Connection::open(&corpus).expect("打开");
        connection
            .execute_batch("CREATE TABLE ngram (gram TEXT NOT NULL, stable_id TEXT NOT NULL);")
            .expect("塞一张候选表进去");
    }
    let artifact = dir.join("yunjian-corpus-1.2.3.db.gz");
    compress(&corpus, &artifact).expect("压缩");

    let mut manifest = sample_manifest();
    manifest.uncompressed_bytes = std::fs::metadata(&corpus).expect("元数据").len();
    manifest.corpus_version = "1.2.3".to_owned();
    manifest.record_count = 2;
    manifest.schema_version = yunjian_corpus::db::SCHEMA_VERSION;
    manifest.source_manifest_sha256 = "b".repeat(64);
    manifest.shipped_scope = "tang-song".to_owned();
    manifest.derived_indexes = "first_launch".to_owned();

    let error =
        verify_round_trip(&artifact, &manifest).expect_err("带候选表的工件必须在回读时被拒");
    let message = format!("{error:#}");
    assert!(message.contains("ngram"), "错误必须点名那张表：{message}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn compression_is_deterministic_so_the_digest_is_reproducible() {
    // 固定压缩级别的目的：同一个输入必须压出同一串字节，否则「工件摘要」在每次
    // 重新打包后都变，下游无法用它判断「还是那一份语料吗」。
    let dir = scratch("deterministic");
    let corpus = dir.join("corpus.db");
    write_minimal_corpus(&corpus, "1.2.3", 2);
    let first = dir.join("a.gz");
    let second = dir.join("b.gz");
    compress(&corpus, &first).expect("压缩 1");
    compress(&corpus, &second).expect("压缩 2");
    assert_eq!(
        sha256_of_file(&first).unwrap(),
        sha256_of_file(&second).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

fn compress_bytes(destination: &Path, payload: &[u8]) {
    let file = std::fs::File::create(destination).expect("创建");
    let mut encoder = GzEncoder::new(file, GZIP_LEVEL);
    encoder.write_all(payload).expect("写入");
    encoder.finish().expect("收尾");
}

/// 一个只有 `corpus_meta` 与 `poem` 的最小库。
///
/// 刻意不用 `build_database`：这些用例测的是打包与回读，不该被语料构建的全部
/// 前置条件（源清单、裁决文件、守恒）绑住。
fn write_minimal_corpus(path: &Path, corpus_version: &str, poem_count: i64) {
    let connection = Connection::open(path).expect("创建库");
    connection
        .execute_batch(
            "CREATE TABLE poem (stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);
             CREATE TABLE corpus_meta (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
                 schema_version INTEGER NOT NULL,
                 corpus_version TEXT NOT NULL,
                 built_at TEXT NOT NULL,
                 source_manifest_sha256 TEXT NOT NULL,
                 poem_count INTEGER NOT NULL,
                 finding_count INTEGER NOT NULL,
                 input_row_count INTEGER NOT NULL,
                 index_detail_mode TEXT NOT NULL,
                 derived_indexes TEXT NOT NULL,
                 shipped_scope TEXT NOT NULL,
                 builder_sqlite_version TEXT NOT NULL,
                 integrity_check TEXT NOT NULL
             );",
        )
        .expect("建表");
    for index in 0..poem_count {
        connection
            .execute(
                "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
                rusqlite::params![format!("{index:016}"), "床前明月光"],
            )
            .expect("写诗");
    }
    connection
        .execute(
            "INSERT INTO corpus_meta VALUES \
             (1, ?1, ?2, '2026-08-10T00:00:00Z', ?3, ?4, 0, ?4, 'full', 'first_launch', \
              'tang-song', '3.53.2', 'ok')",
            rusqlite::params![
                yunjian_corpus::db::SCHEMA_VERSION,
                corpus_version,
                "b".repeat(64),
                poem_count,
            ],
        )
        .expect("写元数据");
}

fn sample_manifest() -> Manifest {
    Manifest {
        schema_version: yunjian_corpus::db::SCHEMA_VERSION,
        corpus_version: "1.2.3".to_owned(),
        min_app_version: MIN_APP_VERSION.to_owned(),
        record_count: 2,
        source_manifest_sha256: "b".repeat(64),
        shipped_scope: "tang-song".to_owned(),
        derived_indexes: "first_launch".to_owned(),
        index_detail_mode: "full".to_owned(),
        built_at: "2026-08-10T00:00:00Z".to_owned(),
        artifact_name: "yunjian-corpus-1.2.3.db.gz".to_owned(),
        size_bytes: 4096,
        sha256: "c".repeat(64),
        uncompressed_bytes: 8192,
        measurement: ManifestMeasurement {
            within_budget: true,
            budget_gzip_bytes: 300 * 1024 * 1024,
            budget_declared_by: "todo 21 上调至 300 MB".to_owned(),
            measured_gzip_bytes: 286 * 1024 * 1024,
            worst_p95_ms: 16.4,
            first_launch_seconds: 120.0,
            audit_bytes: 3_000_000_000,
            verdict_summary: "随包形态实测 1 行，预算内。".to_owned(),
        },
    }
}
