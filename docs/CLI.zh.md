简体中文

# 命令行

`yunjian` 是云笺的终端入口。语料是一份只读 SQLite 文件，检索不联网、不登录。

> **本文档描述已实现的部分。**

## 目录

- [两条流的约定](#两条流的约定)
- [退出码](#退出码)
- [JSON 信封](#json-信封)
- [子命令](#子命令)
- [注册进 MCP 客户端](#注册进-mcp-客户端)
- [全局参数](#全局参数)
- [过滤的作用范围](#过滤的作用范围)

## 两条流的约定

**stdout 只放结果**：人类可读文本，或 `--json` 的一行信封。
**stderr 只放日志**：进度、降级警告、失败原因，全部经 `tracing`。

```bash
yunjian search 明月 --json | jq -e '.data.hits | length > 0'
```

这条管道在任何日志级别下都成立，`RUST_LOG=trace` 也一样。理由不是洁癖：同一个二进制
还要承载 `yunjian mcp` 的 stdio 服务端，而 MCP 规范要求 stdout 上只能有协议帧。工作区级的
`print_stdout = "deny"` 与 `clippy.toml` 的 `disallowed-methods` 把它变成编译期门禁，
豁免只有 `crates/yunjian-cli/src/present.rs` 一处，且有测试盯着这个数量。

日志同时写按天滚动的文件，目录由 `[logger] dir` 决定；目录不可写时降级成只写 stderr，
并以 `warn` 记下这次降级，而不是静默发生。

## 退出码

| 码  | 含义       | 触发条件                                                             |
| --- | ---------- | -------------------------------------------------------------------- |
| 0   | 成功       | 命令执行完成且有结果                                                 |
| 1   | 无结果     | 命令执行完成但结果集为空                                             |
| 2   | 用法错误   | 参数解析失败、请求本身不成立、请求了未随包的韵书                     |
| 3   | 语料不可用 | 语料库缺失、损坏、schema 不兼容，或读取语料时的底层 I/O 与数据库故障 |

**1 与 3 的区别是产品语义**：1 说「我查过了，没有」，3 说「我没法查」。把语料缺失报成
0 条结果会让脚本读成「诗库里没有李白」，而正确反应是去取语料——因此退出 3 的文案一定
点名 `yunjian corpus fetch`。

**2 与 3 的区别是「谁该改」**：2 是调用方改命令，3 是调用方补语料。所以
`--book xinyun` 是 **2 而不是 3**：中华新韵是一条许可判定（现代出版物，授权链未核实），
`corpus fetch` 取不来它。

`show` 一个不存在的 `stable_id` 是 **1**，与 `search` 查不到同类。

## JSON 信封

`--json` 在 stdout 输出**恰好一行** JSON：

```json
{
  "schema_version": 1,
  "command": "search",
  "status": "ok",
  "warnings": [],
  "data": { "...": "各子命令自有" }
}
```

| 字段             | 出现时机           | 说明                                                                                                             |
| ---------------- | ------------------ | ---------------------------------------------------------------------------------------------------------------- |
| `schema_version` | 总是               | 当前为 `1`；不兼容变更才递增                                                                                     |
| `command`        | 总是               | 稳定 ASCII 名：`search` / `show` / `author` / `rhyme` / `corpus.status` / `corpus.fetch` / `mcp` / `mcp.install` |
| `status`         | 总是               | `ok`（有结果）/ `empty`（无结果）/ `error`                                                                       |
| `warnings`       | 总是（可为空数组） | 降级提示，每条带稳定 `code` 与中文 `message`                                                                     |
| `data`           | `status != error`  | 子命令载荷；`status = empty` 时仍在（空数组也是答案）                                                            |
| `error`          | `status == error`  | `code` + 中文 `message` + 可选 `hint`                                                                            |

`status` 与退出码一一对应：`ok` → 0，`empty` → 1，`error` → 2 或 3。

**中文只出现在 `message` 里**，供人阅读；机器判断一律看 `status`、`code` 与退出码。

`warnings[].code` 的取值：

| code                     | 含义                                                     |
| ------------------------ | -------------------------------------------------------- |
| `derived_unavailable`    | 首启派生结构不可用，一至两字查询本次退化为全表扫描       |
| `degraded_plan`          | 本次查询没有走索引约束，附退化原因                       |
| `filtered_page_empty`    | 本页命中被 `--author` / `--dynasty` 清空，但还有后续页   |
| `rhyme_book_unavailable` | 请求的韵书未随包，相关标注为空而不是「没有韵部」         |
| `client_scope_ignored`   | 请求的作用域对该客户端不适用，已按它唯一支持的作用域处理 |

`error.code` 的取值：`usage`、`rhyme_book_unavailable`、`corpus_unavailable`、
`not_implemented`、`client_config_invalid`、`client_config_write_failed`。

**兼容性**：新增字段、新增 `warnings` 或 `error` 的 `code` 取值都不递增 `schema_version`；
删除字段或改变既有字段含义才递增。调用方遇到未知 `code` 的正确反应是原样转达 `message`。

## 子命令

```bash
yunjian search <QUERY> [--limit N] [--author NAME] [--dynasty DYNASTY] [--rhyme-book BOOK] [--cursor CURSOR]
yunjian show <POEM_ID>
yunjian author <NAME> [--cursor CURSOR]
yunjian rhyme <GROUP> --book <BOOK> [--tone TONE]
yunjian corpus status
yunjian corpus fetch
yunjian mcp
yunjian mcp install --client <claude|opencode> [--global] [--path PATH] [--dry-run]
```

- **`search`** 检索正文或残句。两字查询（「明月」）走辅助 n-gram 候选表而不是 trigram，
  因为 `%明月%` 只有两个字面字符，FTS5 推不出任何 trigram 约束。`--limit` 默认 10，
  服务端硬上限 100。
- **`show`** 按 `stable_id` 取本体、平仄、韵部、出处与历代集评。平仄未知位置以 `？` 呈现，
  **不按平声渲染**：韵书未收的字就是不知道。集评每条都带出处，缺出处是硬错误而不是空字段。
- **`author`** 按作者名或前缀取作者详情与作品列表，并列出归属冲突（同一正文挂在多个作者
  名下，如《赤壁》双挂杜牧与李商隐）。
- **`rhyme`** 按韵部检索。`--book` 是**必填项**，没有隐式默认值：平水韵为诗韵，不适用于
  词牌格律，隐式默认会让「拿词对平水韵」变成静默的错答案。韵脚未消歧的候选单列在
  `unresolved` 里，不计入命中。
- **`corpus status`** 报告语料库位置、版本、规模与派生结构状态。**没有副作用**：语料缺失时
  只报告并退出 3，不去落地一份——一条查看状态的命令不该有十分钟的副作用。
- **`corpus fetch`** 校验、解压并落地语料库，必要时派生检索结构。首启派生实测唐宋规模
  571.8 s，因此进度逐步汇报到 stderr。
- **`mcp`** 在 stdio 上承载 MCP 服务器。语料缺失时照样启动并完成握手，工具调用返回结构化的
  `corpus_missing`——客户端等不了一次语料下载。
- **`mcp install`** 把 `yunjian mcp` 写进客户端配置，见下节。

## 注册进 MCP 客户端

```bash
yunjian mcp install --client claude      # 用户级，Claude Desktop
yunjian mcp install --client opencode    # 当前目录的项目级 opencode.json
yunjian mcp install --client opencode --global   # 用户级 ~/.config/opencode/opencode.json
yunjian mcp install --client opencode --dry-run  # 只把结果文件打到 stdout
```

**两个客户端的形态不通用，`--client` 因此没有默认值。** 顶层键不同，`command` 的类型也不同：

Claude Desktop —— `claude_desktop_config.json`，`command` 是**字符串**，参数另放 `args`：

```json
{
  "mcpServers": {
    "yunjian": { "command": "yunjian", "args": ["mcp"] }
  }
}
```

OpenCode —— `opencode.json`，`command` 是**含参数的数组**，另有 `type` 与 `enabled`：

```json
{
  "mcp": {
    "yunjian": { "type": "local", "command": ["yunjian", "mcp"], "enabled": true }
  }
}
```

把其中一种套到另一种上不会报错，只会让客户端读到一个语法合法而语义为空的条目——它永远连不上。

| 客户端     | 目标文件                                             | 作用域                        |
| ---------- | ---------------------------------------------------- | ----------------------------- |
| `claude`   | 平台配置目录下 `Claude/claude_desktop_config.json`   | 只有用户级，`--global` 无效果 |
| `opencode` | 默认当前目录 `opencode.json`；`--global` 走 XDG 目录 | 项目级与用户级                |

OpenCode 的用户级路径按 **XDG 语义**推导（`$XDG_CONFIG_HOME`，否则 `~/.config`），
在 macOS 上也是 `~/.config/opencode/opencode.json` 而不是 `~/Library/Application Support`
——它不跟随平台惯例，按平台惯例去猜会写进一个它永远不读的文件。当前目录已有
`opencode.jsonc` 时写它而不是新建 `.json`，否则客户端会读到两份配置。

四条行为约定：

- **合并，不覆盖。** 除 `yunjian` 那一段，其余字节逐字保留，注释也留着。用户已有的
  MCP 服务器条目一个都不动。
- **注释与扩展名无关。** 实测真实的 `opencode.json` 里就有 `//` 注释，因此注释剥离对所有
  目标文件一律执行，`.jsonc` 不是开关。
- **写前备份。** 备份名形如 `opencode.json.bak-<毫秒时间戳>`。带时间戳而不是固定 `.bak`：
  固定名在第二次安装时会被上一次的结果覆盖，于是原始配置这份最值得留的备份反而最先丢。
  新建文件与内容无变化时不产生备份。
- **解析不了就拒绝。** 目标文件不是合法 JSON/JSONC 时退出 2 并逐字节保留原文，不会拿一份
  干净的新文件替换它——那等于用「安装成功」换掉用户的全部配置。

条目里写裸名 `yunjian` 还是绝对路径，取决于 `yunjian` 在不在 `PATH` 上。在 `PATH` 上时写
裸名，配置因此在换安装位置或同步到另一台机器后仍然成立；不在时落绝对路径，否则那类安装
根本起不来。`PATH` 上先命中的是另一个同名二进制时也落绝对路径。

## 全局参数

| 参数                | 说明                                                                         |
| ------------------- | ---------------------------------------------------------------------------- |
| `--config <PATH>`   | 配置文件。发现顺序：本参数 → `APP_CONFIG` → `./config.toml` → 用户配置目录   |
| `--corpus <PATH>`   | 只读语料库文件，压过配置与 `YUNJIAN_CORPUS_PATH`                             |
| `--log-level LEVEL` | `off` / `error` / `warn` / `info` / `debug` / `trace`；**不**压过 `RUST_LOG` |
| `--json`            | 在 stdout 输出信封而不是人类可读文本                                         |

四个参数都可以出现在子命令之前或之后。

`--config` 与 `APP_CONFIG` **不做存在性检查**：显式指向一个不存在的文件会报错，而不是
静默降到下一级。`--corpus` 同理。

`--log-level` 刻意不压过 `RUST_LOG`：`RUST_LOG` 存在时整条过滤器都由它决定（这样
`RUST_LOG=tower=warn,info` 这类分目标指令才有意义），在 CLI 里改这条会让两个入口各说一套。

## 过滤的作用范围

`--author` 与 `--dynasty` **只过滤本页**，不改变分页边界：`total_estimate` 是过滤前的数，
游标也按过滤前的序列推进。本页被过滤清空时会给出 `filtered_page_empty` 警告并提示翻页，
而不是让人读成「语料里没有这首诗」。

`--rhyme-book` 是**标注**而不是过滤：它给每条命中附上该韵书下的韵部归属，不减少命中数。
请求未随包的韵书会退出 2，而不是给出一串空标注——空标注会被读成「这些诗没有韵部」。
