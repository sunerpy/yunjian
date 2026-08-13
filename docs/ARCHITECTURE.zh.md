简体中文 · [English](readme/ARCHITECTURE.md)

# 架构

本文记录已经落地的边界与它们的理由。**尚未接线的部分明确标注为「尚未接线」，不写成已具备。**

## 目录

- [工作区布局](#工作区布局)
- [为什么 core 不依赖任何外壳](#为什么-core-不依赖任何外壳)
- [检索路由：三条分支，每条都有存在的理由](#检索路由三条分支每条都有存在的理由)
- [语料解析与原子物化](#语料解析与原子物化)
- [长任务、事件与取消：全工作区一套协议](#长任务事件与取消全工作区一套协议)
- [IPC：已定案的部分与尚未接线的部分](#ipc已定案的部分与尚未接线的部分)
- [日志与 stdout 禁令](#日志与-stdout-禁令)

## 工作区布局

根 `Cargo.toml` 的 `members` 是 9 个（`resolver = "3"`）：

| crate            | 依赖的工作区 crate                                                 | features                                      | 职责（取自 crate 级 `//!`）                                            |
| ---------------- | ------------------------------------------------------------------ | --------------------------------------------- | ---------------------------------------------------------------------- |
| `yunjian-core`   | **无**                                                             | 未声明                                        | 配置加载、日志初始化、统一错误类型与稳定标识，与运行环境无关的领域逻辑 |
| `yunjian-corpus` | `yunjian-core`                                                     | 未声明                                        | 公有领域语料入库、清洗、稳定标识铸造、FTS5 索引构建                    |
| `yunjian-ai`     | `yunjian-core`                                                     | 未声明                                        | AI 赏析生成、提示词模板版本管理、多供应商适配                          |
| `yunjian-recite` | `yunjian-core`                                                     | 未声明                                        | 对齐评分内核与 FSRS 复习调度                                           |
| `yunjian-voice`  | **无**                                                             | `default=[]`、`capture`、`download`、`voice`  | 逐音步合成、静音拼接节奏控制、录音采集                                 |
| `yunjian-mcp`    | `yunjian-core`、`yunjian-ai`                                       | `http`（无 `default`）                        | MCP 服务端，默认 stdio                                                 |
| `yunjian-cli`    | `yunjian-core`、`yunjian-ai`、可选 `yunjian-mcp` / `yunjian-voice` | `default=["mcp"]`、`mcp`、`mcp-http`、`voice` | `yunjian` 二进制；stdout 只放结果，stderr 只放日志                     |
| `yunjian-app`    | `yunjian-core`                                                     | 未声明                                        | 桌面外壳；**同时扮演 Tauri 约定里的 `src-tauri` 角色**                 |
| `xtask`          | `yunjian-core`、`yunjian-corpus`、`yunjian-ai`、`yunjian-voice`    | `default=[]`、`voice`                         | 仓库任务运行器（`cargo xtask <子命令>`）                               |

两处布局细节容易被误解，写在这里：

- **没有 `app/src-tauri/` 目录。** `app/` 是 React 前端（`index.html` / `src/` / `vite.config.ts`），Tauri 侧的清单与配置在 `crates/yunjian-app/`（`tauri.conf.json`、`tauri.macos.conf.json`），其 `Cargo.toml` 明确说明该 crate 承担 `src-tauri` 角色。
- **桌面可执行文件叫 `yunjian-desktop`，不叫 `yunjian`。** 后者已被 `yunjian-cli` 的 `[[bin]]` 占用，同名会互相覆盖而 cargo 不报这个冲突。

`yunjian-core` 与 `yunjian-voice` 都不依赖任何其他工作区 crate。前者是刻意的分层要求（见下节）；后者的效果是语音栈可以独立于语料与评分被裁掉。

## 为什么 core 不依赖任何外壳

`yunjian-core` 的 crate 级文档把这条写成不可协商的约束：

> 本 crate 永远不感知 Tauri：不引入任何 `tauri::` 类型，也不假设宿主为桌面外壳，以此保留移动端的实现空间。

它由两道**真实存在的**测试守住，两道都判 `Cargo.toml` 的直接依赖而不是源码字符串：

- `dependency_manifest_excludes_shell_and_rejected_search_engines` 解析自身 manifest 的 `[dependencies]`，逐项断言不存在 `tauri`、`tantivy`、`jieba-rs`、`lindera`、`opencc-rust`。
- `this_crate_declares_no_tauri_dependency` 逐行剥掉 `#` 之后的注释，再断言代码部分不含 `tauri`。

**判依赖而不是判源码，是刻意的**：不给依赖，任何 `use tauri::...` 都无法编译，于是门禁挡住的是整类问题而不是一种写法。需要如实标注适用范围：这两道门禁校验的是**直接依赖边界**，不是对整个传递依赖图做 `cargo tree` 审计。

这条边界买到的是一件具体的东西：移动端外壳的选型（共享框架 vs 原生代码调同一个库）可以在不改动任何诗词逻辑的前提下切换。计划把这个选择挂在一次真机测量上，而不是提前赌一边。

同一层还有两条相关约束，理由与「不依赖外壳」同源：

- **检索是一个 SQLite 文件，不是搜索引擎。** 不引入 tantivy（其 `MmapDirectory` 在 Android 上不受支持）、不引入 jieba / lindera（文言的正确切分粒度是字，不是词）。
- **运行期不带繁简转换字典。** `ferrous-opencc` 只出现在 `yunjian-corpus` 的依赖里，转换发生在构建期；运行期靠构建期同时产出的 `variant_map` 逐字改写查询。

## 检索路由：三条分支，每条都有存在的理由

路由入口是 `yunjian_core::search::query::plan_query(handle, query)`，两个阈值常量是 `TRIGRAM_CHARS = 3` 与 `WHOLE_LINE_MIN_CHARS = 5`。`QueryPlan` 的分支与判据如下（无通配符路径）：

| 查询长度 | 计划                                                        | 落到哪张表                                           | 这条分支为什么必须存在                                                                                                      |
| -------- | ----------------------------------------------------------- | ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 1–2 字   | `NgramCandidates`；派生结构缺失时退 `FullScan` 并带原因     | `ngram`（索引 `ngram_gram_idx`）回表核验 `poem.body` | FTS5 trigram 在三字以下**推不出任何约束**，`LIKE '%明月%'` 会退化成虚表全扫。而「明月」「相思」「李白」正是最常见的真实查询 |
| 3 字     | `Match`（短语表达式）                                       | `poem_fts MATCH`                                     | 恰好够一个 trigram，是 FTS5 唯一能直接吃下的最短形态                                                                        |
| 4 字     | `Like`                                                      | `poem_fts ... WHERE f.body LIKE`                     | trigram 索引能为它形成约束，但短语匹配的收益不足以额外要求 `detail=full`                                                    |
| ≥5 字    | `index_detail_mode() == "full"` 时 `Match`，否则回落 `Like` | `poem_fts`                                           | 整句检索依赖位置信息；`detail` 一旦不是 `full`，短语匹配在 SQLite 层面就不可用，此时诚实回落而不是报错                      |

带 `%` / `_` 的模式另走一条：最长连续字面量不足 3 字时返回 `FullScan`，警告原文是「查询模式没有连续三个字面字符，trigram 索引无法形成约束」；否则走 `Like`。

**三张检索结构是 `poem.body` 的确定性派生物**，常量 `DERIVED_TABLES = ["ngram", "poem_fts", "poem_last_char"]`。它们不随包，由应用首启在本机构建（`yunjian_core::derive`）——理由与体积有关，细节见[语料与索引](CORPUS.zh.md)。

`poem_fts` 的 `detail` 取值不是拍脑袋定的，而是实测裁决，写在 `corpus/reports/index-mode.json`：`chosen_mode = "full"`、`ngram_aux_enabled = true`。两条否掉备选方案的实测：`detail=none` 在整句五言上直接报 `phrase queries are not supported (detail!=full)`；关掉 n-gram 后两字查询在发布规模上外推到 428.0 ms。裁决由 `crates/yunjian-core/tests/index_mode_verdict.rs` 的 `chosen_configuration_violates_neither_gate` 钉住，运行期实际取值来自 `corpus_meta.index_detail_mode`——**所以改裁决就是改真正建出来的索引**，契约立刻变红。

## 语料解析与原子物化

调用链是 `CorpusHandle::open` → `open_with_progress` → `resolve` → `connect_read_only` → `open_corpus` → `CorpusMeta::read` → `ensure_derived`。

`resolve` 三级顺序，**任一级显式给出却缺失就报错，绝不静默降级**：

1. `cfg.path` 显式路径；
2. `cfg.data_dir.join("corpus.db")`；
3. `.db.gz` 归档 → 读 `manifest.json` 的摘要期望 → 清理残留临时文件 → `materialize`。

只读打开是**两道保险**，不是一道：`Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)` 让文件描述符不可写，`PRAGMA query_only = true` 连后续 `ATTACH` 一并封住。每个 worker 现开一个只读连接，因为 `rusqlite::Connection` 不是 `Sync`。

物化的顺序是「校验 → 同目录临时文件 → `fsync` → 改名前验证 → `rename` → `fsync` 目录」：

1. 比对归档字节数；
2. `sha256_of_file` 与期望摘要比较，**失败时明确未写出任何文件**；
3. 在目标同目录生成唯一 `.tmp` 名（同目录是必需的：跨文件系统 `rename` 不原子）；
4. 解压，`writer.flush()` 后 `sync_all()`；
5. `validate_materialized` 用只读连接验证 schema 与 `corpus_meta`——**在改名之前**；
6. `rename(temp, target)`，随后尽力对父目录 `sync_all()`。

失败路径删除临时文件；删不掉也不会被后续运行认作 `corpus.db`，下次物化前 `sweep_stale_temps` 按前缀加 `.tmp` 后缀清扫。

## 长任务、事件与取消：全工作区一套协议

`yunjian_core::operation` 是「全工作区唯一的长任务事件、取消与资源释放协议」。`Event<P, I>` 的五个变体分工明确：

- `Progress(P)`——**可合并**的进度快照；
- `Item(I)`——**不可丢弃**的增量结果；
- `Done` / `Cancelled` / `Failed { message }`——三个终止事件，`message` 已脱敏。

消费侧是 `OperationHandle<P, I>`，生产侧是 `OperationReporter<P, I>`，队列上限 `EVENT_QUEUE_CAPACITY = 256`。

这套协议之所以在 `yunjian-core` 而不是在外壳里，正是「core 不感知外壳」的直接后果：AI 流式赏析、语料首启派生、模型下载都是长任务，它们必须在没有 Tauri 的 CLI 与 MCP 下同样能跑。`yunjian-ai` 的流式赏析就把 `OperationHandle` 当作对外唯一句柄，内部再用 `tokio_util::sync::CancellationToken` 把取消传播到 HTTP 流。

## IPC：已定案的部分与尚未接线的部分

**已定案并已落地：**

- 桌面外壳是 `crates/yunjian-app`，日志走 `yunjian_core::init_logger` 而**不是** Tauri 官方日志插件——三个入口（CLI、MCP stdio、桌面 GUI）因此共享同一份级别解析、同一份凭据脱敏、同一份滚动文件布局。插件会带来第二套格式与第二套过滤语义，而两套约定的差异只会在排障最需要日志的时候暴露。
- 初始化顺序不可换：先 `init_config`（没有配置就不知道级别与目录），再 `init_logger`；返回的 guard 必须绑成具名局部变量活到进程结束，丢给 `_` 会立刻析构、后台写文件线程被提前停掉。
- 长任务的事件、进度、取消协议已经存在且与外壳无关（上一节）。

**尚未接线，如实记录：** 当前 `crates/yunjian-app/src/lib.rs` 里唯一的 Tauri 构造代码是

```rust
tauri::Builder::default()
    .run(tauri::generate_context!())
    .expect("启动 Tauri 应用失败");
```

仓库中**没有任何** `#[tauri::command]`、没有 `tauri::generate_handler!` / `.invoke_handler(...)`、没有 `tauri::ipc::Channel`、没有 `.emit(...)` 事件流、也没有 `spawn_blocking`。crate 文档自己把 IPC 命令表写成「后续 todo 接入的」。因此本节**不能**声称已经在用 Channel 流式或事件系统——那属于后续任务，届时 `OperationHandle` 的 `Progress` / `Item` 分工就是它要映射的东西。

平台覆盖配置里有一条已实测的陷阱值得记在架构层：`app.windows` 是数组，被 RFC 7396 **整体替换**而非逐字段合并，漏写任何几何字段都会让 macOS 静默退回 serde 默认值（800×600、无最小尺寸），而 Linux 与 Windows 完全正常。

## 日志与 stdout 禁令

**日志一律走 stderr 与滚动文件，永不走 stdout。** 理由是同一个二进制要托管 MCP stdio 服务器，stdout 上一行杂音就毁掉协议流。

禁令的作用域是**整个工作区**而不是 `yunjian-mcp` 一个 crate：`yunjian-core` 里的一句 `println!` 被 MCP 工具处理函数调到，对协议流的破坏与写在 MCP crate 里完全等价，而「MCP 代码路径」不是 lint 能表达的概念。因此机制是工作区级 `deny` 加一处豁免（CLI 的展示模块），每个成员以 `[lints] workspace = true` 引用，**没有例外**。

两条已实测的边界：

- clippy 的 `print_stdout` 只拦得住宏，直接 `std::io::stdout().write_all()` 会溜过去，所以 `clippy.toml` 另有 `disallowed-methods` 覆盖 `std::io::stdout`。
- `std::println` **刻意不进** `disallowed-macros`：`print_stdout` 对 build script 有内建豁免而 `disallowed_macros` 没有，加进去会让每个 build script 与 cargo 通信的唯一手段失效，而 build script 的 stdout 由 cargo 捕获、本就到不了进程 stdout。
- `std::process::Stdio::inherit()` 未被封，是已知的残余缺口：子进程继承 stdout 同样能污染协议流。没封的原因是 MCP 一致性 harness 与既有子进程测试都要用 `Command`。当前的补偿只有约定（显式 `Stdio::null()` / `Stdio::piped()`），没有机制。

## 相关文档

- [语料与索引](CORPUS.zh.md)——来源与许可、身份模型、索引实测、集评准入
- [语音](VOICE.zh.md)——模型与许可、破读词表、v1 反馈契约（不评判读音标准）
- [AI 赏析](AI.zh.md)——BYOK、两级缓存、预生成策略与当前状态
- [平台要求](PLATFORM-REQUIREMENTS.zh.md)——五平台最低版本与麦克风授权链
- [第三方许可](../LICENSES.md)——逐资产的许可与署名
