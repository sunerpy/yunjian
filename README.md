简体中文 · [English](docs/readme/README.en.md)

# 云笺 yunjian

离线可用的中国古典诗词工具：本地 SQLite 语料库、检索、AI 赏析、背诵训练，以及一个让 AI 助手直接查你诗库的 MCP 服务器。

[![CI](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml/badge.svg)](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> [!IMPORTANT]
> **本项目处于早期开发阶段，还没有可用的发布版本。** 下面的「功能概览」描述的是目标形态；
> 当前每一项的真实进度见 [项目状态](#项目状态)。不要指望 `cargo install` 之后就能得到一个能查诗的程序。

## 目录

- [项目状态](#项目状态)
- [为什么是这个设计](#为什么是这个设计)
- [快速开始](#快速开始)
- [功能概览](#功能概览)
- [内容来源与许可](#内容来源与许可)
- [文档](#文档)
- [参与开发](#参与开发)

## 项目状态

`main` 上**已经实现并有测试守着**的部分：

| 模块         | 状态   | 说明                                                                                                       |
| ------------ | ------ | ---------------------------------------------------------------------------------------------------------- |
| 工作区骨架   | 已实现 | 8 个 crate 的 Cargo workspace，依赖版本集中锁定                                                            |
| 配置与日志   | 已实现 | 运行时发现 `config.toml`；tracing 日志强制走 stderr 与滚动文件                                             |
| stdout 门禁  | 已实现 | clippy 全仓禁止 `println!` 与 `std::io::stdout`，只放行一处 CLI 出口                                       |
| 许可门禁     | 已实现 | `xtask verify-sources` 逐资产校验上游许可与 SHA-256                                                        |
| 语料记录模型 | 已实现 | 规范化记录与 append-only `stable_id` 注册表                                                                |
| 韵书导入     | 已实现 | 平水韵与词林正韵入库，逐字反向索引构建期推导；中华新韵与词谱扣留                                           |
| 索引选型     | 已实测 | FTS5 `detail` 模式与 n-gram 辅助表由实测定案，见 [语料与索引](docs/CORPUS.zh.md)                           |
| 语料工件     | 已实现 | 随包唐宋 47 万首（gzip 211 MiB），检索结构首启本机派生；打包带六条中止断言                                 |
| 语音构建     | 已验证 | 五平台原生依赖构建与链接路径实测，见 [语音构建](docs/VOICE-BUILD.zh.md)                                    |
| 麦克风与底线 | 已验证 | Linux 实测采集 16 kHz 单声道；各平台授权链与系统底线见 [平台要求](docs/PLATFORM-REQUIREMENTS.zh.md)        |
| 模型许可     | 已核实 | 逐模型核实，只放行 MIT / Apache-2.0；`xtask verify-models` 强制，见 `models.toml`                          |
| 文言字准     | 已实测 | 合成音加增强测得，结论是字准只作参考值，见 [CER 报告](docs/reports/asr-cer.md)                             |
| 核心检索     | 已实现 | 正文、题目、作者、朝代、首句、尾字、标签、韵部检索与作品详情，统一收在 `Yunjian` 门面后                    |
| 命令行       | 已实现 | `yunjian search/show/author/rhyme/corpus`，`--json` 稳定信封与 0/1/2/3 退出码，见 [命令行](docs/CLI.zh.md) |

**尚未实现**（有完整方案，但一行产品代码都还没有）：AI 赏析、背诵训练与 FSRS 复习、
离线朗读与语音识别、MCP 服务器（`yunjian mcp` 目前只有占位）、Tauri 桌面端、移动端。

## 为什么是这个设计

一句话：**版权墙决定了架构。**

现存的开源诗词数据集里，凡是带现代注释、译文、赏析的，授权链都立不住——抓自商业站点、
标注「仅供学术」、或者仓库级 LICENSE 根本盖不住它转录的内容。落地校验时，这条规则在一个
MIT 仓库**内部**就命中了 10 个夹带现代白话注释的文件。

所以云笺只做三件事的组合：

1. **公有领域原文**——底本为前现代作品，已过保护期；
2. **逐条注明出处的前现代集评**——宋人评唐诗本身就是公有领域；
3. **明确标注的 AI 赏析**——用你自己的 API key 生成，永远带「AI 赏析」标签。

AI 功能不是加分项，它是版权墙留下的那个洞的唯一合法填法。

## 快速开始

> 当前只能从源码构建，且构建出来的二进制还没有可用的子命令。这一节现在是**开发者的**快速开始。

需要 Rust 1.88+（仓库里有 `rust-toolchain.toml`，`rustup` 会自动装对版本）：

```bash
git clone https://github.com/sunerpy/yunjian.git
cd yunjian
cargo build --workspace
```

跑一遍与 CI 相同的门禁（格式检查 + clippy + 测试）：

```bash
make ci
```

校验上游语料源的许可与摘要（离线模式，不联网）：

```bash
cargo run -p xtask -- verify-sources --offline
```

看到每个数据源逐资产的许可判定，且退出码为 0，就说明环境是对的。

## 功能概览

以下是**目标**形态。当前进度一律以 [项目状态](#项目状态) 为准。

- **离线检索**。语料是一个只读 SQLite 文件，不联网、不登录、不需要账号。FTS5 trigram
  索引配一张 1/2 字 n-gram 候选表——因为「明月」这种两字查询占绝大多数，而 trigram
  在三字以下推不出任何约束。默认随包唐宋 47 万首（下载 211 MiB），全量 90 万首作为
  应用内可选下载；检索结构在**首启本机派生**（实测约 10 分钟），派生后检索性能与随包
  时完全相同。理由与实测见 [语料与索引](docs/CORPUS.zh.md)。
- **AI 赏析（自带 key）**。key 存在操作系统钥匙串里，不走环境变量、不进配置文件。
  没有 key 也完整可用：常见名篇随包带一批预生成的赏析。
- **背诵训练**。挖空、首字提示、遮挡三种打字模式共用同一个评分内核；语音练习额外给
  完整度与流畅度。语音路径的字准率**永远只是估计值，不是分数**——文言的语音识别不够
  可靠到能给你打分。
- **朗读**。逐音步合成 + Rust 侧插静音，用项目自己的公有领域来源破读表处理古音；
  覆盖范围外回落到现代普通话读音。
- **MCP 服务器**。`yunjian mcp` 跑在 stdio 上，让 Claude Desktop、OpenCode 这类客户端
  直接查你的诗库。生成类工具的输出标注「AI 生成，非古人作品」，永远不写回语料。
- **多端**。桌面端 Tauri v2 + React；命令行工具带机器可读输出；移动端框架由真机实测决定。

## 内容来源与许可

**代码**按 [MIT](LICENSE) 授权。

**随包语料只包含公有领域原文，以及由 MIT 许可的上游整理产物。** 逐资产的许可判定写在
[`corpus/sources.toml`](corpus/sources.toml) 里，由 `xtask verify-sources` 强制校验；被拒绝的
数据集连同拒绝理由记在 [`corpus/DENYLIST.md`](corpus/DENYLIST.md)。判定粒度是**单个文件**
而不是仓库——一个仓库的 MIT LICENSE 不能为它抓来的内容授权。

**赏析文本是 AI 生成的，不是学术成果。** 界面里它与有出处的集评在视觉上分开呈现，并附带
「未经人工审校」的说明。AI 生成的诗永远标注「AI 生成，非古人作品」，永远不进语料库和赏析表。

**关于语音功能的许可，请注意：**

- `voice` cargo 特性**默认关闭**。默认构建是纯 MIT，实测不链接任何 onnxruntime。
- 启用 `voice` 后，预编译的 sherpa-onnx 产物**静态包含 GPL-3.0 的 espeak-ng**
  （实测 50 个 `espeak_*` 导出符号）。MIT 单向兼容 GPL-3.0，所以这不是许可冲突，
  但**分发一份开启语音的云笺，整体须按 GPL-3.0 条款提供**。
- 因此发布产物分两种：默认构建标 MIT，语音构建标 GPL-3.0。细节见
  [语音构建](docs/VOICE-BUILD.zh.md)。

不会随包分发任何模型权重。语音模型按需下载，且只接受经核实的 MIT 或 Apache-2.0 许可——
逐模型的判定、证据文件与摘要在 [`models.toml`](models.toml)，由 `xtask verify-models`
强制校验；被拒的模型连同理由在 [`models/DENYLIST.md`](models/DENYLIST.md)。核实推翻了一条
先前的判断：**FunASR 系（SenseVoice / Paraformer）走的是阿里自家的许可协议，不是 MIT 也
不是 Apache-2.0**，因此离线识别只剩 Whisper 一族可用。

**语音路径上的逐字准确率永远只是参考值，不是分数。** 这不是保守措辞，而是实测结论：
见 [CER 报告](docs/reports/asr-cer.md)。

## 文档

- [架构](docs/ARCHITECTURE.zh.md)——分层、`yunjian-core` 为什么不知道 Tauri 存在、移动端逃生通道
- [语料与索引](docs/CORPUS.zh.md)——构建管线、`stable_id` 身份模型、FTS5 索引选型实测
- [命令行](docs/CLI.zh.md)——子命令、`--json` 信封 schema、四个退出码、stdout/stderr 分工
- [语音构建](docs/VOICE-BUILD.zh.md)——五平台原生依赖构建、链接方式、许可影响
- [平台要求](docs/PLATFORM-REQUIREMENTS.zh.md)——五平台系统最低版本、麦克风授权链、低于底线时的降级行为
- [CER 报告](docs/reports/asr-cer.md)——文言语音识别字准实测，以及它为什么只能当参考值

## 参与开发

```bash
make hooks   # 装 pre-commit（提交时格式化）与 pre-push（推送前跑 make ci）钩子
make help    # 列出所有 target
```

约定：

- **提交信息用中文祈使句 + Conventional Commits**，例如 `feat(core): 添加韵部检索`。
- **不许 `println!`**。日志走 `tracing` 到 stderr 与滚动文件；同一个二进制要托管 MCP
  stdio 服务器，stdout 上一行杂音就会毁掉协议流。
- **密钥不进 `config.toml`**。顶层配置开了 `deny_unknown_fields`，粘一个 `api_key` 进去
  会直接报错而不是被静默丢弃。
- **绝不摄入第三方现代注释、译文或赏析**，理由见上文和 `corpus/DENYLIST.md`。

Issue 与 PR 都欢迎。改动语料源之前先读 [语料与索引](docs/CORPUS.zh.md)。
