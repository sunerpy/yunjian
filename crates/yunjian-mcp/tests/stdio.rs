//! MCP stdio 的进程级契约。
//!
//! 本测试关闭 libtest harness：harness 会把 `running N tests` 写到 stdout，正好污染本测试
//! 要逐字节审计的协议流。父进程复用当前测试可执行文件启动子进程；子进程只运行服务端，
//! 父进程则用真实 `rmcp` 客户端完成握手、`tools/list` 与 `tools/call`。

#[allow(dead_code)]
mod common;

use common::assert_protocol_only;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::Value;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

const CHILD_ENV: &str = "YUNJIAN_MCP_STDIO_TEST_CHILD";

/// 缺语料场景用来探测的工具。固定按名字取，不用 `tools[0]`：声明顺序变一次就会静默换掉
/// 被测对象。
const PROBE_TOOL: &str = "search_poem";

fn main() {
    if std::env::var_os(CHILD_ENV).is_some() {
        child_main();
        return;
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建测试运行时")
        .block_on(parent_main());
}

fn child_main() {
    let level = if std::env::args().any(|argument| argument == "trace") {
        "trace"
    } else {
        "info"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    tracing::info!("启动 stdio 测试服务端");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建服务端运行时")
        .block_on(yunjian_mcp::serve_stdio(
            yunjian_mcp::YunjianServer::without_corpus(),
        ))
        .expect("stdio 服务应正常结束");
}

async fn parent_main() {
    let normal = run_session(Session::Normal).await;
    assert_protocol_only(&normal.stdout);
    assert!(
        normal.status.success(),
        "普通会话退出失败：{}",
        normal.stderr
    );

    let trace = run_session(Session::Trace).await;
    assert_protocol_only(&trace.stdout);
    assert!(
        trace.status.success(),
        "trace 会话退出失败：{}",
        trace.stderr
    );
    assert!(
        trace.stderr.contains("TRACE"),
        "RUST_LOG=trace 与 --log-level trace 应在 stderr 产生 trace 日志：{}",
        trace.stderr
    );

    let missing = run_session(Session::MissingCorpus).await;
    assert_protocol_only(&missing.stdout);
    assert!(
        missing.status.success(),
        "缺语料会话必须以 0 退出：{}",
        missing.stderr
    );
    let result = missing.call_result.expect("缺语料场景必须执行 tools/call");
    assert_eq!(result.is_error, Some(true), "缺语料应是工具级错误");
    let structured = result
        .structured_content
        .expect("缺语料必须返回 structuredContent");
    assert_eq!(structured["code"], "corpus_missing");
    assert!(
        structured["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("yunjian corpus fetch")),
        "缺语料提示必须点名获取命令：{structured}"
    );
}

#[derive(Clone, Copy)]
enum Session {
    Normal,
    Trace,
    MissingCorpus,
}

struct SessionOutput {
    stdout: Vec<u8>,
    stderr: String,
    status: std::process::ExitStatus,
    call_result: Option<rmcp::model::CallToolResult>,
}

async fn run_session(session: Session) -> SessionOutput {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let mut command = tokio::process::Command::new(std::env::current_exe().expect("定位测试进程"));
    command
        .env(CHILD_ENV, "1")
        .env_remove("APP_CONFIG")
        .env_remove("YUNJIAN_CORPUS_PATH")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let empty_data_home;
    match session {
        Session::Normal => {}
        Session::Trace => {
            command
                .env("RUST_LOG", "trace")
                .arg("--log-level")
                .arg("trace");
        }
        Session::MissingCorpus => {
            empty_data_home = tempfile_path("missing-corpus");
            std::fs::create_dir_all(&empty_data_home).expect("创建空 XDG_DATA_HOME");
            command.env("XDG_DATA_HOME", &empty_data_home);
        }
    }

    let mut child = command.spawn().expect("启动 MCP 子进程");
    let stdin = child.stdin.take().expect("获取子进程 stdin");
    let stdout = child.stdout.take().expect("获取子进程 stdout");
    let stderr = child.stderr.take().expect("获取子进程 stderr");
    let stderr_task = tokio::spawn(async move {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.expect("读取 stderr");
        String::from_utf8_lossy(&bytes).into_owned()
    });

    let reader = CapturingReader::new(stdout, Arc::clone(&capture));
    let client = ().serve((reader, stdin)).await.expect("完成 MCP 握手");
    let tools = client.list_all_tools().await.expect("调用 tools/list");
    assert!(!tools.is_empty(), "至少应暴露一个工具");
    assert!(
        tools.iter().any(|tool| tool.name == PROBE_TOOL),
        "缺语料探针要调的 {PROBE_TOOL} 应在 tools/list 里"
    );
    let call_result = if matches!(session, Session::MissingCorpus) {
        // 参数必须合法。缺语料是**语料层**的失败，而参数错误会先在反序列化处被拦掉，
        // 于是拿到的是 invalid_params 而不是 corpus_missing——那样这条断言就换了被测对象。
        let mut arguments = serde_json::Map::new();
        arguments.insert("query".to_owned(), Value::String("明月".to_owned()));
        Some(
            client
                .call_tool(CallToolRequestParams::new(PROBE_TOOL).with_arguments(arguments))
                .await
                .expect("缺语料应返回工具结果而不是协议错误"),
        )
    } else {
        None
    };
    client.cancel().await.expect("关闭 MCP 客户端");

    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("MCP 子进程应在 stdin 关闭后退出")
        .expect("等待 MCP 子进程");
    let stderr = stderr_task.await.expect("汇合 stderr 读取任务");
    let stdout = capture.lock().expect("读取 stdout 捕获").clone();

    SessionOutput {
        stdout,
        stderr,
        status,
        call_result,
    }
}

fn tempfile_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "yunjian-mcp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间晚于 UNIX 纪元")
            .as_nanos()
    ))
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
