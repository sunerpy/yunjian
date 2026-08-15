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

## 写验收命令：退出码 0 不等于验收通过

凡验收命令产出机器可读结果（JSON / manifest / 报告），**必须断言那个结果里的语义字段**，
只看退出码会得到一条永远通过的命令。本仓库两次栽在这上面：

- `xtask corpus-measure --scale 10k` 退出 0，而报告把 10k 标成 `state=not_measured`——
  失败还顺手把原本真实的实测行降级掉了。现在被请求的规模没测出来即**非零退出且不写报告**。
- 一条形如 `gh release view … --jq '.x'` 的验收 shell 判「语料是否已发布」为通过，
  而仓库里一个 Release、一个 tag 都没有。

四种最常见的假绿形态，以及正确写法：

| 形态                              | 为什么绿                           | 正确写法                                                   |
| --------------------------------- | ---------------------------------- | ---------------------------------------------------------- |
| `gh … --jq '.ok'`                 | `--jq` 对 `false` 与 `null` 都退 0 | `test "$(gh … --jq '.ok')" = true`，或落盘后 `jq -e`       |
| `jq '.ok' f.json`                 | 打印 `false` 也退 0                | `jq -e '.ok' f.json > /dev/null`                           |
| `cmd 2>&1 \| tee log`             | 管道退出码是 `tee` 的              | `set -o pipefail`（GitHub 的 `shell: bash` 已自带）        |
| `gh release view --json isLatest` | 字段不存在，报的是无关错误         | 用受支持字段；`latest` 走 `gh api repos/…/releases/latest` |

这四条由 `xtask/tests/acceptance_semantics.rs` 扫真实 workflow 与 `Makefile` 机械守住：
`gh --jq` 的结果必须被比较消费、`--json` 字段必须在实测支持列表内、每处 `| tee` 必须有
pipefail、`corpus-release.yml` 必须在发布后**把资产下载回来重算 SHA-256**。

## 本机打包桌面安装包

```bash
make bundle   # deb / rpm / AppImage，打完逐类核对产物；约 12 分钟，需 4 GiB 可用磁盘
```

它**刻意不签名**（`--no-sign`）。`tauri.conf.json` 声明了 `plugins.updater.pubkey`，于是裸
`cargo tauri build` 的最后一步必然要签名，缺 `TAURI_SIGNING_PRIVATE_KEY` 就报
`A public key has been found, but no private key` 并整体退出 1——而**三个安装包此时已经全部
产出**。这个失败形态极易被读成「打包坏了」（F1 审计就据此把 AppImage 阶段判成失败），所以本机
入口不走签名这一步，签名只在发布流程里做。

它同时带 `-v`：tauri-bundler 默认 `log_level=Error`，那条分支会吞掉 linuxdeploy 的 stderr，
只抛一句 `failed to run linuxdeploy`，缺库、磁盘满、插件失败三种原因都长一个样。

产物核对不是装饰：**Linux updater 只消费 AppImage，`.deb` 不能自动更新**，所以少一个 AppImage
是断掉 Linux 自动更新链，不是少一个可选格式。取值域由
`crates/yunjian-app/tests/bundle_targets.rs` 自锁并扫真实 `Makefile`。

## 发布

正式发布还需要仓库 Actions secrets `TAURI_SIGNING_PRIVATE_KEY` 和
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。任一 updater 签名缺失或与应用内公钥不匹配时，流程都会
失败并让 GitHub Release 保持 draft；**不要通过关闭签名绕过门禁。** 上面 `make bundle` 的
`--no-sign` 只属于本机验证入口，发布路径里出现它就是漏洞，由同一份守卫测试扫 workflow 拦住。

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
