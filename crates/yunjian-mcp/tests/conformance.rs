//! MCP 端到端一致性门禁。
//!
//! 本测试关闭 libtest harness，并把当前可执行文件作为服务端子进程重新启动。父进程通过
//! `rmcp` 的真实客户端完成协商、列工具与逐工具调用；手写 JSON-RPC 无法替代这里对 framing、
//! 生命周期和协议错误映射的覆盖。

#[allow(dead_code)]
mod common;

use common::{
    ANCHOR, EXPECTED_TOOLS_AI, EXPECTED_TOOLS_OFFLINE, Sandbox, args, assert_protocol_only,
    expected_tools_all, structured, tool_named,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo, ProtocolVersion, Tool};
use rmcp::{ClientHandler, RoleClient, ServerHandler, ServiceExt};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use yunjian_core::{CorpusConfig, CorpusHandle, Yunjian};

const CHILD_ENV: &str = "YUNJIAN_MCP_CONFORMANCE_CHILD";
const CORPUS_ENV: &str = "YUNJIAN_MCP_CONFORMANCE_CORPUS";

/// 云笺锁定并要求继续支持的 MCP 协议契约。
///
/// 在固定的 rmcp 3.1.2 中，稳定默认 `ProtocolVersion::LATEST` 指向 `2025-11-25`，而
/// `2026-07-28` 是 `KNOWN_VERSIONS` 中可显式协商的 draft。`ServerHandler` 默认把全部
/// `KNOWN_VERSIONS` 作为 `supported_protocol_versions()`；其文档要求未实现某 revision 的
/// 服务端主动缩窄列表，云笺没有缩窄。因此项目契约固定为该明确 revision，不能改成会随 SDK
/// 漂移的 `ProtocolVersion::LATEST`。
const REQUIRED_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2026_07_28;

/// 单个 `tools/call` 响应的线上字节上限。
///
/// 128 KiB 能容纳 50 条带完整命中行的最坏检索 fixture；它又足够低，使服务端上限若被误改为
/// 5000（核心层最终返回 100 条），本测试稳定失败。按捕获到的完整 JSON-RPC 帧计量，而不是只量
/// `structuredContent`，因此兼容 text block 的重复成本也在预算内。
const TOOL_CALL_RESPONSE_BYTE_CEILING: usize = 128 * 1024;
const SIZE_FIXTURE_ROWS: usize = 100;

fn main() {
    if std::env::var_os(CHILD_ENV).is_some() {
        child_main();
        return;
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建一致性测试运行时")
        .block_on(parent_main());
}

fn child_main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    tracing::info!("启动 MCP 一致性测试服务端");

    let corpus = std::env::var_os(CORPUS_ENV)
        .map(std::path::PathBuf::from)
        .expect("子进程必须收到 fixture 语料路径");
    let data_dir = corpus
        .parent()
        .expect("fixture 语料应有父目录")
        .join("conformance-data");
    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(corpus),
        data_dir,
        archive: None,
    })
    .expect("子进程打开 fixture 语料");
    let server = yunjian_mcp::YunjianServer::new(Yunjian::new(handle));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建一致性服务端运行时")
        .block_on(yunjian_mcp::serve_stdio(server))
        .expect("一致性服务端应正常结束");
}

async fn parent_main() {
    let sandbox = Sandbox::new();
    seed_size_fixture(&sandbox.corpus());

    let supported = yunjian_mcp::YunjianServer::new(sandbox.core()).supported_protocol_versions();
    assert!(
        supported.contains(&REQUIRED_PROTOCOL_VERSION),
        "YunjianServer 的支持集必须显式包含项目锁定版本 {REQUIRED_PROTOCOL_VERSION}，实为 {supported:?}"
    );

    let default_client = ();
    let default_client_offer = default_client.get_info().protocol_version;
    let default_session = ProcessSession::connect(&sandbox, default_client).await;
    assert_eq!(
        default_session
            .client
            .peer_info()
            .expect("默认客户端握手后应有服务端信息")
            .protocol_version,
        default_client_offer,
        "服务端应回显真实默认 rmcp 客户端实际 offer 的协议版本"
    );
    let default_output = default_session.shutdown().await;
    assert_protocol_only(&default_output.stdout);
    assert!(
        default_output.status.success(),
        "默认客户端会话的服务端子进程退出失败：{}",
        default_output.stderr
    );

    // 此会话由真实 rmcp client 显式请求 MCP 2026-07-28。
    // 成功表示当前固定版本的 rmcp 与 YunjianServer 能经 stdio initialize 接受并回显
    // 该 revision，且本测试覆盖的 framing、tools/list、五个 tools/call、schema、
    // 错误映射与响应体积约束均能在该协商上下文中工作。
    //
    // 这不是 2026-07-28 全协议合规声明：未覆盖 tasks、subscriptions、sampling、
    // roots、elicitation、Streamable HTTP、SEP-2243 standard headers、授权，
    // 以及跨 SDK 互操作。且 client 与 server 用同一个 SDK，可能共同接受同一个 SDK bug。
    let mut required_client = ClientInfo::default();
    required_client.protocol_version = REQUIRED_PROTOCOL_VERSION;
    let session = ProcessSession::connect(&sandbox, required_client).await;

    assert_eq!(
        session
            .client
            .peer_info()
            .expect("握手后应有服务端信息")
            .protocol_version,
        REQUIRED_PROTOCOL_VERSION,
        "必须协商到项目锁定的 MCP 协议版本"
    );

    let tools = session
        .client
        .list_all_tools()
        .await
        .expect("调用 tools/list");
    assert_exact_tool_set(&tools);
    assert_schemas(&tools);

    for call in call_cases() {
        let tool = tool_named(&tools, call.name);
        let before = session.captured_len();
        let result = session
            .client
            .call_tool(tool_params(call.name, call.valid_arguments))
            .await
            .unwrap_or_else(|error| panic!("{} 的合法调用不应返回协议错误：{error}", call.name));
        assert_eq!(
            result.is_error,
            Some(false),
            "{} 的合法调用应成功",
            call.name
        );
        validate_structured_content(tool, &result);
        session.assert_response_ceiling(call.name, before);

        let before = session.captured_len();
        let result = session
            .client
            .call_tool(tool_params(call.name, call.invalid_arguments))
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{name} 非法参数不应破坏 tools/call：{error}",
                    name = call.name
                )
            });
        assert_invalid_params(call.name, &result);
        session.assert_response_ceiling(call.name, before);
    }

    // 请求一个故意超大的 limit：正常实现会把它截到 50，因此这里实际量的是 limit=50 的最坏响应；
    // 若服务端硬上限被临时改成 5000，核心层会返回全部 100 条，完整协议帧必须超过预算并使测试变红。
    let before = session.captured_len();
    let result = session
        .client
        .call_tool(tool_params(
            "search_poem",
            args(vec![("query", json!("月")), ("limit", json!(5000))]),
        ))
        .await
        .expect("最坏检索调用应成功");
    session.assert_response_ceiling("search_poem(limit=50)", before);
    let payload = structured(&result);
    assert_eq!(payload["limit"], json!(50), "服务端应把 limit 截到 50");
    assert_eq!(
        payload["hits"].as_array().map(Vec::len),
        Some(50),
        "体积测试必须真的返回 50 条，不能让空 fixture 蒙混过关"
    );
    validate_structured_content(tool_named(&tools, "search_poem"), &result);

    let output = session.shutdown().await;
    assert_protocol_only(&output.stdout);
    assert!(
        output.status.success(),
        "一致性服务端子进程退出失败：{}",
        output.stderr
    );
    assert!(
        !output.stderr.trim().is_empty(),
        "stderr 可以有日志且应非空，以证明它与协议 stdout 分离"
    );
}

struct CallCase {
    name: &'static str,
    valid_arguments: Value,
    invalid_arguments: Value,
}

fn call_cases() -> Vec<CallCase> {
    vec![
        CallCase {
            name: "search_poem",
            valid_arguments: args(vec![("query", json!("明月"))]),
            invalid_arguments: args(vec![("unexpected", json!(true))]),
        },
        CallCase {
            name: "explain_poem",
            valid_arguments: args(vec![("poem_id", json!(ANCHOR))]),
            invalid_arguments: args(vec![("poem_id", json!(ANCHOR)), ("extra", json!(1))]),
        },
        CallCase {
            name: "find_similar_poem",
            valid_arguments: args(vec![("poem_id", json!(ANCHOR))]),
            invalid_arguments: args(vec![("poem_id", json!(ANCHOR)), ("by", json!("embedding"))]),
        },
        CallCase {
            name: "appreciate_poem",
            valid_arguments: args(vec![("poem_id", json!(ANCHOR))]),
            invalid_arguments: args(vec![("style", json!("简明"))]),
        },
        CallCase {
            name: "generate_poem",
            valid_arguments: args(vec![("form", json!("七言绝句")), ("theme", json!("思乡"))]),
            invalid_arguments: args(vec![("form", json!("十四行诗")), ("theme", json!("思乡"))]),
        },
    ]
}

fn tool_params(name: &'static str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!("一致性测试工具参数必须是 JSON 对象")
    };
    CallToolRequestParams::new(name).with_arguments(arguments)
}

fn assert_exact_tool_set(tools: &[Tool]) {
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        expected_tools_all(),
        "tools/list 必须恰好是规范五工具全集"
    );
    assert_eq!(
        names.len(),
        EXPECTED_TOOLS_OFFLINE.len() + EXPECTED_TOOLS_AI.len(),
        "两个规范子集的并集应无重名"
    );
}

fn assert_schemas(tools: &[Tool]) {
    for tool in tools {
        let input = Value::Object(tool.input_schema.as_ref().clone());
        assert_eq!(
            input["type"],
            json!("object"),
            "{} inputSchema 根必须是 object",
            tool.name
        );
        assert_eq!(
            input["$schema"],
            json!("https://json-schema.org/draft/2020-12/schema"),
            "{} inputSchema 必须显式声明 Draft 2020-12",
            tool.name
        );
        assert_eq!(
            input["additionalProperties"],
            json!(false),
            "{} 使用 deny_unknown_fields，线上 schema 也必须拒绝额外参数",
            tool.name
        );
        assert!(
            jsonschema::draft202012::meta::is_valid(&input),
            "{} inputSchema 必须通过 Draft 2020-12 meta-schema：{input}",
            tool.name
        );

        let output = Value::Object(
            tool.output_schema
                .as_ref()
                .unwrap_or_else(|| panic!("{} 必须声明 outputSchema", tool.name))
                .as_ref()
                .clone(),
        );
        assert!(
            jsonschema::draft202012::meta::is_valid(&output),
            "{} outputSchema 必须通过 Draft 2020-12 meta-schema：{output}",
            tool.name
        );
    }
}

fn validate_structured_content(tool: &Tool, result: &CallToolResult) {
    let payload = structured(result);
    let schema = Value::Object(
        tool.output_schema
            .as_ref()
            .expect("工具必须声明 outputSchema")
            .as_ref()
            .clone(),
    );
    let validator = jsonschema::draft202012::new(&schema)
        .unwrap_or_else(|error| panic!("{} outputSchema 无法编译：{error}", tool.name));
    let errors = validator
        .iter_errors(&payload)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "{} structuredContent 不符合 outputSchema：{errors:?}\npayload={payload}",
        tool.name
    );
}

fn assert_invalid_params(name: &str, result: &CallToolResult) {
    assert_eq!(
        result.is_error,
        Some(true),
        "{name} 非法参数必须映射为工具错误"
    );
    assert!(
        result.structured_content.is_none(),
        "{name} 参数反序列化失败不得伪造 structuredContent"
    );
    let message = result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("非法参数工具错误必须带文本诊断");
    assert!(
        message.contains("failed to deserialize parameters"),
        "{name} 工具错误应说明参数反序列化失败：{message}"
    );
}

fn seed_size_fixture(path: &std::path::Path) {
    let mut connection = Connection::open(path).expect("打开体积 fixture 语料");
    connection
        .execute(
            "INSERT OR IGNORE INTO author(name) VALUES ('体积测试作者')",
            [],
        )
        .expect("写体积 fixture 作者");
    let transaction = connection.transaction().expect("开始体积 fixture 事务");
    for index in 0..SIZE_FIXTURE_ROWS {
        let stable_id = format!("fixture:size-{index:03}");
        let title = format!("体积测试诗{index:03}");
        let body = format!("月{}", "山".repeat(300));
        transaction
            .execute(
                "INSERT INTO poem(stable_id, content_hash, source_locator, source_locator_kind, \
                 genre, title, title_raw, author, dynasty, dynasty_raw, body, body_original, script, \
                 first_line, last_chars, line_count, char_count, provenance_source, \
                 provenance_revision, provenance_kind, provenance_license, provenance_license_class, \
                 work_group, edition_group) VALUES (?1, ?2, ?3, 'native', 'shi', ?4, ?4, \
                 '体积测试作者', '唐', '唐', ?5, ?5, 'simplified', ?5, '[\"山\"]', 1, 301, \
                 'conformance-fixture', 'rev-size', '原文', 'MIT', 'permissive', ?1, ?1)",
                params![
                    stable_id,
                    format!("size-hash-{index:03}"),
                    format!("size-locator-{index:03}"),
                    title,
                    body,
                ],
            )
            .expect("写体积 fixture 作品");
    }
    transaction.commit().expect("提交体积 fixture 事务");
    connection
        .execute(
            "UPDATE corpus_meta SET poem_count = (SELECT COUNT(*) FROM poem), \
             input_row_count = (SELECT COUNT(*) FROM poem)",
            [],
        )
        .expect("更新 fixture 守恒计数");
    connection.close().expect("关闭体积 fixture 语料");
}

struct ProcessSession<C: ClientHandler> {
    client: rmcp::service::RunningService<RoleClient, C>,
    child: tokio::process::Child,
    stderr_task: tokio::task::JoinHandle<Vec<u8>>,
    capture: Arc<Mutex<Vec<u8>>>,
}

impl<C: ClientHandler> ProcessSession<C> {
    async fn connect(sandbox: &Sandbox, client_handler: C) -> Self {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let mut command = tokio::process::Command::new(
            std::env::current_exe().expect("定位一致性测试可执行文件"),
        );
        command
            .env(CHILD_ENV, "1")
            .env(CORPUS_ENV, sandbox.corpus())
            .env_remove("RUST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("启动一致性服务端子进程");
        let stdin = child.stdin.take().expect("获取子进程 stdin");
        let stdout = child.stdout.take().expect("获取子进程 stdout");
        let mut stderr = child.stderr.take().expect("获取子进程 stderr");
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr
                .read_to_end(&mut bytes)
                .await
                .expect("读取子进程 stderr");
            bytes
        });
        let reader = CapturingReader::new(stdout, Arc::clone(&capture));
        let client = client_handler
            .serve((reader, stdin))
            .await
            .expect("真实 rmcp 客户端完成握手");
        Self {
            client,
            child,
            stderr_task,
            capture,
        }
    }

    fn captured_len(&self) -> usize {
        self.capture.lock().expect("读取 stdout 捕获长度").len()
    }

    fn assert_response_ceiling(&self, name: &str, before: usize) {
        let capture = self.capture.lock().expect("读取 tools/call 响应字节");
        let response = &capture[before..];
        assert!(!response.is_empty(), "{name} 应产生一个 JSON-RPC 响应帧");
        assert_eq!(response.last(), Some(&b'\n'), "{name} 响应帧应以换行结束");
        for frame in response.split_inclusive(|byte| *byte == b'\n') {
            assert!(
                frame.len() <= TOOL_CALL_RESPONSE_BYTE_CEILING,
                "{name} 单个 tools/call 响应为 {} bytes，超过 {} bytes 上限",
                frame.len(),
                TOOL_CALL_RESPONSE_BYTE_CEILING
            );
        }
    }

    async fn shutdown(self) -> ProcessOutput {
        let Self {
            client,
            mut child,
            stderr_task,
            capture,
        } = self;
        client.cancel().await.expect("关闭真实 rmcp 客户端");
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("服务端应在客户端关闭后及时退出")
            .expect("等待一致性服务端子进程");
        let stderr = stderr_task.await.expect("汇合 stderr 任务");
        let stdout = capture.lock().expect("读取完整 stdout").clone();
        ProcessOutput {
            stdout,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            status,
        }
    }
}

struct ProcessOutput {
    stdout: Vec<u8>,
    stderr: String,
    status: std::process::ExitStatus,
}

struct CapturingReader<R> {
    inner: R,
    capture: Arc<Mutex<Vec<u8>>>,
}

impl<R> CapturingReader<R> {
    fn new(inner: R, capture: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { inner, capture }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CapturingReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        match Pin::new(&mut this.inner).poll_read(context, buffer) {
            Poll::Ready(Ok(())) => {
                let after = buffer.filled().len();
                if after > before {
                    this.capture
                        .lock()
                        .expect("记录 stdout")
                        .extend_from_slice(&buffer.filled()[before..after]);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
