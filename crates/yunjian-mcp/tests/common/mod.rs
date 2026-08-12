//! MCP 工具的**规范名单**与测试脚手架，由 `yunjian-mcp` 的各集成测试共享。
//!
//! # 为什么名单必须只有一份，且必须在这里
//!
//! 工具分两批交付：本批（todo 32）落地三个离线工具，todo 42 再加两个 AI 工具。若任一批在
//! 自己的测试里写「`tools/list` 恰好等于 N 个」，另一批一落地就会把它打红——而那条红是
//! 假警报，掩盖真实回归。所以：
//!
//! - [`EXPECTED_TOOLS_OFFLINE`] 与 [`EXPECTED_TOOLS_AI`] 在这里各定义一次；
//! - 每一批**只断言自己那一组是 `tools/list` 的子集**；
//! - **唯一**可以做精确全集断言的地方是 todo 35 的一致性 harness——到那时五个工具都在了。
//!
//! # 为什么用内存双工而不是子进程
//!
//! `tests/stdio.rs` 已经在进程级审计 stdout 的每一个字节，那件事不需要做第二遍。本模块要的
//! 是**协议级**事实：`tools/list` 的 JSON 里 annotation 长什么样、`tools/call` 的结果是否
//! 同时带 `structuredContent` 与 text block。用 `tokio::io::duplex` 把真实 `rmcp` 服务端与
//! 真实 `rmcp` 客户端接在一起，这些都走完整的序列化路径，同时保留 libtest harness，于是
//! `cargo test` 能逐条打印哪个断言跑过了。

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use yunjian_core::{CorpusConfig, CorpusHandle, SCHEMA_VERSION, Yunjian};
use yunjian_mcp::YunjianServer;

/// 无需 API key、无需网络的工具。**本批（todo 32）交付并断言这一组。**
pub const EXPECTED_TOOLS_OFFLINE: [&str; 3] = ["search_poem", "explain_poem", "find_similar_poem"];

/// 需要 BYOK 凭据的工具。**todo 42 交付；本批不断言它们存在。**
pub const EXPECTED_TOOLS_AI: [&str; 2] = ["appreciate_poem", "generate_poem"];

/// 规范全集 = 离线子集 ∪ AI 子集，共 5 个。**只有 todo 35 可以拿它做精确相等断言。**
#[must_use]
pub fn expected_tools_all() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = EXPECTED_TOOLS_OFFLINE
        .iter()
        .chain(EXPECTED_TOOLS_AI.iter())
        .copied()
        .collect();
    all.sort_unstable();
    all
}

/// MCP 规范允许的工具名字符集：`A-Z a-z 0-9 _ - .`，长度 1..=128。
///
/// 中文只能出现在 `annotations.title`，放进 `name` 会让部分客户端拒绝整份工具清单。
#[must_use]
pub fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 128
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
}

/// 随包 schema 的路径。跨 crate 只读一个文件，不引入依赖。
const CORPUS_SCHEMA_PATH: &str = "../yunjian-corpus/schema.sql";

/// 黄金查询契约的 fixture 语料。
const SHARED_FIXTURES_PATH: &str = "../yunjian-core/tests/fixtures/poems.toml";

/// 详情类断言锚定的那首诗。
pub const ANCHOR: &str = "fixture:tang-libai-jingyesi";

/// 与锚定作品共享标签「思乡」「春」的另一首，供相似度断言使用。
pub const ANCHOR_NEIGHBOUR: &str = "fixture:song-wanganshi-bochuanguazhou";

/// 极小平水韵子集：(韵部, 声调, 字)。
///
/// 刻意留出「床」「低」这两个常用字不收——「韵书未收即未知平仄」这条断言需要一个真实的
/// 未收字才能成立，用生僻字会让它看起来像边角情形。
const PINGSHUI_ROWS: &[(&str, &str, &str)] = &[
    ("七阳", "level", "光"),
    ("七阳", "level", "霜"),
    ("七阳", "level", "乡"),
    ("十五删", "level", "间"),
    ("十五删", "level", "山"),
    ("十五删", "level", "还"),
    ("八庚", "level", "明"),
];

/// 挂了韵部归属的作品：(`poem_id`, 韵部, 声调)。两首同属七阳之外的删韵，使同韵部这一项
/// 在 fixture 上真的能命中——只挂一首的话该分量恒为 0，断言就测不到东西。
const POEM_RHYME_ROWS: &[(&str, &str, &str)] = &[
    (ANCHOR, "七阳", "level"),
    (ANCHOR_NEIGHBOUR, "十五删", "level"),
    ("fixture:tang-libai-zaofabaidicheng", "十五删", "level"),
    ("fixture:tang-wangchangling-chusai", "十五删", "level"),
];

static NEXT_SANDBOX: AtomicU32 = AtomicU32::new(0);

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

/// 一次测试的独立目录；析构时整棵删掉。
pub struct Sandbox {
    dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Sandbox {
    /// 建一份现造的 fixture 语料库。
    #[must_use]
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-mcp-tools-{}-{}",
            std::process::id(),
            NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建沙箱目录");
        let sandbox = Self { dir };
        write_corpus(&sandbox.corpus());
        sandbox
    }

    pub fn corpus(&self) -> PathBuf {
        self.dir.join("corpus.db")
    }

    #[allow(dead_code)]
    pub fn app_data_dir(&self) -> PathBuf {
        self.dir.join("app-data")
    }

    #[allow(dead_code)]
    pub fn corpus_row_count(&self) -> usize {
        let connection = Connection::open(self.corpus()).expect("打开 fixture 语料统计行数");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM poem", [], |row| row.get(0))
            .expect("统计 fixture 作品行数");
        usize::try_from(count).expect("fixture 作品行数应为非负整数")
    }

    /// 打开这份 fixture 语料并交回核心客户端。
    #[must_use]
    pub fn core(&self) -> Yunjian {
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(self.corpus()),
            data_dir: self.dir.join("corpus-data"),
            archive: None,
        })
        .expect("打开 fixture 语料库");
        Yunjian::new(handle)
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

/// 把 `write_corpus` 用到的 fixture 读出来。
fn shared_fixtures() -> SharedFixtures {
    let path = manifest_dir().join(SHARED_FIXTURES_PATH);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 fixture 失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 fixture 失败 {}：{error}", path.display()))
}

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
    for (poem_id, group, tone) in POEM_RHYME_ROWS {
        connection
            .execute(
                "INSERT INTO poem_rhyme_group(poem_id, rhyme_book, rhyme_group, tone, confidence) \
                 VALUES (?1, 'pingshui', ?2, ?3, 'unambiguous')",
                params![poem_id, group, tone],
            )
            .expect("写韵部归属");
    }

    // 两条集评而不是一条：「每条集评都带出处」这个断言在只有一条时几乎测不到东西。
    for (id, poem_id, text, work, author, dynasty, year, note) in COMMENTARY_ROWS {
        connection
            .execute(
                "INSERT INTO commentary(id, poem_id, text, citation_work, citation_author, \
                 citation_dynasty, citation_dynasty_raw, citation_work_completed_by, \
                 citation_source_note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
                params![id, poem_id, text, work, author, dynasty, year, note],
            )
            .expect("写集评");
    }

    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, schema_version, corpus_version, built_at, \
             source_manifest_sha256, poem_count, finding_count, input_row_count, \
             index_detail_mode, derived_indexes, shipped_scope, builder_sqlite_version, \
             integrity_check) \
             VALUES (1, ?1, 'mcp-fixture-v1', '2026-08-12T00:00:00Z', ?2, ?3, 0, ?3, \
             'full', 'first_launch', '10k', '3.51.0', 'ok')",
            params![SCHEMA_VERSION, "0".repeat(64), fixtures.poems.len() as i64],
        )
        .expect("写 corpus_meta");
    connection.close().expect("关闭 fixture 语料库");
}

type CommentaryRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
    &'static str,
);

/// 两条公有领域集评，出处齐备。
const COMMENTARY_ROWS: &[CommentaryRow] = &[
    (
        "fixture-commentary-001",
        ANCHOR,
        "「床前明月光」四句，妙絕古今，蓋以無意得之。",
        "唐诗别裁集",
        "沈德潜",
        "清",
        1717,
        "卷十九・五言絕句；据四部丛刊本，修订号 1234567",
    ),
    (
        "fixture-commentary-002",
        ANCHOR,
        "太白思鄉之作，語近而情遠。",
        "唐诗归",
        "钟惺",
        "明",
        1617,
        "卷十六；据万历刻本，修订号 7654321",
    ),
];

/// 一条已接好的会话：真实 `rmcp` 客户端 + 真实 `YunjianServer`，中间是内存双工。
pub struct Session {
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
}

impl Session {
    /// 在内存双工上接好客户端与服务端。
    pub async fn connect(server: YunjianServer) -> Self {
        let (client_side, server_side) = tokio::io::duplex(1 << 20);
        let (server_read, server_write) = tokio::io::split(server_side);
        let (client_read, client_write) = tokio::io::split(client_side);
        tokio::spawn(async move {
            if let Ok(service) = server.serve((server_read, server_write)).await {
                let _ = service.waiting().await;
            }
        });
        let client = ().serve((client_read, client_write)).await.expect("完成 MCP 握手");
        Self { client }
    }

    /// 取得 `tools/list` 的结果。
    pub async fn tools(&self) -> Vec<Tool> {
        self.client.list_all_tools().await.expect("调用 tools/list")
    }

    /// 调用一个工具。参数按 JSON 对象传入。
    pub async fn call(&self, name: &'static str, arguments: Value) -> CallToolResult {
        let params = match arguments {
            Value::Null => CallToolRequestParams::new(name),
            Value::Object(object) => CallToolRequestParams::new(name).with_arguments(object),
            other => panic!("工具参数必须是 JSON 对象，实为 {other}"),
        };
        self.client
            .call_tool(params)
            .await
            .unwrap_or_else(|error| panic!("调用 {name} 应返回工具结果而不是协议错误：{error}"))
    }

    /// 关闭会话。
    pub async fn shutdown(self) {
        self.client.cancel().await.expect("关闭 MCP 客户端");
    }
}

/// 按名字取一个工具，取不到就带上现有清单 panic。
#[must_use]
pub fn tool_named<'a>(tools: &'a [Tool], name: &str) -> &'a Tool {
    tools
        .iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| {
            let present: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
            panic!("tools/list 里没有 {name}；现有：{present:?}")
        })
}

/// 把工具序列化成线上 JSON，用于断言 `readOnlyHint` 这类 camelCase 字段。
///
/// **断言必须打在 JSON 上而不是 Rust 结构体上**：客户端读到的是前者，而 `Option<bool>` 为
/// `None` 时字段会被整个省略——那正是「annotation 缺失」在线上的形态。
#[must_use]
pub fn tool_json(tool: &Tool) -> Value {
    serde_json::to_value(tool).expect("序列化 Tool")
}

/// 取出结果里的 `structuredContent`。
#[must_use]
pub fn structured(result: &CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("结果必须带 structuredContent")
}

/// 取出结果里第一个 text block 的文本。
#[must_use]
pub fn first_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .expect("结果必须带至少一个 text block")
}

/// 构造 `tools/call` 的参数对象。
#[must_use]
pub fn args(pairs: Vec<(&str, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in pairs {
        object.insert(key.to_owned(), value);
    }
    Value::Object(object)
}
