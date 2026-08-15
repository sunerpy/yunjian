# 开发流程

需要 Rust 1.95+。`rust-toolchain.toml` 默认跟随 stable，CI 另用精确的 Rust 1.95 执行最低版本
编译门禁。

## 三条命令

```bash
make hooks   # 装 pre-commit（提交时格式化）与 pre-push（推送前跑 make ci）钩子
make ci      # 唯一门禁：格式检查 + clippy + 测试 + MCP 一致性 + 前端测试
make help    # 列出所有 target
```

`cargo run -p xtask -- verify-sources --offline` 离线校验上游语料源逐资产的许可与摘要，
退出码 0 说明环境是对的。

## 需要前置工件的命令

有几条 xtask 子命令消费的是 gitignored 的大工件，新鲜检出里不存在。它们**不会裸退出**，
而会点名缺什么、该先跑哪一条：

| 命令                          | 前置工件                                                  | 怎么补                                                 |
| ----------------------------- | --------------------------------------------------------- | ------------------------------------------------------ |
| `xtask corpus-package`        | `corpus/build/release/corpus.db` 与同批 `corpus-audit.db` | 先跑 `xtask corpus-build`，或取 `corpus-v*` release    |
| `xtask pregenerate`           | 同上（只读打开）；`dataset/README.md` 披露段              | 同上                                                   |
| `xtask provider-calls`        | 同上（只读打开）                                          | 同上                                                   |
| `xtask corpus-build`          | 三份上游检出（约 833 MB）                                 | 按 `corpus/sources.toml` 的锁定 SHA 浅克隆             |
| `cargo test -p yunjian-voice` | `models/cache/<模型名>`                                   | `yunjian models fetch <模型名>` 或 `YUNJIAN_MODEL_DIR` |

语料构建实测约 9 分钟（release 构建，32 核），两次独立构建的 `corpus.db` SHA-256 完全相同——
构建是确定性的。缺模型时受影响的语音测试会显示为 `ignored` 并带明确原因，不会伪装通过。

## 发布

正式发布还需要仓库 Actions secrets `TAURI_SIGNING_PRIVATE_KEY` 和
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。任一 updater 签名缺失或与应用内公钥不匹配时，流程都会
失败并让 GitHub Release 保持 draft；**不要通过关闭签名绕过门禁。**

## CI runner 现状

Linux x86_64 作业已记录 AWS CodeBuild runner `yunjian-runner` 的迁移标签，但当前 CodeConnections
连接 `yunjian-github` 仍为 `PENDING`。在 AWS 控制台授权连接并重建 `WORKFLOW_JOB_QUEUED` webhook
前，所有 job 继续使用 GitHub-hosted runner。

Windows、macOS、Linux ARM 和混合平台矩阵**不迁移**到这个 Linux x86_64 项目；启用 CodeBuild 时
每个 job 还必须保留自己的唯一第二标签，避免 GitHub 的 superset 标签匹配把作业路由到错误 runner。

## 约定

- **提交信息用中文祈使句 + Conventional Commits**，例如 `feat(core): 添加韵部检索`。
- **不许 `println!`**。日志走 `tracing` 到 stderr 与滚动文件；同一个二进制要托管 MCP stdio
  服务器，stdout 上一行杂音就会毁掉协议流。
- **密钥不进 `config.toml`**。顶层配置开了 `deny_unknown_fields`，粘一个 `api_key` 进去会直接
  报错而不是被静默丢弃。
- **绝不摄入第三方现代注释、译文或赏析**，理由见 [内容来源与许可](PROVENANCE.zh.md) 与
  `corpus/DENYLIST.md`。
- **两份 README 有行数上限**（各 230 行），由
  `crates/yunjian-corpus/tests/docs_completeness.rs` 断言。要加长内容就往 `docs/` 放，
  README 只留导航——**不许为了省行数把「尚未实现」写成已实现。**

Issue 与 PR 都欢迎。改动语料源之前先读 [语料与索引](CORPUS.zh.md)。
