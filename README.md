简体中文 · [English](docs/readme/README.en.md)

# 云笺 yunjian

离线可用的中国古典诗词工具：本地 SQLite 语料库、检索、AI 赏析、背诵训练，以及一个让 AI 助手直接查你诗库的 MCP 服务器。

[![CI](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml/badge.svg)](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> [!IMPORTANT]
> **本项目处于早期开发阶段，还没有可用的发布版本。** 下面的「功能概览」描述的是目标形态，当前
> 每一项的真实进度见 [项目状态](#项目状态)。不要指望 `cargo install` 就能得到一个能查诗的程序。

## 目录

- [项目状态](#项目状态)
- [快速开始](#快速开始)
- [与 LLM 协作](#与-llm-协作)
- [功能概览](#功能概览)
- [内容来源与许可](#内容来源与许可)
- [文档](#文档)
- [参与开发](#参与开发)

## 项目状态

`main` 上**已经实现并有测试守着**的部分：

| 模块        | 状态       | 说明                                                                           |
| ----------- | ---------- | ------------------------------------------------------------------------------ |
| 工程底座    | 已实现     | 8 crate workspace、依赖集中锁定、`config.toml` 运行时发现、tracing 只走 stderr |
| stdout 门禁 | 已实现     | clippy 全仓禁止 `println!` 与 `std::io::stdout`，只放行一处 CLI 出口           |
| 许可门禁    | 已实现     | `verify-sources` / `verify-models` 逐资产核实许可与 SHA-256，只放行 MIT/Apache |
| 语料管线    | 已实现     | 规范化记录与 append-only `stable_id`、平水韵与词林正韵入库、判重与冲突归属     |
| 索引选型    | 已实测     | FTS5 `detail` 模式与 n-gram 辅助表由实测定案，非拍板                           |
| 语料工件    | 已实现     | 随包唐宋 47 万首（gzip 211 MiB），检索结构首启本机派生；打包带六条中止断言     |
| 语音底座    | 已验证     | 五平台原生依赖构建与链接路径、Linux 16 kHz 单声道采集、各平台授权链均实测      |
| 文言字准    | 已实测     | 合成音加增强测得，结论是字准只作参考值                                         |
| 核心检索    | 已实现     | 正文、题目、作者、朝代、首句、尾字、标签、韵部检索与作品详情                   |
| 命令行      | 已实现     | `yunjian search/show/author/rhyme/corpus`，`--json` 信封与 0/1/2/3 退出码      |
| MCP 服务器  | 已实现     | `yunjian mcp` 走 stdio，三个只读检索工具；`mcp install` 写两种客户端配置       |
| AI 赏析     | 已实现     | BYOK 只走系统钥匙串、流式可取消、两级缓存、开放权重预生成管线                  |
| 背诵训练    | 已实现     | 挖空 / 首字提示 / 遮挡三模式共用评分内核，FSRS 复习排程                        |
| 朗读与识别  | 已实现     | 逐音步 TTS 加破读表、流式识别双解码；`voice` 特性默认关闭                      |
| 桌面端      | 已实现     | Tauri v2 + React：自绘标题栏、检索阅读、设置、背诵与语音全流程，IPC 非阻塞     |
| 移动端      | 门面已实现 | `yunjian-mobile` 覆盖四个领域 crate；真机 verdict 未定，尚未构建 binding       |

**尚未实现或尚未验证**：

- **移动端外壳与分发**——真机 verdict 仍为 `undetermined`，binding、UI 与分发管线需物理 Android /
  iOS 设备、`adb` 与签名凭据才能推进。
- **桌面端真机验收**——20 条预声明断言 Linux 侧 3 PASS / 17 NOT EXECUTED（无 GPU 容器加 Xvfb 下
  WebKitGTK 合成到 X 读不回的 GL 表面）；Windows / macOS 无交互式会话与签名身份。
- **首个 tag**——发布管线已经覆盖五个 CLI 目标、三平台桌面安装包、updater 签名和逐资产
  SHA-256，但还没有切出首个正式 tag；桌面端真机验收与签名凭据仍是发布前置条件。
- **随包赏析数据集**——管线与门禁齐备，但本机没有开放权重推理条件，`dataset/` 目前只有 README，
  一条正文都没有编造。
- **词谱句式表**——只覆盖「念奴娇」「水调歌头」两支，依据是《全宋词》实测众数而非公有领域词谱；
  其余词牌诚实退化到按标点断句。

## 快速开始

三步：装、取语料、查一句。

```bash
# 1. 装（Linux / macOS）
curl -fsSL https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.sh | sh
# 2. 取语料（约 211 MiB，首启本机派生检索结构）
yunjian corpus fetch
# 3. 查一句
yunjian search 明月
```

Windows 用 PowerShell：

```powershell
irm https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.ps1 | iex
```

两个脚本都检测系统与 CPU 架构、挑出对应的发布产物、**校验 SHA-256 之后才落盘**，校验不过就
一个文件也不装。环境变量、私有仓库取法、五平台归档与桌面安装包清单见
[安装与发布产物](docs/INSTALL.zh.md)。

> [!NOTE]
> 首个正式发布（`v0.1.0`）还没切出来，上面的安装命令要等它。在此之前从源码构建
> `cargo build --workspace --release -p yunjian-cli`，产物在 `target/release/yunjian`。

## 与 LLM 协作

结果只走 stdout，日志只走 stderr，判断成败只看退出码。展开下面这段可以直接交给一个 AI 助手。

<details>
<summary>命令、输出契约与 MCP 客户端配置</summary>

### 关键命令

```bash
yunjian corpus fetch                    # 下载校验并落地语料库；已就位时是空操作
yunjian search 明月 --limit 10 --json   # 正文与残句检索；--author/--dynasty 在本页内过滤
yunjian show <poem-id> --json           # 按 stable_id 读本体、平仄、韵部、出处、历代集评
yunjian author 李白 --json              # 作者详情与作品列表
yunjian rhyme 七阳 --book pingshui      # 按韵部检索；--book 必填，没有隐式默认
yunjian recite <poem-id> --mode cloze   # 一轮打字背诵，作答从 stdin 读
yunjian mcp                             # 在 stdio 上承载 MCP 服务器
```

`corpus status`、`recite due`、`models list` 与全部子命令选项见 [命令行](docs/CLI.zh.md)。

### stdout / stderr / 退出码

**stdout 只放结果**（人类可读文本，或 `--json` 时**恰好一行** JSON）；**stderr 只放日志**，
`RUST_LOG=trace` 也不会污染 stdout。退出码只有四个，机器判断只看它：

| 码  | 含义       | 正确反应                              |
| --- | ---------- | ------------------------------------- |
| 0   | 成功有结果 | 读 `data`                             |
| 1   | 结果为空   | 「查过了，没有」——不是错误            |
| 2   | 用法错误   | 改命令（含请求了未随包的韵书）        |
| 3   | 数据不可用 | 补数据，通常是 `yunjian corpus fetch` |

**1 和 3 不能混**：把语料缺失读成「诗库里没有李白」是这条边界上最贵的错。

`--json` 信封的固定形状：

```json
{
  "schema_version": 1,
  "command": "search",
  "status": "ok",
  "warnings": [],
  "data": {}
}
```

`status` 取 `ok` / `empty` / `error`，与退出码一一对应。`status == "error"` 时 `data` 换成
`error`，内含稳定 `code`、中文 `message` 和可执行的 `hint`。`warnings` 每条带稳定 `code`
（如 `voice_fallback`）；遇到不认识的 `code`，原样转达 `message` 即可。

### 注册进 MCP 客户端

`yunjian mcp install --client claude`（或 `--client opencode`）一条命令就能写好。要手写也可以，
但**两种客户端的形态不通用**——顶层键不同，`command` 的类型也不同：

Claude Desktop（`claude_desktop_config.json`）：`command` 是**字符串**，参数另放 `args`。

```json
{
  "mcpServers": {
    "yunjian": {
      "command": "yunjian",
      "args": ["mcp"]
    }
  }
}
```

OpenCode（`opencode.json`）：`command` 是**含参数的数组**，另有 `type` 与 `enabled`。

```json
{
  "mcp": {
    "yunjian": {
      "type": "local",
      "command": ["yunjian", "mcp"],
      "enabled": true
    }
  }
}
```

把其中一种套到另一种上，客户端读到的是一个语法合法而语义为空的条目：不报错，只是永远连不上。
`yunjian` 不在 `PATH` 上时，把 `"yunjian"` 换成绝对路径。

</details>

## 功能概览

以下是**目标**形态，当前进度一律以 [项目状态](#项目状态) 为准。

- **离线检索**。只读 SQLite 文件，不联网、不登录、不需要账号；默认随包唐宋 47 万首，全量
  90 万首作为应用内可选下载。见 [语料与索引](docs/CORPUS.zh.md)。
- **AI 赏析（自带 key）**。key 存在操作系统钥匙串里，不走环境变量、不进配置文件；没有 key 也
  完整可用，随包那批**只由开放权重模型生成**。见 [AI 赏析](docs/AI.zh.md)。
- **背诵训练**。挖空、首字提示、遮挡三种打字模式共用一个评分内核。语音路径的字准率**永远只是
  估计值，不是分数**——文言的语音识别不够可靠到能给你打分。
- **朗读**。逐音步合成 + Rust 侧插静音，用项目自己的公有领域来源破读表处理古音；覆盖范围外
  回落到现代普通话读音。见 [语音](docs/VOICE.zh.md)。
- **MCP 服务器**。让 Claude Desktop、OpenCode 这类客户端直接查你的诗库；生成类工具的输出标注「AI 生成，非古人作品」，永远不写回语料。
- **多端**。桌面端 Tauri v2 + React；命令行带机器可读输出；移动端框架由真机实测决定。

## 内容来源与许可

一句话：**版权墙决定了架构**——现存带现代注释、译文、赏析的开源数据集授权链都立不住，所以
云笺只做「公有领域原文 + 逐条注明出处的前现代集评 + 明确标注的 AI 赏析」这三者的组合。

- **代码**按 [MIT](LICENSE) 授权。**随包语料只有公有领域原文与 MIT 许可的上游整理产物**：
  逐资产判定在 [`corpus/sources.toml`](corpus/sources.toml)，被拒数据集连同理由在
  [`corpus/DENYLIST.md`](corpus/DENYLIST.md)；判定粒度是单个文件而不是仓库。
- **赏析文本是 AI 生成的，不是学术成果**，界面里与有出处的集评分开呈现并标注未经人工审校；
  随包预生成的那批只由开放权重模型生成，不用闭源 API。
- **`voice` 特性默认关闭**，默认构建纯 MIT；开启后产物静态含 GPL-3.0 的 espeak-ng，因此分发
  一份开启语音的云笺整体须按 GPL-3.0 提供。不随包分发任何模型权重。
- **语音路径的逐字准确率永远只是参考值，不是分数**——见 [CER 报告](docs/reports/asr-cer.md)。
- 完整判定链与逐条许可见 [内容来源与许可](docs/PROVENANCE.zh.md) 与 [第三方许可](LICENSES.md)。

## 文档

- [架构](docs/ARCHITECTURE.zh.md)——分层、`yunjian-core` 为什么不知道 Tauri 存在、移动端逃生通道
- [语料与索引](docs/CORPUS.zh.md)——构建管线、`stable_id` 身份模型、FTS5 索引选型实测
- [命令行](docs/CLI.zh.md)——子命令、`--json` 信封 schema、四个退出码、stdout/stderr 分工
- [AI 赏析](docs/AI.zh.md)——BYOK、两级缓存、预生成策略与标注义务
- [语音](docs/VOICE.zh.md)——模型与许可、破读词表、v1 反馈契约（**不评判读音标准**）；构建与链接见 [语音构建](docs/VOICE-BUILD.zh.md)
- [安装与发布产物](docs/INSTALL.zh.md)——安装脚本环境变量、私有仓库、五平台归档与安装包
- [内容来源与许可](docs/PROVENANCE.zh.md)——版权墙如何决定架构，以及逐类内容的许可判定
- [开发流程](docs/DEVELOPMENT.zh.md)——门禁命令、需要前置工件的命令、发布凭据与提交约定
- [平台要求](docs/PLATFORM-REQUIREMENTS.zh.md)——五平台系统最低版本、麦克风授权链、降级行为
- [CER 报告](docs/reports/asr-cer.md)——文言语音识别字准实测，以及它为什么只能当参考值
- [第三方许可](LICENSES.md)——逐条列出随包或下载的第三方资产、许可与署名

## 参与开发

需要 Rust 1.95+。`make hooks` 装提交与推送钩子，`make ci` 是唯一门禁（格式 + clippy + 测试 +
MCP 一致性 + 前端测试），`make help` 列出所有 target。

几条 xtask 子命令消费 gitignored 的大工件（语料库、语音模型），新鲜检出里没有；它们不会裸退出，
而会点名缺什么、该先跑哪一条。完整清单、发布凭据、CI runner 现状与提交约定见
[开发流程](docs/DEVELOPMENT.zh.md)。三条硬约定：**提交信息用中文祈使句 + Conventional
Commits**；**不许 `println!`**（同一个二进制托管 MCP stdio 服务器，stdout 一行杂音毁掉协议流）；
**密钥不进 `config.toml`**，也绝不摄入第三方现代注释、译文或赏析。Issue 与 PR 都欢迎，改动语料
源之前先读 [语料与索引](docs/CORPUS.zh.md)。
