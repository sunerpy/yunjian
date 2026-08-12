//! `yunjian mcp --http` 的验收：认证、跨站、绑定守卫、token 保密与关停。
//!
//! # 为什么这里保留 libtest harness，而 `stdio.rs` 不能
//!
//! `stdio.rs` 逐字节审计子进程 stdout，libtest 自己的 `running N tests` 就在那条流里，所以
//! 它只能 `harness = false`。本测试审计的是**两条流里都不得出现 token 值**——libtest 的统计
//! 行不含 token，因此不构成干扰，而九条验收逐条打印用例名是有价值的。
//!
//! # 唯一一处子进程：token 保密
//!
//! 「token 不出现在 stdout / stderr」只能在**进程**层面验证：`tracing` 写的是真实 fd，
//! 进程内没法可靠地把它接住。做法与 `stdio.rs` 同源——用 `current_exe()` 自举一个子进程，
//! 让它跑完整的「起服务 → 真实客户端握手 → tools/call → 关停」，父进程抓两条管道 grep
//! token 值。这条断言因此覆盖了启动日志、请求日志与关停日志三个泄露点。

// 本测试只用得到 `Sandbox`，而 `tests/tools.rs` 用得到其余部分。allow 打在这一处而不是
// `common/mod.rs` 里，否则 `tools.rs` 也会失去 dead_code 检查。
#[allow(dead_code)]
mod common;

use common::Sandbox;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio_util::sync::CancellationToken;
use yunjian_mcp::YunjianServer;
use yunjian_mcp::http::token::BearerToken;
use yunjian_mcp::http::{DEFAULT_BIND, HttpOptions, HttpServer};

/// 子进程模式的开关。父进程设它，子进程据此改跑服务而不是跑断言。
const CHILD_MARKER: &str = "YUNJIAN_HTTP_TOKEN_CHILD";

/// 子进程写 token 文件的位置由父进程指定。
const CHILD_TOKEN_FILE: &str = "YUNJIAN_HTTP_TOKEN_CHILD_FILE";

/// 子进程入口所在的用例名。父进程用 `--exact` 只跑它。
const CHILD_TEST_NAME: &str = "the_token_never_reaches_stdout_or_stderr";

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-mcp-http-{tag}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

fn ephemeral(token_file: &Path) -> HttpOptions {
    HttpOptions {
        http: true,
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        allow_remote: false,
        token_file: Some(token_file.to_path_buf()),
    }
}

/// 一台跑起来的服务：拿到端点、token 明文与关停开关。
struct Running {
    endpoint: String,
    secret: String,
    cancellation: CancellationToken,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Running {
    async fn start(dir: &Path, server: YunjianServer) -> Self {
        let token_file = dir.join("mcp-token");
        let cancellation = CancellationToken::new();
        let bound = HttpServer::bind(&ephemeral(&token_file), cancellation.clone())
            .await
            .expect("绑定 MCP HTTP 服务");
        let endpoint = bound.endpoint();
        let secret = std::fs::read_to_string(&token_file)
            .expect("读回 token 文件")
            .trim()
            .to_owned();
        let task = tokio::spawn(async move { bound.serve(server).await });
        Self {
            endpoint,
            secret,
            cancellation,
            task,
        }
    }

    fn transport(&self, token: Option<&str>) -> StreamableHttpClientTransport<reqwest::Client> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.endpoint.clone());
        if let Some(token) = token {
            config = config.auth_header(token);
        }
        StreamableHttpClientTransport::with_client(reqwest::Client::default(), config)
    }

    async fn post(&self, token: Option<&str>, origin: Option<&str>) -> reqwest::StatusCode {
        let mut request = reqwest::Client::default()
            .post(&self.endpoint)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(INITIALIZE_FRAME);
        if let Some(token) = token {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(origin) = origin {
            request = request.header(reqwest::header::ORIGIN, origin);
        }
        request.send().await.expect("发出 HTTP 请求").status()
    }

    async fn stop(self) {
        self.cancellation.cancel();
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), self.task).await;
        let joined = stopped.expect("cancellation token 必须在 2 秒内让服务停下");
        joined
            .expect("服务任务不得 panic")
            .expect("服务不得异常结束");
    }
}

/// 一条合法但最小的 `initialize` 请求，用来在不建 MCP 会话的前提下探测 HTTP 状态码。
///
/// 用它而不是 `{}`：认证被拒时必须是 401 而不是 400，两者混在一起就分不清「守卫生效」与
/// 「请求本身畸形」。
const INITIALIZE_FRAME: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}"#;

#[tokio::test]
async fn a_valid_bearer_token_completes_the_handshake_and_a_tool_call() {
    let sandbox = Sandbox::new();
    let dir = scratch("happy");
    let running = Running::start(&dir, YunjianServer::new(sandbox.core())).await;

    let client =
        ().serve(running.transport(Some(&running.secret)))
            .await
            .expect("带合法 token 必须完成握手");
    let tools = client.list_all_tools().await.expect("调用 tools/list");
    assert!(
        tools.iter().any(|tool| tool.name == "search_poem"),
        "HTTP 传输上必须能看到工具清单，实为 {:?}",
        tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>()
    );

    let result = client
        .call_tool(
            CallToolRequestParams::new("search_poem").with_arguments(
                serde_json::json!({"query": "明月"})
                    .as_object()
                    .cloned()
                    .expect("参数对象"),
            ),
        )
        .await
        .expect("带合法 token 必须能调用工具");
    assert_ne!(
        result.is_error,
        Some(true),
        "带合法 token 的 tools/call 不该是错误结果：{result:?}"
    );
    assert!(
        result.structured_content.is_some(),
        "HTTP 传输不改变工具契约，结果必须带 structuredContent"
    );

    client.cancel().await.expect("关闭客户端");
    running.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn requests_without_a_token_and_with_a_wrong_token_both_get_401() {
    let dir = scratch("401");
    let running = Running::start(&dir, YunjianServer::without_corpus()).await;

    assert_eq!(
        running.post(None, None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "无 token 的请求必须是 401"
    );
    assert_eq!(
        running.post(Some("not-the-token"), None).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "错 token 的请求必须是 401"
    );
    assert_eq!(
        running
            .post(Some(&format!("{}x", running.secret)), None)
            .await,
        reqwest::StatusCode::UNAUTHORIZED,
        "多一个字符的 token 必须是 401"
    );

    // 反向断言：合法 token 必须被放行。少了这一条，一个「把所有请求都拒掉」的实现也能让
    // 上面三条全绿。
    let allowed = running.post(Some(&running.secret), None).await;
    assert!(
        allowed.is_success(),
        "合法 token 必须被放行，实为 {allowed}"
    );

    running.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_foreign_origin_is_rejected_even_with_a_valid_token() {
    let dir = scratch("origin");
    let running = Running::start(&dir, YunjianServer::without_corpus()).await;

    assert_eq!(
        running
            .post(Some(&running.secret), Some("https://evil.example"))
            .await,
        reqwest::StatusCode::FORBIDDEN,
        "跨站 Origin 必须被拒，且不因带对了 token 而放行"
    );
    assert_eq!(
        running.post(Some(&running.secret), Some("null")).await,
        reqwest::StatusCode::FORBIDDEN,
        "Origin: null（沙箱页面 / file://）必须被拒"
    );

    // 反向断言：本机同端口的 Origin 必须放行，否则这条检查等价于「禁止一切 Origin」，
    // 而那会让浏览器端的正当调用无法与攻击区分。
    let own = format!(
        "http://{}",
        running
            .endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/mcp")
    );
    let allowed = running.post(Some(&running.secret), Some(&own)).await;
    assert!(
        allowed.is_success(),
        "本端口自身的 Origin {own} 必须放行，实为 {allowed}"
    );

    running.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_default_bind_is_loopback_and_a_wildcard_bind_needs_the_flag() {
    assert!(
        DEFAULT_BIND.ip().is_loopback(),
        "默认绑定必须是回环，实为 {DEFAULT_BIND}"
    );
    assert_eq!(DEFAULT_BIND.ip(), Ipv4Addr::LOCALHOST);

    let parsed = probe(&["mcp"]);
    assert_eq!(parsed.bind, DEFAULT_BIND, "不传 --bind 时必须落在回环上");
    assert!(!parsed.http);
    assert!(!parsed.allow_remote);

    let wildcard = probe(&["mcp", "--http", "--bind", "0.0.0.0:8000"]);
    let refusal = wildcard
        .check_bind()
        .expect_err("0.0.0.0 不带 --allow-remote 必须被拒");
    assert!(
        refusal.to_string().contains("--allow-remote"),
        "拒绝理由必须点名 --allow-remote，实为 {refusal}"
    );
    assert_ne!(
        yunjian_mcp::http::EXIT_REMOTE_BIND_REFUSED,
        0,
        "拒绝必须是非零退出码"
    );

    assert!(
        probe(&["mcp", "--http", "--bind", "0.0.0.0:8000", "--allow-remote"])
            .check_bind()
            .is_ok(),
        "显式许可后必须放行"
    );
}

#[test]
fn the_cli_surface_has_no_token_flag() {
    let rejected = Probe::try_parse_from(["mcp", "--http", "--token", "s3cr3t"])
        .expect_err("--token 必须不存在");
    assert_eq!(
        rejected.kind(),
        clap::error::ErrorKind::UnknownArgument,
        "--token 必须是「未知参数」而不是被解析成别的东西：{rejected}"
    );
    assert!(
        Probe::try_parse_from(["mcp", "--http", "--token-file", "/tmp/t"]).is_ok(),
        "--token-file 必须存在：它是 --token 的替代品"
    );

    // 上面只覆盖了这一个 Args 结构。真正的 CLI 表面是整个 clap 定义，所以再扫一遍 CLI 源码：
    // 将来谁在别处加一个 `--token`，这条会红。
    let cli_source = std::fs::read_to_string("../yunjian-cli/src/cli.rs").expect("读 CLI 定义");
    for line in cli_source.lines() {
        assert!(
            !line.contains("\"token\"") && !line.contains("--token\""),
            "CLI 定义里不得出现 --token 长参数：{line}"
        );
    }
}

#[derive(Debug, clap::Parser)]
#[command(name = "mcp")]
struct Probe {
    #[command(flatten)]
    http: HttpOptions,
}

use clap::Parser as _;

fn probe(argv: &[&str]) -> HttpOptions {
    Probe::try_parse_from(argv)
        .unwrap_or_else(|error| panic!("解析 {argv:?} 失败：{error}"))
        .http
}

#[tokio::test]
async fn the_token_file_is_mode_0600() {
    let dir = scratch("mode");
    let token_file = dir.join("mcp-token");
    let cancellation = CancellationToken::new();
    let bound = HttpServer::bind(&ephemeral(&token_file), cancellation.clone())
        .await
        .expect("绑定服务");
    assert_eq!(bound.token_path(), token_file.as_path());
    assert_mode_0600(&token_file);

    // 已存在但过宽的文件必须被收紧，而不是被静默接受：一个 0644 的 token 文件与没有认证等价。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o644))
            .expect("放宽权限");
        let _reloaded = BearerToken::load_or_create(&token_file).expect("读回 token");
        assert_mode_0600(&token_file);
    }

    cancellation.cancel();
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn assert_mode_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let mode = std::fs::metadata(path)
        .expect("读取 token 文件权限")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode,
        yunjian_mcp::http::token::TOKEN_FILE_MODE,
        "token 文件权限必须恰好是 0600，实为 {mode:04o}"
    );
}

#[cfg(not(unix))]
fn assert_mode_0600(path: &Path) {
    assert!(path.is_file(), "token 文件必须存在");
}

#[tokio::test]
async fn the_cancellation_token_stops_the_server_within_two_seconds() {
    let dir = scratch("shutdown");
    let running = Running::start(&dir, YunjianServer::without_corpus()).await;
    let endpoint = running.endpoint.clone();

    let started = std::time::Instant::now();
    running.stop().await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "关停耗时 {elapsed:?}，超过 2 秒"
    );

    let after = reqwest::Client::default()
        .post(&endpoint)
        .body("{}")
        .send()
        .await;
    assert!(after.is_err(), "关停后端口必须不再接受连接");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_token_never_reaches_stdout_or_stderr() {
    if std::env::var_os(CHILD_MARKER).is_some() {
        child_serves_one_session().await;
        return;
    }

    let dir = scratch("capture");
    let token_file = dir.join("mcp-token");
    let output = std::process::Command::new(std::env::current_exe().expect("定位测试可执行文件"))
        .args(["--exact", CHILD_TEST_NAME, "--nocapture"])
        .env(CHILD_MARKER, "1")
        .env(CHILD_TOKEN_FILE, &token_file)
        .env("RUST_LOG", "trace")
        .output()
        .expect("运行子进程");

    let secret = std::fs::read_to_string(&token_file)
        .expect("子进程必须已写出 token 文件")
        .trim()
        .to_owned();
    assert_eq!(secret.len(), 64, "token 必须是 64 个十六进制字符");
    assert!(
        output.status.success(),
        "子进程必须成功结束；stderr：{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(&secret),
        "token 出现在了 stdout 里；这条流会被 MCP 客户端当协议流读"
    );
    assert!(
        !stderr.contains(&secret),
        "token 出现在了 stderr 里；客户端会把这条流整条收进它的日志文件"
    );

    // 反向断言：这条 grep 真的能在两条流里找到东西。少了它，一个「secret 取错了、于是永远
    // 找不到」的实现也会让上面两条绿着。
    let fingerprint = BearerToken::load_or_create(&token_file)
        .expect("读回 token")
        .fingerprint();
    assert!(
        stderr.contains(&fingerprint),
        "日志里必须有指纹（证明捕获到了启动日志、且断言的目标流是对的）；stderr：{stderr}"
    );
    assert!(
        stderr.contains(&token_file.display().to_string()),
        "日志里必须有 token 文件路径"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 子进程：起服务、用真实客户端握手并调一次工具、再关停。
///
/// 三个动作缺一不可——泄露可能发生在启动日志、请求日志或关停日志任意一处，只起不连的子进程
/// 覆盖不到后两者。
async fn child_serves_one_session() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let token_file = PathBuf::from(std::env::var_os(CHILD_TOKEN_FILE).expect("token 文件路径"));
    let cancellation = CancellationToken::new();
    let bound = HttpServer::bind(&ephemeral(&token_file), cancellation.clone())
        .await
        .expect("子进程绑定服务");
    let endpoint = bound.endpoint();
    let secret = std::fs::read_to_string(&token_file)
        .expect("子进程读回 token")
        .trim()
        .to_owned();
    let task = tokio::spawn(async move { bound.serve(YunjianServer::without_corpus()).await });

    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(endpoint).auth_header(secret),
    );
    let client = ().serve(transport).await.expect("子进程握手");
    let tools = client.list_all_tools().await.expect("子进程 tools/list");
    assert!(!tools.is_empty());
    let _ = client
        .call_tool(
            CallToolRequestParams::new("search_poem").with_arguments(
                serde_json::json!({"query": "明月"})
                    .as_object()
                    .cloned()
                    .expect("参数对象"),
            ),
        )
        .await
        .expect("子进程 tools/call");
    client.cancel().await.expect("子进程关闭客户端");

    cancellation.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
}
