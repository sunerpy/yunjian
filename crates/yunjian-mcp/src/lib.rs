//! 云笺 MCP stdio 服务端。
//!
//! 服务启动不以语料可用为前提：缺语料时仍完成握手并列出工具，调用工具则返回可见的
//! `corpus_missing` 结构化错误。stdout 只承载换行分隔的 JSON-RPC 协议帧。

#![warn(missing_docs)]

use rmcp::{
    ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter, model::CallToolResult,
    tool, tool_handler, tool_router, transport::stdio,
};
use yunjian_core::Yunjian;

#[derive(Debug, Clone)]
enum CoreClient {
    Ready(Yunjian),
    Missing,
}

/// 通过 MCP 暴露云笺核心能力的服务端。
#[derive(Debug, Clone)]
pub struct YunjianServer {
    core: CoreClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl YunjianServer {
    /// 使用已就绪的只读核心客户端创建服务端。
    #[must_use]
    pub fn new(core: Yunjian) -> Self {
        Self {
            core: CoreClient::Ready(core),
            tool_router: Self::tool_router(),
        }
    }

    /// 创建可完成握手但会报告缺少语料的服务端。
    #[must_use]
    pub fn without_corpus() -> Self {
        Self {
            core: CoreClient::Missing,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "search_poem",
        description = "检索云笺诗词语料；本版本先建立 MCP 路由，完整检索参数将在后续版本提供。"
    )]
    async fn search_poem(&self) -> CallToolResult {
        match &self.core {
            CoreClient::Ready(core) => {
                let _ = core.clone();
                CallToolResult::structured(serde_json::json!({
                    "status": "ready"
                }))
            }
            CoreClient::Missing => CallToolResult::structured_error(serde_json::json!({
                "code": "corpus_missing",
                "message": "尚无可用的云笺语料库",
                "hint": "运行 `yunjian corpus fetch` 获取语料后重新启动 MCP 服务"
            })),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for YunjianServer {}

/// 在当前进程的 stdin/stdout 上运行服务，直到客户端关闭输入流。
///
/// # Errors
///
/// 初始化 MCP 会话或等待服务任务结束失败时返回错误。
pub async fn serve_stdio(server: YunjianServer) -> anyhow::Result<()> {
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
