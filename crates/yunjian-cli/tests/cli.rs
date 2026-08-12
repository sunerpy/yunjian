//! `yunjian` 可执行文件的端到端契约。
//!
//! # 为什么必须用子进程
//!
//! 本文件断言的三件事在进程内一件都测不了：
//!
//! 1. **stdout 里没有一个字节的日志。** libtest 自己就往 stdout 打 `running N tests` 与统计行，
//!    任何进程内断言都在一条已经被污染的流上做判断。
//! 2. **退出码是 0/1/2/3。** 退出码只在进程真的 `exit()` 之后才存在。
//! 3. **`--json | jq` 不会被日志破坏。** 这条要的正是「两个流分开」这个进程级事实。
//!
//! # fixture 从哪里来
//!
//! 语料库在测试里现建。随包语料是 633 MiB、首启派生实测 571.8 s 的工件，任何门禁都不
//! 可能等它。诗取自 `yunjian-core` 的黄金查询契约 fixture（`tests/fixtures/poems.toml`）
//! ——那是仓库里唯一一份签入的、逐字校对过的公有领域测试语料，再抄一份只会多一个漂移源。

use assert_cmd::Command;
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 随包 schema 的路径。跨 crate 只读一个文件，不引入依赖。
const CORPUS_SCHEMA_PATH: &str = "../yunjian-corpus/schema.sql";

/// 黄金查询契约的 fixture 语料。
const SHARED_FIXTURES_PATH: &str = "../yunjian-core/tests/fixtures/poems.toml";

/// 详情类断言锚定的那首诗。
const ANCHOR: &str = "fixture:tang-libai-jingyesi";

/// 锚定作品的韵部（平水韵 七阳 平）。
const ANCHOR_GROUP: &str = "七阳";

/// 极小平水韵子集：(韵部, 声调, 字)。只需覆盖到锚定作品的韵脚。
const PINGSHUI_ROWS: &[(&str, &str, &str)] = &[
    ("七阳", "level", "光"),
    ("七阳", "level", "霜"),
    ("七阳", "level", "乡"),
    ("八庚", "level", "明"),
];

/// 日志行里必然出现、而结果里绝不该出现的记号。
///
/// 级别名之外还要查 target：`tracing` 的 fmt 层会打出 `yunjian_cli:` / `yunjian_core:`，
/// 而它们不可能是诗句或 JSON 字段名的一部分。
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

// ---------------------------------------------------------------- fixture 数据

#[derive(Debug, Deserialize)]
struct SharedFixtures {
    #[serde(rename = "poem")]
    poems: Vec<SharedPoem>,
}

#[derive(Debug, Deserialize)]
struct SharedPoem {
    stable_id: String,
    title: String,
    author: String,
    dynasty: String,
    ci_tune: String,
    body: String,
    first_line: String,
    last_chars: Vec<String>,
    tags: Vec<String>,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn shared_fixtures() -> SharedFixtures {
    let path = manifest_dir().join(SHARED_FIXTURES_PATH);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 fixture 失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 fixture 失败 {}：{error}", path.display()))
}

// ---------------------------------------------------------------- 沙箱

/// 一次测试的独立目录：语料库、配置与日志全在里面，析构时整棵删掉。
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
            "yunjian-cli-{}-{}",
            std::process::id(),
            NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建沙箱目录");
        let sandbox = Self { dir };
        write_corpus(&sandbox.corpus());
        sandbox.write_config();
        sandbox
    }

    fn corpus(&self) -> PathBuf {
        self.dir.join("corpus.db")
    }

    fn config(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    fn write_config(&self) {
        let contents = format!(
            "[app]\n\
             name = \"云笺\"\n\
             data_dir = {app_dir}\n\
             \n\
             [corpus]\n\
             path = {corpus}\n\
             data_dir = {corpus_dir}\n\
             \n\
             [logger]\n\
             level = \"info\"\n\
             json = false\n\
             dir = {logs}\n\
             file_prefix = \"yunjian\"\n",
            app_dir = quote(&self.dir.join("app")),
            corpus = quote(&self.corpus()),
            corpus_dir = quote(&self.dir.join("corpus")),
            logs = quote(&self.dir.join("logs")),
        );
        std::fs::write(self.config(), contents).expect("写沙箱配置");
    }

    /// 一条已隔离环境的命令。
    ///
    /// 三个环境变量必须显式清掉：`APP_CONFIG` 与 `YUNJIAN_CORPUS_PATH` 会压过配置发现，
    /// `RUST_LOG` 会压过 `--log-level`。宿主机上恰好设了任何一个，测试就会验错东西。
    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("yunjian").expect("定位 yunjian 可执行文件");
        command
            .env_remove("APP_CONFIG")
            .env_remove("YUNJIAN_CORPUS_PATH")
            .env_remove("RUST_LOG")
            .arg("--config")
            .arg(self.config());
        command
    }
}

/// 把路径写成合法的 TOML 字符串字面量。
///
/// **不能用 `format!("\"{}\"", path.display())`。** Windows 上临时目录形如
/// `C:\Users\RUNNER~1\...`，而 `\U` 在 TOML 的 basic string 里是非法转义，解析当场失败
/// ——CI 的 `Test (Windows)` 作业实测 18 个用例因此全红，报「解析 TOML 失败」。
/// 交给 `toml` 自己序列化，转义规则就与生产端（`yunjian_core::config` 的模板写出）一致。
fn quote(path: &Path) -> String {
    toml::Value::from(path.to_string_lossy().into_owned()).to_string()
}

// ---------------------------------------------------------------- 语料库 fixture

fn write_corpus(path: &Path) {
    let schema_path = manifest_dir().join(CORPUS_SCHEMA_PATH);
    let schema = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|error| panic!("读取随包 schema 失败 {}：{error}", schema_path.display()));
    let connection = Connection::open(path).expect("创建 fixture 语料库");
    connection.execute_batch(&schema).expect("套用随包 schema");

    let fixtures = shared_fixtures();
    for poem in &fixtures.poems {
        connection
            .execute(
                "INSERT OR IGNORE INTO author(name) VALUES (?1)",
                params![poem.author],
            )
            .expect("写作者");
        connection
            .execute(
                "INSERT INTO poem(stable_id, content_hash, source_locator, source_locator_kind, \
                 genre, title, title_raw, ci_tune, author, dynasty, dynasty_raw, body, \
                 body_original, script, first_line, last_chars, line_count, char_count, \
                 provenance_source, provenance_revision, provenance_kind, provenance_license, \
                 provenance_license_class, work_group, edition_group) \
                 VALUES (?1, ?2, ?3, 'native', 'shi', ?4, ?4, ?5, ?6, ?7, ?7, ?8, ?8, \
                 'simplified', ?9, ?10, ?11, ?12, 'chinese-poetry', 'rev-abc123', '原文', 'MIT', \
                 'permissive', ?13, ?14)",
                params![
                    poem.stable_id,
                    format!("hash-{}", poem.stable_id),
                    format!("locator:{}", poem.stable_id),
                    poem.title,
                    (!poem.ci_tune.is_empty()).then(|| poem.ci_tune.clone()),
                    poem.author,
                    poem.dynasty,
                    poem.body,
                    poem.first_line,
                    serde_json::to_string(&poem.last_chars).expect("序列化 last_chars"),
                    poem.last_chars.len() as i64,
                    poem.body.chars().count() as i64,
                    format!("wg-{}", poem.title),
                    format!("eg-{}-{}", poem.author, poem.title),
                ],
            )
            .expect("写诗");
    }

    let mut tags: Vec<&str> = fixtures
        .poems
        .iter()
        .flat_map(|poem| poem.tags.iter().map(String::as_str))
        .collect();
    tags.sort_unstable();
    tags.dedup();
    for name in &tags {
        connection
            .execute("INSERT INTO tag(name) VALUES (?1)", params![name])
            .expect("写标签");
    }
    for poem in &fixtures.poems {
        for tag in &poem.tags {
            connection
                .execute(
                    "INSERT INTO poem_tag(poem_id, tag) VALUES (?1, ?2)",
                    params![poem.stable_id, tag],
                )
                .expect("写标签关联");
        }
    }

    for (group, tone, character) in PINGSHUI_ROWS {
        connection
            .execute(
                "INSERT INTO rhyme(rhyme_book, rhyme_group, tone, tone_raw, character) \
                 VALUES ('pingshui', ?1, ?2, ?2, ?3)",
                params![group, tone, character],
            )
            .expect("写韵书行");
    }
    connection
        .execute(
            "INSERT INTO poem_rhyme_group(poem_id, rhyme_book, rhyme_group, tone, confidence) \
             VALUES (?1, 'pingshui', ?2, 'level', 'unambiguous')",
            params![ANCHOR, ANCHOR_GROUP],
        )
        .expect("写韵部归属");
    connection
        .execute(
            "INSERT INTO commentary(id, poem_id, text, citation_work, citation_author, \
             citation_dynasty, citation_dynasty_raw, citation_work_completed_by, \
             citation_source_note) VALUES ('fixture-commentary-001', ?1, \
             '「床前明月光」四句，妙絕古今，蓋以無意得之。', '唐诗别裁集', '沈德潜', '清', '清', \
             1717, '卷十九・五言絕句；据四部丛刊本，修订号 1234567')",
            params![ANCHOR],
        )
        .expect("写集评");

    // `source_manifest_sha256` 有 `length = 64` 的 CHECK，因此必须给一个真正 64 位的串。
    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, schema_version, corpus_version, built_at, \
             source_manifest_sha256, poem_count, finding_count, input_row_count, \
             index_detail_mode, derived_indexes, shipped_scope, builder_sqlite_version, \
             integrity_check) \
             VALUES (1, ?1, 'cli-fixture-v1', '2026-08-11T00:00:00Z', ?2, ?3, 0, ?3, \
             'full', 'first_launch', '10k', '3.51.0', 'ok')",
            params![
                yunjian_core::SCHEMA_VERSION,
                "0".repeat(64),
                fixtures.poems.len() as i64
            ],
        )
        .expect("写 corpus_meta");
    connection.close().expect("关闭 fixture 语料库");
}

// ---------------------------------------------------------------- 断言辅助

/// stdout 里不得出现任何日志痕迹。
fn assert_stdout_has_no_log_text(stdout: &str) {
    for marker in LOG_MARKERS {
        assert!(
            !stdout.contains(marker),
            "stdout 出现日志痕迹 `{marker}`：\n{stdout}"
        );
    }
}

/// stdout 必须恰好是一行合法 JSON，即 `jq -e .` 能成功的形态。
fn parse_single_json_line(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "`--json` 的 stdout 必须恰好一行，实际 {} 行：\n{stdout}",
        lines.len()
    );
    serde_json::from_str(lines[0])
        .unwrap_or_else(|error| panic!("stdout 不是合法 JSON（{error}）：\n{stdout}"))
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("输出必须是合法 UTF-8")
}

// ---------------------------------------------------------------- 用例

#[test]
fn a_two_character_query_returns_results_as_json_while_logs_stay_on_stderr() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月", "--json"])
        .output()
        .expect("运行 yunjian search");

    let stdout = utf8(&output.stdout);
    let stderr = utf8(&output.stderr);
    assert!(
        output.status.success(),
        "两字查询应当成功，退出码 {:?}\nstderr:\n{stderr}",
        output.status.code()
    );

    let value = parse_single_json_line(&stdout);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["command"], "search");
    assert_eq!(value["status"], "ok");
    let hits = value["data"]["hits"]
        .as_array()
        .expect("data.hits 必须是数组");
    assert!(!hits.is_empty(), "「明月」至少应命中一首：\n{stdout}");

    // 日志确实产生了（否则「日志没污染 stdout」这条断言是空的），且全在 stderr。
    assert!(
        LOG_MARKERS.iter().any(|marker| stderr.contains(marker)),
        "stderr 应当包含日志行：\n{stderr}"
    );
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn the_two_character_query_uses_the_ngram_candidate_path() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月", "--json"])
        .output()
        .expect("运行 yunjian search");
    let value = parse_single_json_line(&utf8(&output.stdout));
    // 首启派生在 fixture 规模上必然成功，因此两字查询应当走候选表而不是退化路径。
    assert_eq!(
        value["data"]["plan"],
        "Ngram",
        "两字查询应走 n-gram 候选表：\n{}",
        utf8(&output.stdout)
    );
    assert_eq!(value["warnings"], serde_json::json!([]));
}

#[test]
fn human_output_also_lands_on_stdout_without_any_log_text() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月"])
        .output()
        .expect("运行 yunjian search");
    let stdout = utf8(&output.stdout);
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    assert!(stdout.contains("明月"), "人类输出应当回显查询：\n{stdout}");
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn a_query_with_no_hits_exits_one_and_is_not_reported_as_an_error() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "zzzznotfound", "--json"])
        .output()
        .expect("运行 yunjian search");
    assert_eq!(
        output.status.code(),
        Some(1),
        "查不到结果应当退出 1\nstderr:\n{}",
        utf8(&output.stderr)
    );
    let value = parse_single_json_line(&utf8(&output.stdout));
    // 空结果不是错误：`status` 是 `empty`，且载荷仍在。
    assert_eq!(value["status"], "empty");
    assert!(value.get("error").is_none());
    assert_eq!(value["data"]["hits"], serde_json::json!([]));
}

#[test]
fn a_missing_corpus_exits_three_and_names_the_fetch_command() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["--corpus", "/nonexistent/yunjian-corpus.db", "search", "x"])
        .output()
        .expect("运行 yunjian search");
    assert_eq!(
        output.status.code(),
        Some(3),
        "语料不可用应当退出 3\nstderr:\n{}",
        utf8(&output.stderr)
    );
    let stderr = utf8(&output.stderr);
    assert!(
        stderr.contains("corpus fetch"),
        "失败文案必须点名 `yunjian corpus fetch`：\n{stderr}"
    );
    // 缺语料绝不能 panic：panic 会打印 `panicked at`，而且退出码是 101。
    assert!(
        !stderr.contains("panicked at"),
        "缺语料不得 panic：\n{stderr}"
    );
    assert!(utf8(&output.stdout).is_empty(), "失败时 stdout 应为空");
}

#[test]
fn a_missing_corpus_in_json_mode_puts_the_hint_in_the_envelope() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args([
            "--corpus",
            "/nonexistent/yunjian-corpus.db",
            "search",
            "x",
            "--json",
        ])
        .output()
        .expect("运行 yunjian search");
    assert_eq!(output.status.code(), Some(3));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "corpus_unavailable");
    assert!(
        value["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("corpus fetch")),
        "信封的 hint 必须点名取语料的命令：{value}"
    );
}

#[test]
fn trace_level_logging_cannot_pollute_the_json_pipe() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .env("RUST_LOG", "trace")
        .args(["--log-level", "trace", "search", "明月", "--json"])
        .output()
        .expect("运行 yunjian search");
    let stdout = utf8(&output.stdout);
    let stderr = utf8(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    // 这是本文件存在的理由：日志再啰嗦，stdout 仍然只有那一行 JSON。
    let value = parse_single_json_line(&stdout);
    assert!(!value["data"]["hits"].as_array().expect("hits").is_empty());
    assert_stdout_has_no_log_text(&stdout);
    assert!(
        stderr.lines().count() > 1,
        "trace 级别应当产生多行日志：\n{stderr}"
    );
}

#[test]
fn show_renders_a_poem_with_its_tones_rhymes_and_sourced_commentary() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["show", ANCHOR])
        .output()
        .expect("运行 yunjian show");
    let stdout = utf8(&output.stdout);
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    for expected in ["静夜思", "李白", "平仄", ANCHOR_GROUP, "唐诗别裁集"] {
        assert!(stdout.contains(expected), "缺少 `{expected}`：\n{stdout}");
    }
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn show_on_an_unknown_id_exits_one_because_it_is_a_miss_not_a_broken_corpus() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["show", "fixture:does-not-exist", "--json"])
        .output()
        .expect("运行 yunjian show");
    assert_eq!(
        output.status.code(),
        Some(1),
        "查不到作品是「无结果」而不是「语料坏了」\nstderr:\n{}",
        utf8(&output.stderr)
    );
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["status"], "empty");
    assert_eq!(value["data"]["found"], serde_json::json!(false));
}

#[test]
fn author_lists_the_works_and_reports_the_total() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["author", "李白", "--json"])
        .output()
        .expect("运行 yunjian author");
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["command"], "author");
    assert!(
        value["data"]["poem_count"].as_u64().is_some_and(|n| n > 0),
        "李白 应当有作品：{value}"
    );
}

#[test]
fn rhyme_search_requires_a_book_and_finds_the_anchor() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["rhyme", ANCHOR_GROUP, "--book", "pingshui", "--json"])
        .output()
        .expect("运行 yunjian rhyme");
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    let value = parse_single_json_line(&utf8(&output.stdout));
    let hits = value["data"]["hits"].as_array().expect("hits 必须是数组");
    assert!(!hits.is_empty(), "七阳 应当命中锚定作品：{value}");
}

#[test]
fn an_unshipped_rhyme_book_exits_two_rather_than_answering_no() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["rhyme", "第一部", "--book", "xinyun", "--json"])
        .output()
        .expect("运行 yunjian rhyme");
    // 2 而不是 3：`corpus fetch` 取不来一条许可判定。
    assert_eq!(
        output.status.code(),
        Some(2),
        "未随包的韵书是用法错误\nstderr:\n{}",
        utf8(&output.stderr)
    );
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["error"]["code"], "rhyme_book_unavailable");
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("未随包分发")),
        "文案必须说清是「没有这本书」而不是「查过了不押韵」：{value}"
    );
}

#[test]
fn corpus_status_reports_the_shape_of_the_corpus_it_actually_opened() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["corpus", "status", "--json"])
        .output()
        .expect("运行 yunjian corpus status");
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["command"], "corpus.status");
    assert_eq!(value["data"]["corpus_version"], "cli-fixture-v1");
    assert_eq!(value["data"]["derived_ready"], serde_json::json!(true));
    assert_eq!(
        value["data"]["schema_version"],
        serde_json::json!(yunjian_core::SCHEMA_VERSION)
    );
}

#[test]
fn corpus_status_on_an_empty_data_dir_exits_three_without_materializing() {
    let sandbox = Sandbox::new();
    // 指向一个不存在的语料文件：`corpus status` 必须只报告，不去落地。
    let output = sandbox
        .command()
        .args([
            "--corpus",
            &sandbox.dir.join("absent.db").display().to_string(),
            "corpus",
            "status",
        ])
        .output()
        .expect("运行 yunjian corpus status");
    assert_eq!(output.status.code(), Some(3));
    assert!(utf8(&output.stderr).contains("corpus fetch"));
    assert!(!sandbox.dir.join("absent.db").exists());
}

#[test]
fn corpus_fetch_opens_the_configured_corpus_and_reports_it_ready() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["corpus", "fetch", "--json"])
        .output()
        .expect("运行 yunjian corpus fetch");
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["command"], "corpus.fetch");
    assert_eq!(value["data"]["derived_ready"], serde_json::json!(true));
}

#[test]
fn the_mcp_subcommand_enters_stdio_mode_without_a_cli_envelope() {
    let sandbox = Sandbox::new();
    let help = sandbox
        .command()
        .args(["mcp", "--help"])
        .output()
        .expect("运行 yunjian mcp --help");
    assert!(help.status.success(), "{}", utf8(&help.stderr));
    assert!(
        utf8(&help.stdout).contains("MCP"),
        "`mcp --help` 应当说明它是什么：\n{}",
        utf8(&help.stdout)
    );

    let output = sandbox
        .command()
        .args(["mcp", "--json"])
        .output()
        .expect("运行 yunjian mcp");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "stdio 模式不得输出 CLI JSON 信封：{}",
        utf8(&output.stdout)
    );
    assert_stdout_has_no_log_text(&utf8(&output.stdout));
    assert!(
        utf8(&output.stderr).contains("initialize request"),
        "握手前 EOF 应被识别为 MCP 初始化失败：{}",
        utf8(&output.stderr)
    );
}

#[test]
fn an_unknown_flag_exits_two() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月", "--nonexistent-flag"])
        .output()
        .expect("运行 yunjian search");
    // clap 自己用 2 表示用法错误，这与本 CLI 的映射一致——不能让同一类失败有两个码。
    assert_eq!(output.status.code(), Some(2));
    assert!(utf8(&output.stdout).is_empty(), "用法错误时 stdout 应为空");
}

#[test]
fn a_zero_limit_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月", "--limit", "0", "--json"])
        .output()
        .expect("运行 yunjian search");
    assert_eq!(output.status.code(), Some(2));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["error"]["code"], "usage");
}

#[test]
fn the_author_filter_narrows_the_page_and_explains_an_emptied_page() {
    let sandbox = Sandbox::new();
    let kept = sandbox
        .command()
        .args(["search", "明月", "--author", "李白", "--json"])
        .output()
        .expect("运行 yunjian search");
    let value = parse_single_json_line(&utf8(&kept.stdout));
    assert_eq!(value["data"]["filters"]["author"], "李白");
    for hit in value["data"]["hits"].as_array().expect("hits") {
        assert_eq!(hit["author"], "李白", "过滤后不该出现别的作者：{value}");
    }

    let emptied = sandbox
        .command()
        .args(["search", "明月", "--author", "不存在的作者", "--json"])
        .output()
        .expect("运行 yunjian search");
    assert_eq!(emptied.status.code(), Some(1));
    let value = parse_single_json_line(&utf8(&emptied.stdout));
    // 被过滤清空必须说明白，否则用户读成「语料里没有明月」。
    assert_eq!(value["warnings"][0]["code"], "filtered_page_empty");
}

#[test]
fn the_rhyme_book_option_annotates_hits_without_filtering_them() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["search", "明月", "--rhyme-book", "pingshui", "--json"])
        .output()
        .expect("运行 yunjian search");
    assert!(output.status.success(), "{}", utf8(&output.stderr));
    let value = parse_single_json_line(&utf8(&output.stdout));
    let hits = value["data"]["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "标注不该减少命中：{value}");
    let annotated = hits
        .iter()
        .find(|hit| hit["poem_id"] == ANCHOR)
        .expect("锚定作品应在命中里");
    assert_eq!(annotated["rhyme_groups"][0]["group"], ANCHOR_GROUP);
}

#[test]
fn the_shipped_crates_have_exactly_one_stdout_exemption() {
    // 这条断言守的是整条 stdout 禁令：新开一个 `#[allow(clippy::print_stdout)]` 就等于新开
    // 一个能毁掉 MCP 协议流与 `--json | jq` 的出口，而 lint 本身管不了「豁免了几处」。
    //
    // 作用域是 `crates/*/src/`，即真正会被分发的代码。刻意不含两处：
    // `xtask`（`publish = false` 的开发工具，终端报告就是它的产品，它自己的豁免收在一个函数上），
    // 以及测试目录（libtest 本来就占着 stdout，测试里写 stdout 污染不了任何产物）。
    let mut exemptions = Vec::new();
    collect_exemptions(&manifest_dir().join("..").join(".."), &mut exemptions);
    assert_eq!(
        exemptions,
        vec![PathBuf::from("crates/yunjian-cli/src/present.rs")],
        "分发代码里的 stdout 豁免点只允许 `present.rs` 一处，实际：{exemptions:?}"
    );
}

fn collect_exemptions(root: &Path, found: &mut Vec<PathBuf>) {
    let crates = root.join("crates");
    let Ok(entries) = std::fs::read_dir(&crates) else {
        panic!("读取 {} 失败", crates.display());
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        walk_sources(&entry.path().join("src"), root, found);
    }
    found.sort();
}

fn walk_sources(dir: &Path, root: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk_sources(&path, root, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.contains("allow(clippy::print_stdout") {
                found.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
}

// ---------------------------------------------------------------- models 子命令

/// 模型缓存指向沙箱内的空目录。
///
/// **必须显式设置**：不设时 `models` 会落到仓库内 `models/cache`，于是测试结果取决于
/// 开发机上有没有下载过权重——那正是「随机绿」的来源。
fn models_command(sandbox: &Sandbox, args: &[&str]) -> std::process::Output {
    let cache = sandbox.dir.join("model-cache");
    std::fs::create_dir_all(&cache).expect("建模型缓存目录");
    sandbox
        .command()
        .env("YUNJIAN_MODEL_DIR", &cache)
        .args(args)
        .output()
        .expect("运行 yunjian models")
}

#[test]
fn models_list_reports_every_manifest_entry_with_its_license_and_never_opens_the_corpus() {
    let sandbox = Sandbox::new();
    // 语料库删掉：`models` 与诗库无关，删了它仍必须成功。这条同时证明 `models` 不会
    // 触发首启落地那个十分钟级的副作用。
    std::fs::remove_file(sandbox.corpus()).expect("删掉 fixture 语料库");

    let output = models_command(&sandbox, &["models", "list", "--json"]);
    let stdout = utf8(&output.stdout);
    let stderr = utf8(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "list 应当退出 0（即使没有语料库）\nstderr:\n{stderr}"
    );

    let value = parse_single_json_line(&stdout);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["command"], "models.list");
    assert_eq!(value["status"], "ok");
    let models = value["data"]["models"]
        .as_array()
        .expect("data.models 必须是数组");
    assert!(!models.is_empty(), "清单不能是空的：\n{stdout}");
    for model in models {
        let license = model["license"].as_str().expect("license 必须是字符串");
        assert!(
            license == "MIT" || license == "Apache-2.0",
            "{} 的许可 {license} 不在允许列表",
            model["name"]
        );
        assert_eq!(model["unpacked"], false, "空缓存里不该有已就位的模型");
        assert!(
            model["refused"].is_null(),
            "清单里不该有被拒的条目：{model}"
        );
    }
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn models_list_in_human_mode_names_the_attribution_directory() {
    let sandbox = Sandbox::new();
    let output = models_command(&sandbox, &["models", "list"]);
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout.contains("licenses/"),
        "人类输出要指向许可原文所在目录：\n{stdout}"
    );
    assert!(
        stdout.contains("未下载"),
        "空缓存下每一行都该标未下载：\n{stdout}"
    );
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn models_fetch_with_an_unknown_name_exits_two_and_lists_the_real_names() {
    let sandbox = Sandbox::new();
    let output = models_command(&sandbox, &["models", "fetch", "no-such-model", "--json"]);
    let stdout = utf8(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(2),
        "名字打错是用法错误，不是数据不可用\nstdout:\n{stdout}"
    );
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], "models.fetch");
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "usage");
    assert!(
        value["error"]["message"]
            .as_str()
            .expect("message 必须是字符串")
            .contains("sherpa-onnx-whisper-tiny"),
        "报错要列出实际可用的名字：{value}"
    );
    assert!(
        !value["error"]["hint"]
            .as_str()
            .expect("hint 必须存在")
            .contains("corpus fetch"),
        "模型的问题不该建议去取语料：{value}"
    );
}

/// 模型侧的问题必须指向 `models fetch`，**绝不建议去取语料**。
///
/// # 为什么这条不再断言退出码 3
///
/// 它原先请求一个真实条目并假设「本机无外网所以必然失败」。那个假设是错的——CDN
/// 有时可达，我们真的撞见过一次它下载成功、退出 0，于是这条测试随机绿。
/// 而清单是 `include_str!` 内嵌的，没有环境变量能注入一个指向不可达地址的假清单，
/// 所以「清单里有、本地缺、且取不到」这个组合无法在单测里确定性地造出来。
///
/// 退出码的语义本身另有两条测试守着（未知名字 → 2 用法错误；语料缺失 → 3 数据不可用），
/// 这条于是只守它唯一能确定性验证的那件事：**模型侧的诊断不得把用户引向语料命令**。
/// 无论 fetch 成功还是失败，这条不变量都必须成立。
#[test]
fn a_model_problem_never_points_at_corpus_fetch() {
    let sandbox = Sandbox::new();
    let output = models_command(
        &sandbox,
        &[
            "models",
            "fetch",
            "this-model-does-not-exist-in-manifest",
            "--json",
        ],
    );
    let stdout = utf8(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout 必须是 JSON：{error}\n{stdout}"));
    assert_ne!(
        output.status.code(),
        Some(0),
        "清单里没有这个名字，不该成功\nstdout:\n{stdout}"
    );
    let rendered = value.to_string();
    assert!(
        !rendered.contains("corpus fetch"),
        "模型侧的诊断不得把用户引向语料命令：{rendered}"
    );
}

#[test]
fn models_verify_on_an_empty_cache_succeeds_because_nothing_local_is_not_a_failure() {
    let sandbox = Sandbox::new();
    let output = models_command(&sandbox, &["models", "verify", "--json"]);
    let stdout = utf8(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "本地没有归档时跳过而不算失败\nstdout:\n{stdout}"
    );
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], "models.verify");
    for model in value["data"]["models"]
        .as_array()
        .expect("data.models 必须是数组")
    {
        assert!(
            model["verified_sha256"].is_null(),
            "本地无归档时不该声称核对过摘要：{model}"
        );
    }
}

/// 归档摘要不符时必须变红，而不是「核对了，通过」。
#[test]
fn models_verify_rejects_a_local_archive_whose_digest_does_not_match() {
    let sandbox = Sandbox::new();
    let cache = sandbox.dir.join("model-cache");
    let archives = cache.join("archives");
    std::fs::create_dir_all(&archives).expect("建归档目录");
    std::fs::write(
        archives.join("kitten-nano-en-v0_2-fp16.tar.bz2"),
        "这不是真的归档".as_bytes(),
    )
    .expect("写一个假归档");

    let output = models_command(
        &sandbox,
        &["models", "verify", "kitten-nano-en-v0_2-fp16", "--json"],
    );
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(3), "假归档必须失败\n{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["error"]["code"], "model_unavailable");
    assert!(
        value["error"]["message"]
            .as_str()
            .expect("message 必须是字符串")
            .contains("字节"),
        "字节数不符要先报出来：{value}"
    );
}

#[test]
fn models_remove_on_an_empty_cache_exits_one_rather_than_claiming_it_deleted_something() {
    let sandbox = Sandbox::new();
    let output = models_command(
        &sandbox,
        &["models", "remove", "kitten-nano-en-v0_2-fp16", "--json"],
    );
    let stdout = utf8(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "什么都没删是「无结果」，不是成功也不是失败\n{stdout}"
    );
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], "models.remove");
    assert_eq!(value["status"], "empty");
    assert_eq!(value["data"]["removed_dir"], false);
    assert_eq!(value["data"]["removed_archive"], false);
}

#[test]
fn models_remove_deletes_a_present_cache_and_reports_what_went() {
    let sandbox = Sandbox::new();
    let cache = sandbox.dir.join("model-cache");
    let dir = cache.join("kitten-nano-en-v0_2-fp16");
    std::fs::create_dir_all(&dir).expect("建模型目录");
    std::fs::write(dir.join("tokens.txt"), b"0 <blk>\n").expect("写文件");

    let output = models_command(
        &sandbox,
        &["models", "remove", "kitten-nano-en-v0_2-fp16", "--json"],
    );
    let stdout = utf8(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "删掉了东西应当退出 0\n{stdout}"
    );
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["data"]["removed_dir"], true);
    assert!(!dir.exists(), "目录必须真的没了");
}

/// 已就位的模型必须直接返回，不发起任何网络请求。
///
/// 本机无外网，所以「命令在毫秒级返回退出 0」本身就是证据：真去下载 25 MiB 会超时失败。
#[test]
fn models_fetch_on_a_present_model_returns_immediately_without_downloading() {
    let sandbox = Sandbox::new();
    let cache = sandbox.dir.join("model-cache");
    let dir = cache.join("kitten-nano-en-v0_2-fp16");
    std::fs::create_dir_all(&dir).expect("建模型目录");
    std::fs::write(dir.join("tokens.txt"), b"0 <blk>\n").expect("写文件");

    let started = std::time::Instant::now();
    let output = models_command(
        &sandbox,
        &["models", "fetch", "kitten-nano-en-v0_2-fp16", "--json"],
    );
    let elapsed = started.elapsed();
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "已就位必须成功\n{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], "models.fetch");
    assert_eq!(value["data"]["license"], "Apache-2.0");
    assert_eq!(
        value["data"]["attribution"], "kitten-nano-en-v0_2-fp16.LICENSE",
        "必须指向 licenses/ 下真实存在的那份原文"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "已就位时不该有网络往返，实测 {elapsed:?}"
    );
}
