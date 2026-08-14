//! `yunjian recite` 的端到端契约。
//!
//! # 为什么与 `tests/cli.rs` 分开
//!
//! 验收要求 `cargo test -p yunjian-cli --test recite` 单独可跑，而 `cli.rs` 是一份已经
//! 稳定的 1055 行契约文件。背诵用例另起一个 target，既满足验收，也不必去动那份文件。
//!
//! 语料 fixture 因此在本文件里另建了一份**最小**的：`recite` 只需要作品本体（题目、作者、
//! 朝代、正文）与 `corpus_meta`，用不到 `cli.rs` 那份里的标签、韵书与集评。诗句本身仍取自
//! `yunjian-core` 的黄金查询 fixture，所以「诗文长什么样」在仓库里依然只有一个出处。
//!
//! # 这份文件守的是什么
//!
//! 一句话：**命令行是薄壳。** 分数、对齐操作与 FSRS 等级全部由 `yunjian-recite` 算出，
//! 命令行只负责取诗、读 stdin、渲染与记账。`the_cli_carries_no_scoring_logic_of_its_own`
//! 是这条边界的可执行判据；桌面端（todo 63）将是同一层内核之上的第二个薄壳。

use assert_cmd::Command;
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 随包 schema 的路径。
const CORPUS_SCHEMA_PATH: &str = "../yunjian-corpus/schema.sql";

/// 黄金查询契约的 fixture 语料。
const SHARED_FIXTURES_PATH: &str = "../yunjian-core/tests/fixtures/poems.toml";

/// 用来背的那首诗。
const ANCHOR: &str = "fixture:tang-libai-jingyesi";

const ANCHOR_CHUNK: &str = "fixture:tang-libai-jingyesi:v1:0-4";

/// 复习库文件名，与 `output::RECITE_DATABASE_FILE` 一致。
const REVIEW_DATABASE: &str = "recite.db";

/// 日志行里必然出现、而结果里绝不该出现的记号。
const LOG_MARKERS: &[&str] = &["INFO", "WARN", "ERROR", "DEBUG", "TRACE", "yunjian_cli:"];

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
    body: String,
    first_line: String,
    last_chars: Vec<String>,
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

/// 锚定作品的正文原文，带标点。
fn anchor_body() -> String {
    shared_fixtures()
        .poems
        .into_iter()
        .find(|poem| poem.stable_id == ANCHOR)
        .map(|poem| poem.body)
        .unwrap_or_else(|| panic!("fixture 里必须有 {ANCHOR}"))
}

// ---------------------------------------------------------------- 沙箱

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
            "yunjian-recite-{}-{}",
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

    fn app_dir(&self) -> PathBuf {
        self.dir.join("app")
    }

    fn review_database(&self) -> PathBuf {
        self.app_dir().join(REVIEW_DATABASE)
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
            app_dir = quote(&self.app_dir()),
            corpus = quote(&self.corpus()),
            corpus_dir = quote(&self.dir.join("corpus")),
            logs = quote(&self.dir.join("logs")),
        );
        std::fs::write(self.config(), contents).expect("写沙箱配置");
    }

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

    /// 跑一轮练习，作答由 stdin 送入。
    fn recite(&self, answer: &str, extra: &[&str]) -> std::process::Output {
        self.command()
            .arg("recite")
            .arg(ANCHOR)
            .args(extra)
            .write_stdin(answer.to_owned())
            .output()
            .expect("执行 recite")
    }
}

/// 把路径写成合法的 TOML 字符串字面量。Windows 的 `\U` 在 basic string 里是非法转义。
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
                 VALUES (?1, ?2, ?3, 'native', 'shi', ?4, ?4, NULL, ?5, ?6, ?6, ?7, ?7, \
                 'simplified', ?8, ?9, ?10, ?11, 'chinese-poetry', 'rev-abc123', '原文', 'MIT', \
                 'permissive', ?12, ?13)",
                params![
                    poem.stable_id,
                    format!("hash-{}", poem.stable_id),
                    format!("locator:{}", poem.stable_id),
                    poem.title,
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

    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, schema_version, corpus_version, built_at, \
             source_manifest_sha256, poem_count, finding_count, input_row_count, \
             index_detail_mode, derived_indexes, shipped_scope, builder_sqlite_version, \
             integrity_check) \
             VALUES (1, ?1, 'recite-fixture-v1', '2026-08-12T00:00:00Z', ?2, ?3, 0, ?3, \
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

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("输出必须是合法 UTF-8")
}

fn assert_stdout_has_no_log_text(stdout: &str) {
    for marker in LOG_MARKERS {
        assert!(
            !stdout.contains(marker),
            "stdout 出现日志痕迹 `{marker}`：\n{stdout}"
        );
    }
}

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

fn today() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .expect("系统时钟应晚于 Unix 纪元")
}

// ---------------------------------------------------------------- 打字练习

#[test]
fn a_perfect_typed_answer_prints_a_full_score_and_exits_zero() {
    let sandbox = Sandbox::new();
    let body = anchor_body();
    let output = sandbox.recite(&body, &["--seed", "42"]);
    let stdout = utf8(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(0),
        "完美作答必须退出 0\nstdout:\n{stdout}\nstderr:\n{}",
        utf8(&output.stderr)
    );
    assert_stdout_has_no_log_text(&stdout);
    assert!(
        stdout.contains("完整度 1.000"),
        "满分必须逐项写清而不是只给一个总分：\n{stdout}"
    );
    assert!(stdout.contains("严格字准 1.000"), "{stdout}");
    assert!(stdout.contains("宽容字准 1.000"), "{stdout}");
    assert!(
        stdout.contains("全篇相符，没有差异。"),
        "没有差异时必须明说，而不是留一片空白：\n{stdout}"
    );
    assert!(
        stdout.contains("不评估发音标准度"),
        "诚实边界必须出现在每次结果里：\n{stdout}"
    );
    // 首次完美作答按 todo 51 的严格优先级评 Easy。等级由内核判定，这里只核对它被如实转达。
    assert!(stdout.contains("评级 easy"), "{stdout}");
}

#[test]
fn a_wrong_answer_marks_every_character_and_lists_the_differences() {
    let sandbox = Sandbox::new();
    // 末字「乡」写成同音的「香」，并漏掉「霜」：一次覆盖替换与漏读两类。
    let body = anchor_body().replace('乡', "香").replace('霜', "");
    let output = sandbox.recite(&body, &["--seed", "42"]);
    let stdout = utf8(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(0),
        "答错也是一次完成的练习\n{stdout}"
    );
    assert!(
        stdout.contains("逐字：") && stdout.contains('✓'),
        "必须给出逐字标记：\n{stdout}"
    );
    assert!(
        stdout.contains("漏读：应读「霜」"),
        "漏读要点名是哪个字：\n{stdout}"
    );
    assert!(
        stdout.contains("应读「乡」，实读「香」"),
        "替换要同时给出应读与实读：\n{stdout}"
    );
    assert!(
        stdout.contains("近音替换"),
        "乡与香是近音，分类由内核给出，命令行须如实呈现：\n{stdout}"
    );
    assert!(
        !stdout.contains("完整度 1.000"),
        "漏了字不该报满完整度：\n{stdout}"
    );
}

#[test]
fn the_same_seed_reproduces_the_same_cloze_blanks() {
    let sandbox = Sandbox::new();
    let body = anchor_body();
    let blanks = |seed: &str| -> (Value, Value) {
        let output = sandbox.recite(&body, &["--json", "--seed", seed, "--ratio", "0.3"]);
        assert_eq!(output.status.code(), Some(0));
        let value = parse_single_json_line(&utf8(&output.stdout));
        (
            value["data"]["hidden_indices"].clone(),
            value["data"]["prompt"].clone(),
        )
    };

    let first = blanks("42");
    let second = blanks("42");
    assert_eq!(
        first, second,
        "同一个 --seed 必须挖在同样的位置，否则一局练习无法复现"
    );
    assert!(
        first.0.as_array().is_some_and(|blanks| !blanks.is_empty()),
        "比例 0.3 必须真的挖掉字：{:?}",
        first.0
    );

    // 种子必须真的起作用：只断言「同种子相同」时，一个把种子整个忽略的实现也能通过。
    let differs = (1..=12)
        .map(|seed| blanks(&seed.to_string()))
        .any(|other| other.0 != first.0);
    assert!(differs, "换种子必须能换出不同的挖空位置");
}

#[test]
fn the_seed_is_echoed_so_a_session_can_be_replayed_without_having_passed_one() {
    let sandbox = Sandbox::new();
    let output = sandbox.recite(&anchor_body(), &["--json"]);
    assert_eq!(output.status.code(), Some(0));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert!(
        value["data"]["seed"].as_u64().is_some(),
        "没给 --seed 时也必须回显本次用的种子，否则这一局永远复现不了：{value}"
    );
}

#[test]
fn first_char_and_masked_modes_run_without_any_cloze_parameter() {
    let sandbox = Sandbox::new();
    let body = anchor_body();
    for (mode, extra) in [
        ("first-char", Vec::new()),
        ("masked", vec!["--masked-lines", "2"]),
    ] {
        let mut args = vec!["--json", "--mode", mode];
        args.extend(extra);
        let output = sandbox.recite(&body, &args);
        let stdout = utf8(&output.stdout);
        assert_eq!(output.status.code(), Some(0), "{mode} 应能完成\n{stdout}");
        let value = parse_single_json_line(&stdout);
        assert_eq!(value["data"]["mode"], serde_json::json!(mode));
        // 挖空专属字段不该出现在别的形态里：填一个无意义的值会让调用方以为它生效了。
        assert!(value["data"].get("ratio").is_none(), "{value}");
        assert!(value["data"].get("seed").is_none(), "{value}");
    }
}

// ---------------------------------------------------------------- JSON 信封

#[test]
fn the_json_envelope_keeps_the_documented_shape_and_carries_the_full_score() {
    let sandbox = Sandbox::new();
    let output = sandbox.recite(&anchor_body(), &["--json", "--seed", "7"]);
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert_stdout_has_no_log_text(&stdout);

    let value = parse_single_json_line(&stdout);
    let envelope = value.as_object().expect("信封是 JSON 对象");
    for key in ["schema_version", "command", "status", "warnings", "data"] {
        assert!(envelope.contains_key(key), "缺少信封字段 {key}：{value}");
    }
    assert_eq!(envelope["schema_version"], serde_json::json!(1));
    assert_eq!(envelope["command"], serde_json::json!("recite"));
    assert_eq!(envelope["status"], serde_json::json!("ok"));
    assert_eq!(envelope["warnings"], serde_json::json!([]));
    assert!(!envelope.contains_key("error"), "{value}");

    let data = &value["data"];
    for key in [
        "poem_id",
        "title",
        "author",
        "dynasty",
        "mode",
        "prompt",
        "hidden_indices",
        "reference",
        "answer",
        "score",
        "ops",
        "grade",
        "grade_source",
        "first_attempt",
        "database",
        "review",
    ] {
        assert!(data.get(key).is_some(), "载荷缺少字段 {key}：{value}");
    }
    // 分数必须是完整的 `TypedScore`，不是挑几项报出来。
    for key in [
        "completeness",
        "accuracy_strict",
        "accuracy_lenient",
        "fluency",
        "is_rejected",
        "ops_summary",
    ] {
        assert!(
            data["score"].get(key).is_some(),
            "score 缺少字段 {key}：{value}"
        );
    }
    for key in [
        "normal_count",
        "deletion_count",
        "insertion_count",
        "rerecitation_count",
        "substitution_count",
    ] {
        assert!(
            data["score"]["ops_summary"].get(key).is_some(),
            "ops_summary 缺少字段 {key}：{value}"
        );
    }
    let ops = data["ops"].as_array().expect("ops 是数组");
    assert!(!ops.is_empty(), "完美作答也该给出逐项相符的 op：{value}");
    for op in ops {
        let kind = op["kind"].as_str().expect("每项 op 都带 kind");
        assert!(
            matches!(
                kind,
                "normal" | "deletion" | "insertion" | "re_recitation" | "substitution"
            ),
            "未预期的 op 类别 `{kind}`：{value}"
        );
    }
    for key in ["poem_id", "due_day", "scheduled_days", "last_grade"] {
        assert!(
            data["review"].get(key).is_some(),
            "review 缺少字段 {key}：{value}"
        );
    }
}

// ---------------------------------------------------------------- 语音退化

#[test]
fn voice_mode_falls_back_to_typing_and_still_exits_zero() {
    let sandbox = Sandbox::new();
    let output = sandbox.recite(&anchor_body(), &["--mode", "voice", "--seed", "3"]);
    let stdout = utf8(&output.stdout);

    // 这条是整个 todo 的硬约束：语音不可用绝不能把一次能做完的练习变成失败。
    assert_eq!(
        output.status.code(),
        Some(0),
        "`--mode voice` 不可用时必须退出 0\nstdout:\n{stdout}\nstderr:\n{}",
        utf8(&output.stderr)
    );
    assert!(stdout.contains("退化："), "必须打印退化提示：\n{stdout}");
    assert!(
        stdout.contains("已退化为挖空打字练习"),
        "提示要说清退到了哪种形态：\n{stdout}"
    );
    // 真的跑了一轮打字练习，而不是打完提示就退出。
    assert!(stdout.contains("完整度 1.000"), "{stdout}");
    assert!(stdout.contains("评级 easy"), "{stdout}");
    assert!(
        stdout.contains("下次复习："),
        "退化后的这一轮同样要计入排程：\n{stdout}"
    );
}

#[test]
fn the_voice_fallback_is_a_warning_in_the_envelope_not_an_error() {
    let sandbox = Sandbox::new();
    let output = sandbox.recite(
        &anchor_body(),
        &["--json", "--mode", "voice", "--seed", "3"],
    );
    assert_eq!(output.status.code(), Some(0));
    let value = parse_single_json_line(&utf8(&output.stdout));

    assert_eq!(value["status"], serde_json::json!("ok"));
    assert!(value.get("error").is_none(), "退化不是错误：{value}");
    assert_eq!(
        value["warnings"][0]["code"],
        serde_json::json!("voice_fallback"),
        "{value}"
    );
    assert_eq!(value["data"]["requested_mode"], serde_json::json!("voice"));
    assert_eq!(value["data"]["mode"], serde_json::json!("cloze"));
    assert!(
        value["data"]["fallback_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("已退化为挖空打字练习")),
        "{value}"
    );
    assert_eq!(
        value["data"]["score"]["completeness"],
        serde_json::json!(1.0),
        "退化后必须是一次真实的打字评分：{value}"
    );
}

// ---------------------------------------------------------------- 排程

#[test]
fn recite_due_lists_the_schedule_after_a_review() {
    let sandbox = Sandbox::new();
    let practised = sandbox.recite(&anchor_body(), &["--seed", "42"]);
    assert_eq!(
        practised.status.code(),
        Some(0),
        "{}",
        utf8(&practised.stdout)
    );

    let output = sandbox
        .command()
        .args(["recite", "due", "--all", "--json"])
        .output()
        .expect("执行 recite due --all");
    let stdout = utf8(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "已有排程必须退出 0\n{stdout}"
    );
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], serde_json::json!("recite.due"));
    assert_eq!(value["data"]["scope"], serde_json::json!("all"));
    assert_eq!(
        value["data"]["items"][0]["poem_id"],
        serde_json::json!(ANCHOR_CHUNK)
    );
    assert_eq!(
        value["data"]["items"][0]["last_grade"],
        serde_json::json!("easy")
    );
}

#[test]
fn recite_due_without_all_lists_only_what_is_due_today() {
    let sandbox = Sandbox::new();
    // 刚练完的一首下次到期必在将来（内核的间隔下限是 1 天），所以「今天到期」应当是空的。
    let practised = sandbox.recite(&anchor_body(), &["--seed", "42"]);
    assert_eq!(practised.status.code(), Some(0));
    let fresh = sandbox
        .command()
        .args(["recite", "due"])
        .output()
        .expect("执行 recite due");
    assert_eq!(
        fresh.status.code(),
        Some(1),
        "刚练完就说「今天还要复习」是错的\n{}",
        utf8(&fresh.stdout)
    );
    assert!(
        utf8(&fresh.stdout).contains("没有到期项"),
        "空队列要给出下一步：\n{}",
        utf8(&fresh.stdout)
    );

    // 造一条日期在过去的复习记录，让它今天真的到期。
    {
        let mut scheduler =
            yunjian_recite::Scheduler::open(sandbox.review_database()).expect("打开沙箱复习库");
        scheduler
            .review_at(
                "fixture:overdue",
                yunjian_recite::FsrsGrade::Again,
                today() - 30,
            )
            .expect("写一条已过期的复习记录");
    }
    let due = sandbox
        .command()
        .args(["recite", "due", "--json"])
        .output()
        .expect("执行 recite due");
    let stdout = utf8(&due.stdout);
    assert_eq!(due.status.code(), Some(0), "有到期项必须退出 0\n{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["data"]["scope"], serde_json::json!("due_today"));
    let items = value["data"]["items"].as_array().expect("items 是数组");
    assert!(
        items
            .iter()
            .any(|item| item["poem_id"] == serde_json::json!("fixture:overdue")),
        "已过期的项必须出现在今天的队列里：{value}"
    );
}

#[test]
fn recite_due_on_an_empty_schedule_exits_one_rather_than_failing() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["recite", "due"])
        .output()
        .expect("执行 recite due");
    let stdout = utf8(&output.stdout);
    // 空队列是「查过了，没有」而不是「查不了」：判成 3 会让脚本以为复习库坏了。
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert_stdout_has_no_log_text(&stdout);
}

#[test]
fn recite_stats_reports_the_distribution_and_the_thresholds_in_effect() {
    let sandbox = Sandbox::new();
    let practised = sandbox.recite(&anchor_body(), &["--seed", "42"]);
    assert_eq!(practised.status.code(), Some(0));

    let output = sandbox
        .command()
        .args(["recite", "stats", "--json"])
        .output()
        .expect("执行 recite stats");
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["command"], serde_json::json!("recite.stats"));
    assert_eq!(value["data"]["scheduled_total"], serde_json::json!(1));
    assert_eq!(value["data"]["by_last_grade"]["easy"], serde_json::json!(1));
    assert_eq!(
        value["data"]["daily_plan"]["task_count"],
        serde_json::json!(0)
    );
    assert_eq!(value["data"]["backlog"]["count"], serde_json::json!(0));
    assert!(
        value["data"]["next_seven_days"].is_array(),
        "未来七日压力必须稳定输出数组：{value}"
    );
    assert_eq!(
        value["data"]["observed_retention"]["sample_size"],
        serde_json::json!(0),
        "首次建立联片不算到期正式复习样本：{value}"
    );
    assert_eq!(value["data"]["retention_target"], serde_json::json!(0.85));
    // 阈值原样来自 `[recite.grading]`；命令行不得在这里自己算等级。
    for key in [
        "again_completeness_below",
        "hard_accuracy_lenient_below",
        "hard_rerecitation_above",
        "easy_accuracy_strict_at_least",
    ] {
        assert!(
            value["data"]["grading"].get(key).is_some(),
            "grading 缺少字段 {key}：{value}"
        );
    }

    let human = sandbox
        .command()
        .args(["recite", "stats"])
        .output()
        .expect("执行 recite stats");
    let rendered = utf8(&human.stdout);
    assert!(
        rendered.contains("等级由背诵内核按严格优先级判定"),
        "必须说清等级不是这条命令算的：\n{rendered}"
    );
}

#[test]
fn a_user_chosen_grade_overrides_the_typed_mapping() {
    let sandbox = Sandbox::new();
    // 语音路径按 2026-08-11 的裁决不做自动评级，等级由用户自选；退化后这条入口须同样可用。
    let output = sandbox.recite(
        &anchor_body(),
        &["--json", "--seed", "42", "--grade", "hard"],
    );
    assert_eq!(output.status.code(), Some(0));
    let value = parse_single_json_line(&utf8(&output.stdout));
    assert_eq!(value["data"]["grade"], serde_json::json!("hard"));
    assert_eq!(
        value["data"]["grade_source"],
        serde_json::json!("user_chosen")
    );
}

// ---------------------------------------------------------------- 用法错误

#[test]
fn an_unknown_poem_id_exits_two_and_sends_the_caller_to_search() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["recite", "fixture:no-such-poem", "--json"])
        .write_stdin("床前明月光")
        .output()
        .expect("执行 recite");
    let stdout = utf8(&output.stdout);
    // 名字不成立是用法错误（2），不是「语料坏了」（3），也不是一条空结果（1）。
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["error"]["code"], serde_json::json!("usage"));
    assert!(
        value["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("yunjian search")),
        "要指出怎么查到正确的 stable_id：{value}"
    );
    assert!(
        !value["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("corpus fetch")),
        "名字打错与语料缺失无关，不该建议去取语料：{value}"
    );
}

#[test]
fn an_out_of_range_ratio_exits_two_with_a_readable_message() {
    let sandbox = Sandbox::new();
    // 负数写成 `--ratio=-0.2`：分开写时 clap 先把 `-0.2` 当成短选项，值解析器根本收不到它
    // （那一路同样退出 2，由下面的 `every_rejected_ratio_shape` 钉住）。
    for ratio in ["--ratio=5", "--ratio=0", "--ratio=-0.2", "--ratio=abc"] {
        let output = sandbox
            .command()
            .args(["recite", ANCHOR, ratio])
            .write_stdin("床前明月光")
            .output()
            .expect("执行 recite");
        let stderr = utf8(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{ratio} 必须是用法错误\n{stderr}"
        );
        assert!(
            stderr.contains("挖空比例"),
            "{ratio} 的报错要说清哪个参数不对：\n{stderr}"
        );
    }
}

#[test]
fn every_rejected_ratio_shape_exits_two_however_it_was_written() {
    let sandbox = Sandbox::new();
    for arguments in [
        vec!["--ratio", "5"],
        vec!["--ratio", "-0.2"],
        vec!["--ratio"],
    ] {
        let mut args = vec!["recite", ANCHOR];
        args.extend(arguments.iter().copied());
        let output = sandbox
            .command()
            .args(&args)
            .write_stdin("床前明月光")
            .output()
            .expect("执行 recite");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} 必须退出 2\n{}",
            utf8(&output.stderr)
        );
    }
}

#[test]
fn an_unknown_mode_exits_two_and_lists_the_real_modes() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["recite", ANCHOR, "--mode", "singing"])
        .write_stdin("床前明月光")
        .output()
        .expect("执行 recite");
    let stderr = utf8(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("cloze"), "报错要列出真实取值：\n{stderr}");
    assert!(stderr.contains("voice"), "{stderr}");
}

#[test]
fn an_empty_answer_exits_two_without_recording_a_review() {
    let sandbox = Sandbox::new();
    let output = sandbox.recite("   \n", &["--json"]);
    let stdout = utf8(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(value["error"]["code"], serde_json::json!("usage"));
    assert!(
        value["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("stdin")),
        "要告诉用户作答从哪里来：{value}"
    );
    // 空作答按零分记账会往复习历史里写一条用户没做过的 Again，事后无法撤回。
    assert!(
        !sandbox.review_database().exists()
            || yunjian_recite::Scheduler::open(sandbox.review_database())
                .expect("打开复习库")
                .due_on(i64::MAX)
                .expect("读排程")
                .is_empty(),
        "空作答不得留下任何复习记录"
    );
}

#[test]
fn a_missing_corpus_exits_three_and_names_the_fetch_command() {
    let sandbox = Sandbox::new();
    std::fs::remove_file(sandbox.corpus()).expect("删掉 fixture 语料库");
    let output = sandbox.recite(&anchor_body(), &["--json"]);
    let stdout = utf8(&output.stdout);
    // 语料缺失是 3；与「名字打错」的 2 必须分开，否则脚本无法判断该改命令还是补语料。
    assert_eq!(output.status.code(), Some(3), "{stdout}");
    let value = parse_single_json_line(&stdout);
    assert_eq!(
        value["error"]["code"],
        serde_json::json!("corpus_unavailable")
    );
    assert!(
        value["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("corpus fetch")),
        "{value}"
    );
}

// ---------------------------------------------------------------- 薄壳门禁

/// 命令行里不得有第二套评分实现。
///
/// 这条断言是「评分内核只有一处」的可执行判据。它查三件事：命令行源码里没有在分数字段上
/// 做算术（哪怕只是换算百分比）、没有内核阈值的字面量、没有自己的对齐实现；并且**正向**
/// 确认三个内核入口真的被调用了——只写禁令的话，一个把评分整段删掉的实现也能通过。
#[test]
fn the_cli_carries_no_scoring_logic_of_its_own() {
    let sources = collect_sources(&manifest_dir().join("src"));
    assert!(
        sources.len() >= 5,
        "应当扫到命令行的全部源文件：{sources:?}"
    );

    let score_fields = [
        "completeness",
        "accuracy_strict",
        "accuracy_lenient",
        "fluency",
    ];
    let kernel_thresholds = ["0.35", "0.85", "0.97", "MIN_MATCH_RATIO", "MAX_CER"];
    let mut delegates = Vec::new();

    for (path, text) in &sources {
        for line in text.lines() {
            for field in score_fields {
                let Some(rest) = line.split_once(field).map(|(_, rest)| rest) else {
                    continue;
                };
                // 字段名后紧跟运算符才算算术；`accuracy_lenient_below,` 这类字段名不算。
                let next = rest.trim_start().chars().next().unwrap_or(',');
                assert!(
                    !matches!(next, '*' | '+' | '/') && !rest.starts_with(" - "),
                    "{}：不得在分数字段上做算术，比例请原样呈现\n{line}",
                    path.display()
                );
            }
            for threshold in kernel_thresholds {
                assert!(
                    !line.contains(threshold),
                    "{}：不得在命令行里重复内核阈值 `{threshold}`\n{line}",
                    path.display()
                );
            }
            assert!(
                !line.contains("fn align") && !line.contains("fn score_"),
                "{}：对齐与评分的实现只许有一份，在 yunjian-recite 里\n{line}",
                path.display()
            );
        }
        delegates.push(text.clone());
    }

    let all = delegates.join("\n");
    for entry in [
        "review_typed(",
        "grade_typed(",
        "align(",
        "Scheduler::open(",
        "issue_review_ticket_at(",
        "submit_review_ticket_at(",
    ] {
        assert!(
            all.contains(entry),
            "命令行必须真的调用内核入口 `{entry}`，否则这条守卫是空的"
        );
    }
}

fn collect_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    walk(dir, &mut found);
    found
}

fn walk(dir: &Path, found: &mut Vec<(PathBuf, String)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("读取目录失败 {}：{error}", dir.display()));
    for entry in entries {
        let path = entry.expect("读取目录项").path();
        if path.is_dir() {
            walk(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("读取源文件失败 {}：{error}", path.display()));
            found.push((path, text));
        }
    }
}
