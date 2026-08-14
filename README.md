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
- [与 LLM 协作](#与-llm-协作)
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
| MCP 服务器   | 已实现 | `yunjian mcp` 走 stdio，三个只读检索工具；`yunjian mcp install` 写 Claude 与 OpenCode 配置                 |
| AI 赏析      | 已实现 | BYOK 只走系统钥匙串、流式可取消、两级缓存、开放权重预生成管线，见 [AI 赏析](docs/AI.zh.md)                 |
| 背诵训练     | 已实现 | 挖空 / 首字提示 / 遮挡三模式共用评分内核，FSRS 复习排程；`yunjian recite` 可无 GUI 使用                    |
| 朗读与识别   | 已实现 | 逐音步 TTS 加破读表、流式识别双解码；`voice` 特性默认关闭，见 [语音](docs/VOICE.zh.md)                     |
| 桌面端       | 已实现 | Tauri v2 + React：自绘标题栏、检索阅读、设置、背诵与语音全流程，IPC 全部非阻塞                             |

**尚未实现或尚未验证**：

- **移动端**——`yunjian-mobile` 门面、UI 与分发管线都还没有一行代码，需物理 Android / iOS
  设备、`adb` 与签名凭据才能推进。
- **桌面端真机验收**——20 条预声明断言 Linux 侧 3 PASS / 17 NOT EXECUTED（无 GPU 容器加 Xvfb
  下 WebKitGTK 合成到 X 读不回的 GL 表面）；Windows / macOS 无交互式会话与签名身份。
- **首个 tag**——发布管线已经覆盖五个 CLI 目标、三平台桌面安装包、updater 签名和逐资产
  SHA-256，但还没有切出首个正式 tag；桌面端真机验收与签名凭据仍是发布前置条件。
- **随包赏析数据集**——管线与门禁齐备，但没有开放权重推理条件，`dataset/` 目前只有 README，
  一条正文都没有编造。
- **词谱句式表**——只覆盖「念奴娇」「水调歌头」两支，依据是《全宋词》实测众数而非公有领域
  词谱；其余词牌诚实退化到按标点断句。

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

两个脚本都检测系统与 CPU 架构、挑出对应的发布产物、**校验 SHA-256 之后才落盘**，
校验不过就一个文件也不装。可用环境变量：

| 变量                        | 缺省               | 作用                                                         |
| --------------------------- | ------------------ | ------------------------------------------------------------ |
| `YUNJIAN_VERSION`           | 最新正式发布       | 装指定版本，`v0.1.0` 与 `0.1.0` 都收                         |
| `YUNJIAN_INSTALL_DIR`       | `$HOME/.local/bin` | 安装目录                                                     |
| `GH_TOKEN` / `GITHUB_TOKEN` | 无                 | 通过 GitHub CLI 下载私有仓库 Release；也可先 `gh auth login` |

私有仓库必须先安装 `gh`，再提供能读取该仓库 Release 的 token，或提前完成 `gh auth login`：

```bash
GH_TOKEN=github_pat_xxx sh scripts/install.sh
```

```powershell
$env:GH_TOKEN = 'github_pat_xxx'
.\scripts\install.ps1
```

token 只交给 GitHub CLI，不写入安装脚本的临时目录。自定义 `YUNJIAN_BASE_URL` /
`YUNJIAN_API_URL` 时仍使用普通 HTTP 下载，便于内部镜像和离线测试。

发布管线生成以下 CLI 归档，全部启用 `voice,mcp`，并将 voice 所需动态库放在归档内：

| 系统    | 目标                        | 归档     |
| ------- | --------------------------- | -------- |
| Linux   | `x86_64-unknown-linux-gnu`  | `tar.gz` |
| Linux   | `aarch64-unknown-linux-gnu` | `tar.gz` |
| macOS   | `x86_64-apple-darwin`       | `tar.gz` |
| macOS   | `aarch64-apple-darwin`      | `tar.gz` |
| Windows | `x86_64-pc-windows-msvc`    | `zip`    |

Linux CLI 以 glibc 2.31 为上限；sherpa-onnx 没有可用于 `voice` 的 musl 预编译库，所以安装脚本
会先兼容探测旧版 musl 资产，再回退到当前 GNU 资产。桌面端另发布 Linux x86_64 的 `.deb` 与
`.AppImage`、macOS Apple Silicon 的 `.dmg` 与 updater `.app.tar.gz`、Windows x86_64 的 NSIS
`.exe` 与 `.msi`。Tauri 自动更新只声明 `linux-x86_64`、`darwin-aarch64` 和
`windows-x86_64-nsis`；每个安装包、签名、`latest.json` 与 CLI 归档都有同名 `.sha256`。

> [!NOTE]
> 首个正式发布（`v0.1.0`）还没切出来，上面的安装命令要等它。在此之前从源码构建：
> `cargo build --workspace --release -p yunjian-cli`，可执行文件在
> `target/release/yunjian`。开发者流程见 [参与开发](#参与开发)。

## 与 LLM 协作

结果只走 stdout，日志只走 stderr，判断成败只看退出码。展开下面这段可以直接交给一个
AI 助手。

<details>
<summary>命令、输出契约与 MCP 客户端配置</summary>

### 关键命令

```bash
yunjian corpus fetch                    # 下载校验并落地语料库；已就位时是空操作
yunjian corpus status                   # 语料位置、版本、规模、派生结构状态
yunjian search 明月 --limit 10 --json   # 正文与残句检索；--author/--dynasty 在本页内过滤
yunjian show <poem-id> --json           # 按 stable_id 读本体、平仄、韵部、出处、历代集评
yunjian author 李白 --json              # 作者详情与作品列表
yunjian rhyme 七阳 --book pingshui      # 按韵部检索；--book 必填，没有隐式默认
yunjian recite <poem-id> --mode cloze   # 一轮打字背诵，作答从 stdin 读
yunjian recite due                      # 今天到期的复习项；不读语料库
yunjian models list                     # 语音模型清单与本地缓存状态；不联网
yunjian mcp                             # 在 stdio 上承载 MCP 服务器
```

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

`status` 取 `ok` / `empty` / `error`，与退出码一一对应。`status == "error"` 时 `data`
换成 `error`，内含稳定 `code`、中文 `message` 和可执行的 `hint`。`warnings` 每条带稳定
`code`（如 `voice_fallback`）；遇到不认识的 `code`，原样转达 `message` 即可。

### 注册进 MCP 客户端

一条命令就能写好：

```bash
yunjian mcp install --client claude     # 或 --client opencode
```

要手写也可以。**两种客户端的形态不通用**——顶层键不同，`command` 的类型也不同：

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

把其中一种套到另一种上，客户端读到的是一个语法合法而语义为空的条目：不报错，只是永远
连不上。`yunjian` 不在 `PATH` 上时，把 `"yunjian"` 换成绝对路径。

</details>

## 功能概览

以下是**目标**形态。当前进度一律以 [项目状态](#项目状态) 为准。

- **离线检索**。语料是一个只读 SQLite 文件，不联网、不登录、不需要账号。FTS5 trigram
  索引配一张 1/2 字 n-gram 候选表——因为「明月」这种两字查询占绝大多数，而 trigram
  在三字以下推不出任何约束。默认随包唐宋 47 万首（下载 211 MiB），全量 90 万首作为
  应用内可选下载；检索结构在**首启本机派生**（实测约 10 分钟），派生后检索性能与随包
  时完全相同。理由与实测见 [语料与索引](docs/CORPUS.zh.md)。
- **AI 赏析（自带 key）**。key 存在操作系统钥匙串里，不走环境变量、不进配置文件。
  没有 key 也完整可用：常见名篇随包带一批预生成的赏析，**那批只由开放权重模型生成**，
  不用任何闭源 API。理由、逐条溯源与披露见 [`dataset/README.md`](dataset/README.md)。
  你自己 key 生成的赏析只留在本机。
- **背诵训练**。挖空、首字提示、遮挡三种打字模式共用同一个评分内核；语音练习额外给
  完整度与流畅度。语音路径的字准率**永远只是估计值，不是分数**——文言的语音识别不够
  可靠到能给你打分。
- **朗读**。逐音步合成 + Rust 侧插静音，用项目自己的公有领域来源破读表处理古音；
  覆盖范围外回落到现代普通话读音。
- **MCP 服务器**。`yunjian mcp` 跑在 stdio 上，让 Claude Desktop、OpenCode 这类客户端
  直接查你的诗库。生成类工具的输出标注「AI 生成，非古人作品」，永远不写回语料。
  配置形态与 `mcp install` 见 [与 LLM 协作](#与-llm-协作)。
- **多端**。桌面端 Tauri v2 + React；命令行工具带机器可读输出；移动端框架由真机实测决定。

## 内容来源与许可

**代码**按 [MIT](LICENSE) 授权。

**随包语料只包含公有领域原文，以及由 MIT 许可的上游整理产物。** 逐资产的许可判定写在
[`corpus/sources.toml`](corpus/sources.toml) 里，由 `xtask verify-sources` 强制校验；被拒绝的
数据集连同拒绝理由记在 [`corpus/DENYLIST.md`](corpus/DENYLIST.md)。判定粒度是**单个文件**
而不是仓库——一个仓库的 MIT LICENSE 不能为它抓来的内容授权。

**赏析文本是 AI 生成的，不是学术成果。** 界面里它与有出处的集评在视觉上分开呈现，并附带
「未经人工审校」的说明。AI 生成的诗永远标注「AI 生成，非古人作品」，永远不进语料库和赏析表。

**随包预生成的那批赏析只由开放权重模型生成，不用闭源 API。** 这是授权链约束而非性能
偏好——三家闭源 API 的输出再分发条款里有两家未能核实，而下载下来的权重不附带这类条款。
约束由 `xtask pregenerate` 在生成任何一条记录之前强制（只认 MIT 与 Apache-2.0 权重、
只认本地运行时），逐条溯源与完整披露在 [`dataset/README.md`](dataset/README.md)。

**语音功能的许可分两档。** `voice` cargo 特性**默认关闭**，默认构建是纯 MIT 且实测不
链接 onnxruntime；开启后预编译的 sherpa-onnx 产物静态包含 GPL-3.0 的 espeak-ng，因此
**分发一份开启语音的云笺，整体须按 GPL-3.0 条款提供**。发布产物据此分两种，细节见
[语音构建](docs/VOICE-BUILD.zh.md)。

不会随包分发任何模型权重。语音模型按需下载，只接受经核实的 MIT 或 Apache-2.0 许可——
逐模型判定、证据与摘要在 [`models.toml`](models.toml)，由 `xtask verify-models` 强制；
被拒的模型连同理由在 [`models/DENYLIST.md`](models/DENYLIST.md)。核实推翻了一条先前的
判断：**FunASR 系（SenseVoice / Paraformer）走阿里自家的许可协议**，因此离线识别只剩
Whisper 一族可用。

**语音路径上的逐字准确率永远只是参考值，不是分数。** 这不是保守措辞，而是实测结论：
见 [CER 报告](docs/reports/asr-cer.md)。

## 文档

- [架构](docs/ARCHITECTURE.zh.md)——分层、`yunjian-core` 为什么不知道 Tauri 存在、移动端逃生通道
- [语料与索引](docs/CORPUS.zh.md)——构建管线、`stable_id` 身份模型、FTS5 索引选型实测
- [语音](docs/VOICE.zh.md)——模型与许可、破读词表、v1 反馈契约（**不评判读音标准**）
- [AI 赏析](docs/AI.zh.md)——BYOK、两级缓存、预生成策略与标注义务
- [命令行](docs/CLI.zh.md)——子命令、`--json` 信封 schema、四个退出码、stdout/stderr 分工
- [语音构建](docs/VOICE-BUILD.zh.md)——五平台原生依赖构建、链接方式、许可影响
- [平台要求](docs/PLATFORM-REQUIREMENTS.zh.md)——五平台系统最低版本、麦克风授权链、低于底线时的降级行为
- [CER 报告](docs/reports/asr-cer.md)——文言语音识别字准实测，以及它为什么只能当参考值
- [第三方许可](LICENSES.md)——逐条列出随包或下载的第三方资产、许可与署名

## 参与开发

需要 Rust 1.95+。`rust-toolchain.toml` 默认跟随 stable，CI 另用精确的 Rust 1.95
执行最低版本编译门禁：

```bash
make hooks   # 装 pre-commit（提交时格式化）与 pre-push（推送前跑 make ci）钩子
make ci      # 唯一门禁：格式检查 + clippy + 测试 + MCP 一致性 + 前端测试
make help    # 列出所有 target
```

`cargo run -p xtask -- verify-sources --offline` 离线校验上游语料源逐资产的许可与摘要，
退出码 0 说明环境是对的。

正式发布还需要仓库 Actions secrets `TAURI_SIGNING_PRIVATE_KEY` 和
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。任一 updater 签名缺失或与应用内公钥不匹配时，流程
都会失败并让 GitHub Release 保持 draft；不要通过关闭签名绕过门禁。

Linux x86_64 作业已记录 AWS CodeBuild runner `yunjian-runner` 的迁移标签，但当前
CodeConnections 连接 `yunjian-github` 仍为 `PENDING`。在 AWS 控制台授权连接并重建
`WORKFLOW_JOB_QUEUED` webhook 前，所有 job 继续使用 GitHub-hosted runner。Windows、macOS、
Linux ARM 和混合平台矩阵不迁移到这个 Linux x86_64 项目；启用 CodeBuild 时每个 job 还必须
保留自己的唯一第二标签，避免 GitHub 的 superset 标签匹配把作业路由到错误 runner。

约定：

- **提交信息用中文祈使句 + Conventional Commits**，例如 `feat(core): 添加韵部检索`。
- **不许 `println!`**。日志走 `tracing` 到 stderr 与滚动文件；同一个二进制要托管 MCP
  stdio 服务器，stdout 上一行杂音就会毁掉协议流。
- **密钥不进 `config.toml`**。顶层配置开了 `deny_unknown_fields`，粘一个 `api_key` 进去
  会直接报错而不是被静默丢弃。
- **绝不摄入第三方现代注释、译文或赏析**，理由见上文和 `corpus/DENYLIST.md`。

Issue 与 PR 都欢迎。改动语料源之前先读 [语料与索引](docs/CORPUS.zh.md)。
