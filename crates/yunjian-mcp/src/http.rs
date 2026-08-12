//! 可选的 Streamable HTTP 传输：`yunjian mcp --http`。
//!
//! # 它与 stdio 的差别只有一条，但那一条决定了本模块的全部形状
//!
//! stdio 传输没有监听端口：谁能读写那两个管道，由启动它的进程决定，操作系统替我们做了
//! 全部访问控制。HTTP 传输一开就有一个 socket，**即使只绑在 `127.0.0.1` 上，同一台机器上
//! 任何进程都能连**，而一张恶意网页也能从浏览器里对 `http://127.0.0.1:<port>/mcp` 发请求。
//! 所以这里的每一层都不是可选的加固，而是把 stdio 免费得到的那份隔离手工补回来：
//!
//! | 层 | 挡住什么 |
//! |---|---|
//! | 默认绑 `127.0.0.1`，非回环需 `--allow-remote` | 一次手滑把诗库暴露到局域网 |
//! | 每个请求必须带 `Authorization: Bearer` | 同机的其它进程与用户 |
//! | `Origin` 白名单 | 浏览器页面发起的跨站请求 |
//! | `Host` 白名单（由 `rmcp` 提供） | DNS rebinding |
//!
//! # token 只经文件传递
//!
//! 见 [`token`]：启动时生成，以 0600 写入 `$XDG_RUNTIME_DIR/yunjian/mcp-token`，日志里
//! **只有路径和一个不可逆指纹**。没有 `--token` 参数，也不读环境变量。

pub mod token;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use token::BearerToken;
use tokio_util::sync::CancellationToken;

use crate::YunjianServer;

/// MCP 端点在 HTTP 路由里的挂载路径。
pub const MCP_PATH: &str = "/mcp";

/// 默认监听端口。
pub const DEFAULT_PORT: u16 = 8765;

/// 默认监听地址：**只有回环**。
pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT);

/// 拒绝绑定非回环地址时的进程退出码。
///
/// 与 CLI 的「用法错误」同码：这是调用方要改命令，而不是环境缺东西。
pub const EXIT_REMOTE_BIND_REFUSED: i32 = 2;

/// 拒绝对外提供服务时的进程退出码。
pub const EXIT_HTTP_FAILED: i32 = 2;

/// `yunjian mcp` 的 HTTP 传输开关。
///
/// **定义在这里而不是 `yunjian-cli`，是为了让「有哪些 flag」与「flag 的守卫」在同一处、
/// 被同一个测试断言。** 守卫和它保护的参数分居两个 crate 时，「参数还在、守卫被删了」
/// 与「新加了一个绕过守卫的参数」两种回归都不会有任何一个测试变红。
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct HttpOptions {
    /// 改用 Streamable HTTP 传输（默认走 stdio）。
    #[arg(long)]
    pub http: bool,

    /// 监听地址。默认只监听回环。
    #[arg(long, value_name = "ADDR", default_value_t = DEFAULT_BIND)]
    pub bind: SocketAddr,

    /// 确认要监听非回环地址。
    ///
    /// 不带这个开关时，任何非回环的 `--bind` 都会被拒绝并退出。
    #[arg(long)]
    pub allow_remote: bool,

    /// token 文件路径。已存在则读取，不存在则在该路径生成。
    ///
    /// 缺省为 `$XDG_RUNTIME_DIR/yunjian/mcp-token`，回落到应用数据目录。
    /// **没有 `--token`**：命令行参数在同机对任何用户可见。
    #[arg(long, value_name = "PATH")]
    pub token_file: Option<PathBuf>,
}

/// 拒绝一次非回环绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBindRefused {
    /// 被拒绝的地址。
    pub addr: SocketAddr,
}

impl fmt::Display for RemoteBindRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "拒绝把 MCP 服务绑定到非回环地址 {}：这会让同网段的任何机器都能连上你的诗库。\
             确实需要请显式加 --allow-remote，并确保 token 文件只有你能读",
            self.addr
        )
    }
}

impl std::error::Error for RemoteBindRefused {}

impl HttpOptions {
    /// 校验绑定地址与远程许可是否匹配。
    ///
    /// # Errors
    ///
    /// `bind` 不是回环地址而 `allow_remote` 为假时返回 [`RemoteBindRefused`]。
    pub fn check_bind(&self) -> Result<(), RemoteBindRefused> {
        if self.allow_remote || self.bind.ip().is_loopback() {
            return Ok(());
        }
        Err(RemoteBindRefused { addr: self.bind })
    }

    /// 解析出这次要用的 token 文件路径。
    ///
    /// # Errors
    ///
    /// 未给出 `--token-file` 且平台既没有运行时目录也没有数据目录时返回错误。
    pub fn resolve_token_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.token_file {
            return Ok(path.clone());
        }
        token::default_token_path().ok_or_else(|| {
            anyhow::anyhow!(
                "无法确定 token 文件位置：本平台既没有 XDG_RUNTIME_DIR 也没有数据目录，\
                 请用 --token-file 指定"
            )
        })
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            http: false,
            bind: DEFAULT_BIND,
            allow_remote: false,
            token_file: None,
        }
    }
}

/// 已经占好端口、已经备好 token，但还没开始收请求的 HTTP 服务。
///
/// 分成 `bind` 与 `serve` 两步是为了让调用方（以及测试）在真正开始服务前拿到**实际**监听
/// 地址：`--bind 127.0.0.1:0` 时端口由内核分配，而客户端必须知道那个端口。
pub struct HttpServer {
    listener: tokio::net::TcpListener,
    local_addr: SocketAddr,
    token: Arc<BearerToken>,
    token_path: PathBuf,
    allow_remote: bool,
    cancellation: CancellationToken,
}

impl fmt::Debug for HttpServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpServer")
            .field("local_addr", &self.local_addr)
            .field("token_path", &self.token_path)
            .field("allow_remote", &self.allow_remote)
            .finish_non_exhaustive()
    }
}

impl HttpServer {
    /// 校验参数、占好端口、准备 token 文件。
    ///
    /// # Errors
    ///
    /// 参数被 [`HttpOptions::check_bind`] 拒绝、端口占不上，或 token 文件读写失败时返回错误。
    pub async fn bind(
        options: &HttpOptions,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Self> {
        options.check_bind()?;
        let token_path = options.resolve_token_path()?;
        let token = BearerToken::load_or_create(&token_path)?;

        let listener = tokio::net::TcpListener::bind(options.bind)
            .await
            .map_err(|error| anyhow::anyhow!("监听 {} 失败：{error}", options.bind))?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| anyhow::anyhow!("读取实际监听地址失败：{error}"))?;

        if options.allow_remote && !local_addr.ip().is_loopback() {
            tracing::warn!(
                bind = %local_addr,
                "MCP 正在监听非回环地址：同网段任何机器都能连上，唯一的门是 bearer token"
            );
        }
        tracing::info!(
            bind = %local_addr,
            path = MCP_PATH,
            token_file = %token_path.display(),
            token_fingerprint = %token.fingerprint(),
            "MCP Streamable HTTP 服务已就绪；token 只在该文件里，日志不含它"
        );

        Ok(Self {
            listener,
            local_addr,
            token: Arc::new(token),
            token_path,
            allow_remote: options.allow_remote,
            cancellation,
        })
    }

    /// 实际监听地址。`--bind ...:0` 时这里才是内核分配的端口。
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// token 文件路径。
    #[must_use]
    pub fn token_path(&self) -> &Path {
        &self.token_path
    }

    /// 客户端应当使用的端点 URL。
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("http://{}{MCP_PATH}", self.local_addr)
    }

    /// 开始收请求，直到 cancellation token 被取消。
    ///
    /// # Errors
    ///
    /// 服务循环异常结束时返回错误。
    pub async fn serve(self, server: YunjianServer) -> anyhow::Result<()> {
        let Self {
            listener,
            local_addr,
            token,
            token_path: _,
            allow_remote,
            cancellation,
        } = self;

        let allowed_origins = allowed_origins(local_addr);
        // `rmcp` 自己也做 Origin 与 Host 校验，这里同样喂给它：两层挡同一件事是刻意的，
        // 但**不能只靠它**——`allowed_origins` 默认是空列表，而空列表在 `rmcp` 里的含义是
        // 「不校验 Origin」，一次无心的默认值改动就会静默关掉整条防线。
        let mut config = StreamableHttpServerConfig::default()
            .with_cancellation_token(cancellation.child_token())
            .with_allowed_origins(allowed_origins.clone());
        config = if allow_remote {
            // 显式远程服务时无法枚举运维方会用的主机名，而 DNS rebinding 防的是浏览器，
            // 浏览器又已被 bearer token 挡住，所以这里放开 Host 白名单而不是猜一个。
            config.disable_allowed_hosts()
        } else {
            config.with_allowed_hosts([
                "localhost".to_owned(),
                "127.0.0.1".to_owned(),
                "::1".to_owned(),
                local_addr.to_string(),
            ])
        };

        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let guard = Arc::new(Guard {
            token,
            allowed_origins,
        });
        let router = Router::new()
            .nest_service(MCP_PATH, service)
            .layer(middleware::from_fn_with_state(guard, authorize));

        let shutdown = cancellation.clone();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
            .map_err(|error| anyhow::anyhow!("MCP HTTP 服务异常结束：{error}"))?;
        tracing::info!(bind = %local_addr, "MCP Streamable HTTP 服务已停止");
        Ok(())
    }
}

/// 允许的 `Origin` 值。
///
/// 只有浏览器会发 `Origin`，而唯一合法的浏览器来源是这台机器上的这个端口本身。任何其它
/// 来源都是跨站请求。
fn allowed_origins(addr: SocketAddr) -> Vec<String> {
    let port = addr.port();
    let mut origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ];
    let literal = format!("http://{addr}");
    if !origins.contains(&literal) {
        origins.push(literal);
    }
    origins
}

struct Guard {
    token: Arc<BearerToken>,
    allowed_origins: Vec<String>,
}

/// 唯一的准入检查：先看 `Origin`，再看 `Authorization`。
///
/// **顺序是刻意的。** 跨站请求即使带对了 token 也要拒——那意味着 token 已经泄露给了一张
/// 网页，此时放行等于把泄露扩大成一次完整的数据读取。反过来，先查 token 会让「Origin 检查
/// 是否独立生效」无法单独验证。
async fn authorize(State(guard): State<Arc<Guard>>, request: Request, next: Next) -> Response {
    if let Some(rejection) = reject_origin(request.headers(), &guard.allowed_origins) {
        return rejection;
    }
    match bearer(request.headers()) {
        Some(presented) if guard.token.matches(presented) => next.run(request).await,
        Some(_) => unauthorized("bearer token 不匹配"),
        None => unauthorized("缺少 Authorization: Bearer <token> 头"),
    }
}

fn reject_origin(headers: &HeaderMap, allowed: &[String]) -> Option<Response> {
    let origin = headers.get(header::ORIGIN)?;
    let Ok(origin) = origin.to_str() else {
        return Some(forbidden("Origin 头不是合法 UTF-8"));
    };
    if allowed
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(origin))
    {
        return None;
    }
    tracing::warn!(origin, "拒绝跨站 Origin 的 MCP 请求");
    Some(forbidden("该 Origin 不在允许列表内"))
}

/// 取出 `Authorization: Bearer <token>` 里的 token。
///
/// 方案名（`Bearer`）按 RFC 7235 大小写不敏感比较；token 本身原样取出，不做 trim 之外的
/// 加工——把 `Bearer  x` 里的多余空格当成 token 的一部分才是正确行为，宽容解析只会让
/// 「服务端认为的 token」和「客户端发出的 token」出现两套口径。
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

fn unauthorized(reason: &'static str) -> Response {
    tracing::warn!(reason, "拒绝未认证的 MCP 请求");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"yunjian-mcp\"")],
        reason,
    )
        .into_response()
}

fn forbidden(reason: &'static str) -> Response {
    (StatusCode::FORBIDDEN, reason).into_response()
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BIND, EXIT_REMOTE_BIND_REFUSED, HttpOptions, allowed_origins, bearer};
    use axum::http::{HeaderMap, HeaderValue, header};
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn the_default_bind_is_loopback() {
        assert!(DEFAULT_BIND.ip().is_loopback());
        assert_eq!(DEFAULT_BIND.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(HttpOptions::default().bind, DEFAULT_BIND);
        assert!(HttpOptions::default().check_bind().is_ok());
    }

    #[test]
    fn a_non_loopback_bind_needs_the_flag() {
        let wildcard = HttpOptions {
            bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8000)),
            ..HttpOptions::default()
        };
        let refusal = wildcard.check_bind().expect_err("必须拒绝");
        assert!(
            refusal.to_string().contains("--allow-remote"),
            "拒绝理由必须点名那个开关，实为 {refusal}"
        );
        assert_ne!(EXIT_REMOTE_BIND_REFUSED, 0);

        let allowed = HttpOptions {
            allow_remote: true,
            ..wildcard
        };
        assert!(
            allowed.check_bind().is_ok(),
            "显式许可后同一个地址必须放行，否则这条守卫无从关掉"
        );
    }

    #[test]
    fn the_bearer_scheme_is_case_insensitive_but_the_token_is_not_guessed() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), None);

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer abc"),
        );
        assert_eq!(bearer(&headers), Some("abc"));

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("BEARER abc"),
        );
        assert_eq!(bearer(&headers), Some("abc"));

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer(&headers), None, "其它认证方案不得被当成 bearer");

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer   "));
        assert_eq!(bearer(&headers), None, "空 token 不得被当成有效凭据");

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("abc"));
        assert_eq!(bearer(&headers), None, "缺方案名的裸 token 不得被接受");
    }

    #[test]
    fn the_origin_allowlist_covers_this_port_only() {
        let origins = allowed_origins(SocketAddr::from((Ipv4Addr::LOCALHOST, 4321)));
        assert!(origins.contains(&"http://127.0.0.1:4321".to_owned()));
        assert!(origins.contains(&"http://localhost:4321".to_owned()));
        assert!(
            !origins.iter().any(|origin| origin.contains("4322")),
            "同机另一个端口也是跨站来源：{origins:?}"
        );
        assert!(!origins.iter().any(|origin| origin == "null"));
    }
}
