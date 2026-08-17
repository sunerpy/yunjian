简体中文 · [English](readme/AI.md)

# AI 赏析

AI 赏析在云笺里**不是锦上添花，而是填一个洞**：凡带现代注释、译文、赏析的数据集授权链一律立不住
（见[语料与索引](CORPUS.zh.md)），于是随包内容只可能是「公有领域原文 + 逐条注明出处的前现代集评 +
明确标注的 AI 赏析」这个组合。本文说明密钥怎么存、缓存怎么省钱、预生成怎么做，
**以及预生成当前的真实状态与它的目标状态有什么差别**。

## 目录

- [BYOK：密钥从操作系统钥匙串来，不从环境变量来](#byok密钥从操作系统钥匙串来不从环境变量来)
- [Linux keyutils 是内存型，重启即失（必须如实呈现）](#linux-keyutils-是内存型重启即失必须如实呈现)
- [密钥永不出现在 Debug、日志或 URL 里](#密钥永不出现在-debug日志或-url-里)
- [两级缓存：随包层为什么必须与供应商无关](#两级缓存随包层为什么必须与供应商无关)
- [供应商边界与无密钥运行](#供应商边界与无密钥运行)
- [流式与真正的取消](#流式与真正的取消)
- [预生成策略](#预生成策略)
- [预生成的当前状态：已生成（如实记录）](#预生成的当前状态已生成如实记录)
- [标注义务与准确性免责](#标注义务与准确性免责)
- [尚未具备的部分（如实记录）](#尚未具备的部分如实记录)

## BYOK：密钥从操作系统钥匙串来，不从环境变量来

云笺是 BYOK（自带密钥）：用户填自己的 API key，项目不代理任何请求、不持有任何凭据。
多供应商客户端是 `genai`（`0.6`，`default-features = false` + 显式 `rustls-tls`）。

密钥存储在 `yunjian-ai::keystore`，底层抽象是 `keyring_core::CredentialStore`。
**刻意只依赖 `keyring-core` 加逐平台 store，绝不依赖 `keyring` 门面 crate**——后者自己的文档就写
应用「should not be linking to this library at all」，且它会把五个平台的 store 一次性拖进依赖树，
也就把 Linux 上的 D-Bus 栈塞进 Windows 构建里。

`Backend` 八个变体：`SecretService`、`Keyutils`、`WindowsCredential`、`AppleKeychain`、
`AndroidKeystore`、`SessionMemory`、`PlaintextFile`、`Absent`。逐平台实际探测：

| 平台    | 首选                                              | 兜底                                         |
| ------- | ------------------------------------------------- | -------------------------------------------- |
| Linux   | `zbus_secret_service_keyring_store::Store::new()` | `linux_keyutils_keyring_store::Store::new()` |
| macOS   | `apple_native_keyring_store::keychain::Store`     | —                                            |
| iOS     | `apple_native_keyring_store::protected::Store`    | —                                            |
| Windows | `windows_native_keyring_store::Store`             | —                                            |
| Android | `android_native_keyring_store::Store`             | —                                            |

**总降级链**：操作系统钥匙串 → 若都不可用且 `allow_plaintext_file == true` 则 `PlaintextFile`
→ 否则 `SessionMemory`。**明文文件不是自动的最终兜底，而是显式 opt-in**——「装不上钥匙串就悄悄
写明文」是最容易被接受也最不该做的默认值。

Linux / 无头环境的实际链是 `SecretService → Keyutils → SessionMemory`。Secret Service 需要
D-Bus 会话**和**一个真实的 secrets 守护进程，两者在 CI、容器与纯 SSH 会话里都不存在，
所以 `Store::new()` 失败是**预期路径而不是异常**。集成测试 `linux_no_dbus.rs` 实测确认无 D-Bus 时
选中 `Backend::Keyutils` 且报告 `Persistence::LoginSession`。

**来源报告不是一个 `KeySource` 枚举，而是四个字段的 `StorageReport`：**
`backend: Backend`、`persistence: Persistence`、`protection: Protection`、`location: String`。
界面上的标签**由 `persistence` 推导，而不是由后端名推导**——理由见下节。

## Linux keyutils 是内存型，重启即失（必须如实呈现）

**这条必须显式写出来，因为把它称作「系统钥匙串」是一句假话。**

`Backend::Keyutils` 的文档原文是「Linux 内核 keyutils。**纯内存，重启即失效**」；对应的
`Persistence::LoginSession` 文档是「活到注销或重启。keyutils 属于这一档」。上游文档在无头 Linux 上
称其「strongly recommended」（它属于内核、总是可用），但同时明确要求调用方
「prepare for `Entry::get_password` to fail」。

所以：

- `settings_summary()` 对这一档输出的是「内核会话密钥环（…）：**重启或注销后失效**，届时需要重新
  输入密钥」——**产品不承诺持久化**。
- **密钥读取路径必须能处理缺失并重新索要**，这不是防御性编程而是这一档的正常行为。
- 测试 `keyutils_persistence_maps_to_login_session_not_persistent` 明确断言
  `UntilReboot → LoginSession` 且**不等于** `Persistent`。

`keyutils` 和 Secret Service 都是「钥匙串」，但**持久性根本不同**，这正是
`StorageReport.persistence` 这个字段必须存在、且界面标签必须从它推导的全部理由。

## 密钥永不出现在 Debug、日志或 URL 里

四道机制，都有测试：

- **不经环境变量。** `auth_resolver` 只返回 `Ok(Some(AuthData::Key(...)))` 或 `Err`，
  **绝不返回 `Ok(None)`**——那会触发 `genai` 的 `AuthData::FromEnv` 兜底。测试
  `the_resolver_key_reaches_the_authorization_header_without_touching_the_environment` 验证 resolver
  的密钥直接形成 `Authorization` 头，并比较调用前后所有名称含 `KEY` / `TOKEN` / `SECRET` 的环境
  变量集合不变。
- **不进 `Debug`。** `OsKeychain::fmt` 不渲染底层 `CredentialStore`；`KeyStore::fmt` 只渲染
  `service` 与 `tier_report()`，不渲染持有 `SecretString` 的层结构；`GenAiProvider::fmt` 不渲染
  捕获密钥的 `client`，只报告 `key_configured: bool` 这类非机密字段。测试
  `keystore_errors_never_render_the_key` 同时检查 `Display` 与 `Debug`，
  `debug_output_of_keystore_and_report_carries_no_secret` 检查 `KeyStore` 与 `StorageReport`。
- **不进日志。** 平台错误先经 `redact_credentials(&err.to_string())` 才进 `tracing::info!`。
- **不进 `config.toml`。** 顶层配置开了 `deny_unknown_fields`，粘一个 `api_key` 进去会直接报错而不是
  被静默丢弃。代价是前向兼容——旧二进制读不懂新配置里的新键会报错；这是刻意的取舍，不是遗漏。

脱敏器本身有两个已实测的真实缺陷，修法值得记下，因为它们说明**正例与反例必须同时有用例**：

1. `--api-key hunter2xyz`（空白分隔的命令行形态）最初完全漏过。修法是**只在键名前紧邻 `-` 时**才
   接受空白作分隔符——无条件接受空白是错的，那会把「缺少 token 配置」洗成「缺少 token <已脱敏>」。
2. 把 `Token` / `Basic` 当作可在自由文本里独立识别的认证方案词会大面积误伤中文诊断。HTTP 头大小写
   不敏感，所以不能靠区分大小写消歧，只能缩小词表：自由文本里只认 `Bearer`，其余四个词只在凭据
   键名之后才用。

**只测「密钥被洗掉」会得到一个把诊断洗成一片占位符的实现**，所以两条都留了回归护栏
（`keeps_prose_that_merely_mentions_a_credential_name`、
`redacts_named_credentials_regardless_of_value_shape`）。

## 两级缓存：随包层为什么必须与供应商无关

两张表，schema 在 `crates/yunjian-ai/schema-cache.sql`：

| 表                     | 层     | 内容                         |
| ---------------------- | ------ | ---------------------------- |
| `appreciation_shipped` | 随包层 | 预生成的赏析，随语料工件分发 |
| `appreciation_cache`   | 本地层 | 用户用自己密钥生成的结果     |

**查询顺序是本地层优先、随包层次之**；带 `style`（用户自定风格）的请求**不查随包层**——
随包文本不是按那个风格生成的，拿它冒充是错的。

**本地层的键是六项加温度的 BLAKE3**：`provider.as_str()`、`request.model()`、
`request.style().unwrap_or("")`、`request.template_version()`、`stable_id`、`grounding_digest`
（每项后加 `0` 分隔字节），最后并入 `temperature().to_bits().to_le_bytes()`。

**随包层的键只有 `(stable_id, template_version)`**——没有 provider、没有 model、没有 style、
没有 temperature，随后单独校验 `grounding_digest`；返回时 provider 被固定构造为
`ProviderId("shipped")`。

**这正是随包层「与供应商无关」的全部含义，也是它能真的省钱的原因。** 如果随包键里含 provider，
一个用 DeepSeek 的用户就拿不到用别的模型预生成的那条，随包数据集对他等于不存在——那时它是一份
只对特定供应商用户有效的装饰。测试
`shipped_hit_is_provider_independent_and_performs_zero_provider_calls` 用一个 provider id 刻意不同的
`CountingProvider`，断言文本来自随包行**且 `provider.calls() == 0`**；MCP 入口另有一条端到端测试
断言 `provider.appreciate_calls() == 0`。**这条断言是可证伪的，不是摆设。**

**淘汰只作用于 `appreciation_cache`，随包行永不淘汰**（由
`lru_eviction_removes_only_user_paid_rows` 守住）。

**如实记录一处措辞与实现不符：** 淘汰 SQL 是
`ORDER BY created_at ASC, key ASC LIMIT ?1`，命中时**不刷新** `created_at`，所以实际语义是
**按创建时间淘汰的 FIFO，不是 LRU**——尽管索引名叫 `appreciation_cache_lru(created_at, key)`。
这个取舍本身有道理：刷新 `created_at` 会把生成时间篡改成访问时间，污染溯源。真正的修法是加一个
独立的 `last_accessed_at` 列，让淘汰按它排、`created_at` 保持生成语义。**当前差别只影响命中率，
不影响正确性或省钱结论**，但索引名会误导下一个读到它的人，因此写在这里。

## 供应商边界与无密钥运行

赏析边界是 trait `AppreciationProvider`，三个方法：

```rust
async fn appreciate(&self, AppreciationRequest) -> Result<Appreciation>;
async fn appreciate_stream(&self, AppreciationRequest)
    -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>>;
fn id(&self) -> ProviderId;
```

作诗边界是 `PoemGenerationProvider::generate_poem`；组合 trait 是
`AiProvider: AppreciationProvider + PoemGenerationProvider`。
**`PoemGenerationProvider` 只有 `generate_poem`，不提供任何写入入口**——这是接口层面的隔离，
不是运行期检查。

**提示词模板是版本化的、编译期内嵌的：** `APPRECIATION_TEMPLATE_FILE = "appreciation.1.0.0.md"`、
`APPRECIATION_TEMPLATE_VERSION = "1.0.0"`，正文由 `include_str!` 内嵌。
`PromptTemplate::register(name, file_name, version, source)` 校验三件事：版本是数字形式 semver、
文件名**严格**等于 `{name}.{version}.md`、正文非空。
`AppreciationRequest::render_prompt()` 把 `{{grounding}}` 替换为语料事实块，再附加可选 style。

**模板版本进缓存键**，所以改模板不会让旧缓存冒充新模板的输出——这是「版本化」在这里的实际作用，
不是记录用途。

**无密钥运行是一等要求，不是降级。** `NullProvider` 的三个入口（赏析、流式赏析、作诗）全部返回
类型化的 `Error::AiKeyNotConfigured { provider }`。MCP 侧的行为是：

1. **先查两级缓存**——所以随包赏析在完全没有密钥时就能用；
2. 无命中且没有 provider（或 provider 返回 `AiKeyNotConfigured`）时，返回**普通结果**
   `configuration_required` 并附上设置路径 `AI_SETTINGS_PATH`，**不是工具错误**。

区别是实质的：工具错误会让 MCP 客户端把它当故障处理，而「没配密钥」是一个正常的、有明确下一步的
状态。

## 流式与真正的取消

对外统一句柄是 `yunjian_core::operation::OperationHandle`（见[架构](ARCHITECTURE.zh.md)）。
内部再用 `tokio_util::sync::CancellationToken` 把取消传播到 HTTP 流：

- worker 侧 `reporter.wait_for_stop(Duration::from_millis(2))`、sink 关闭或拒绝 item 时调用
  `cancellation.cancel()`；
- HTTP 侧在**建立流时**与**读取每个 event 时**都用 `tokio::select!` 等待
  `cancellation.cancelled()`；发送 channel 同样由 `send_unless_cancelled` 竞争取消。

**取消是真的取消，不是丢弃后续结果。** 这一点由测试
`cancelling_mid_stream_stops_chunks_within_100_ms_and_never_caches_partial_text` 断言：取消后
100 ms 内得到 `Event::Cancelled`、**零个额外 chunk**、**缓存长度为 0**。

**只有完整结果才进缓存。** 缓存写入边界是 trait `AppreciationCacheWriter`，其文档原文是
「接收且只接收完整赏析的缓存写入边界」；实现只在收到 `ChatStreamEvent::End`、完整正文非空且
**尚未取消**时才调用 `cache_writer.store_completed`。半截赏析进了缓存，用户下次看到的就是一段
被截断的文字而系统认为它是完整的——那比不缓存糟得多。

## 预生成策略

随包赏析数据集由 `cargo run -p xtask -- pregenerate` 产出。

**只用开放权重模型，不用闭源 API。这是授权链约束，不是性能偏好。** 三家闭源供应商的输出条款里
两家是未知：Anthropic 的商用条款把 Output 的权利让给客户但禁止训练竞品；**OpenAI 的对应条款未能
核实（研究期间站点返回 403）；DeepSeek 的条款完全未核实**。下载下来的开放权重不附带任何限制
「输出如何再分发」的条款，于是这条不确定性被绕开。

约束由代码强制：pregenerate 在生成任何一条记录之前校验权重许可（**只认 MIT 与 Apache-2.0**）与
运行时，配成闭源 API 供应商即中止并点明开放权重要求。

**覆盖集选择有两条路径，且首选路径优先：**

- `select_by_poem_tag()`——SQL 是
  `SELECT DISTINCT poem_id FROM poem_tag WHERE tag IN (...) ORDER BY poem_id`，按选本标签
  （唐诗三百首 / 宋词三百首 / 千家诗 / 古诗文名篇）筛选。**这是第一顺位。**
- `select_by_roster()`——落空时按 `tags.toml` 的评审名单解析 `(author, title)`。

用了哪条**写进清单**：`CoverageSelector` 两个变体 `PoemTag` / `ReviewedRoster`，稳定字符串
`"poem_tag"` / `"reviewed_roster"`。**所以降级不是静默的**，读清单的人能看出用的不是首选途径。

**用户密钥生成的赏析永不进入随包数据集。** 由 `ensure_readable_table()` 强制：预生成**只允许读**
`appreciation_shipped`，对 `appreciation_cache` **硬失败**；续跑用的
`existing_pregenerated_ids()` 同样只查 shipped 表。

清单（`DatasetManifest`）的关键字段：`template_version`、`coverage_selector`、
`generation_executed`、`not_executed_reason`、`corpus_version`。

## 预生成的当前状态：已生成（如实记录）

**当前检出里有一份 16 条的赏析数据集，每条正文都是开放权重模型的真实输出。**
这一节记录真实状态，不是目标状态。

**一、推理已执行**（2026-08-17，本机 Ollama 0.32.14 加载 `deepseek-r1:7b`，CPU 推理约
40 秒/条，16 条约 11 分钟）。实测事实：

| 事实                  | 值                                        |
| --------------------- | ----------------------------------------- |
| `generation_executed` | `true`（`not_executed_reason` 为 `null`） |
| 模型 / 许可 / 运行时  | `deepseek-r1:7b` / `MIT` / `ollama`       |
| 记录数                | 16，未生成标记 0 条                       |
| 正文长度              | 187 – 506 字                              |
| `coverage_selector`   | `poem_tag`（首选路径，非回退）            |
| 模板版本              | `1.0.0`                                   |

`--endpoint` 的 URL **必须带尾斜杠**（`http://127.0.0.1:11434/`）。不带会报网络请求失败，
`/v1` 与 `/v1/` 都报 HTTP 404 —— 三种错误读起来都像运行时没起来，实际是路径拼接问题。

**二、防伪门禁仍在，且两个方向都拦。** 未执行推理时每条正文写显式标记
`NOT_GENERATED_MARKER = "<<未生成：本条不是模型输出，需开放权重模型推理>>"`，清单标
`generation_executed = false` 并带原因，终端末行打印 NOT EXECUTED 通告；反过来，
`PregeneratedDataset::push` 也拒绝「声明已执行却带未生成标记」的记录。所以
「看起来像真跑过」在结构上蒙不过去 —— 这套机制不因本次已真跑而失效，它守的是下一次。

**三、推理型权重的思维块残留由生成入口收敛。** `deepseek-r1` 一类权重的思维块被运行时
摘掉后会留下前导空行，实测 16/16 条都以 `\n\n` 开头。`Generator::appreciate` 因此 `trim`
后再收下：判据与它自己的空正文检查一致 —— 既然「trim 后为空」算没有内容，trim 掉的那部分
就不是内容。不收敛的话，随包表里每条正文前都会带一段空白，在详情页赏析面板顶上可见。

**四、`dataset/appreciations.json` 及其摘要、清单、`.work/` 全部在 `.gitignore` 里**
（`/dataset/appreciations.json` 等规则），只有人工维护的
[`dataset/README.md`](../dataset/README.md) 入库。**这个取舍与数据集是否已生成无关**：
它是两件独立发布工件的版本策略 —— 数据集随 `corpus-v*` Release 分发，不随源码走。

**五、覆盖集 16 首。** 首选路径 `select_by_poem_tag` 在当前 `corpus/build/release/corpus.db`
上命中，`coverage_selector` 记为 `poem_tag`。目标是「唐诗三百首 + 宋词三百首 + 选本标签，
约数千首」，当前工件的 `poem_tag` 只覆盖到这 16 首；扩大覆盖集要重跑
`cargo run -p xtask -- corpus-build`（唐宋规模约 50 分钟）让更多标签落进工件，**无需改代码**。

## 标注义务与准确性免责

**AI 赏析文本是 AI 生成的，不是学术成果。** 三条义务：

1. **视觉分区。** 界面里 AI 赏析与有出处的集评**在视觉上分开呈现**，不渲染在同一个视觉层级里。
   把没有出处的生成文字排成和带卷次页码的引文一样的样子，本身就是一种误导。
2. **附「未经人工审校」的说明。** 随包数据集的每条记录 `reviewed == false`，
   `dataset/README.md` 逐条披露编造与错置风险。
3. **AI 生成的诗永远标注、永远不入库。** 用户可见标签常量是
   `GENERATED_POEM_LABEL = "AI 生成，非古人作品"`；MCP 的赏析工具注解 `title = "AI 赏析"`。

**「永不入库」由测试而不是约定强制。**
`generated_seven_character_quatrain_is_labelled_rhymed_and_never_persisted` 在生成前保存
`corpus_before` 与 `cache_before`，生成后断言**语料行数与两级缓存计数均完全不变**，
并断言 `payload["label"] == "AI 生成，非古人作品"`。

**准确性免责，说清楚它的形状：** AI 赏析可能编造典故、错置作者、误解句意，也可能把一首诗的
背景安到另一首上。它没有出处可核，因此**不能当作事实引用**。这不是免责套话——本项目的整个语料
立场就是「无法复核的分析内容不入库」，AI 赏析之所以能存在，靠的正是它被标注为 AI 生成、
与可复核的集评分区呈现、且从不写入语料。

`yunjian-ai` 的 crate 级文档把这条写成契约：所有生成内容均标注为 AI 产出，不进入语料表。

## 尚未具备的部分（如实记录）

- **覆盖集只有 16 首**，目标是数千首。当前语料工件的 `poem_tag` 只覆盖到这些；需重跑
  `corpus-build` 再跑 pregenerate 才能拿到目标覆盖集（上节第五条）。
- **移动端真机上的随包赏析仍是占位标记。** 不是代码缺口：移动端首启从
  `releases/latest/download/assets_manifest.json` 取种子，而线上 `corpus-v0.1.0` 的
  `appreciations.json` 仍是 2026-08-15 那份 16 条占位版（实测下载后核对：16 条全占位）。
  桌面端把 `YUNJIAN_ASSETS_MANIFEST` 指向本地新种子后渲染的是真赏析（真机验收
  `shipped_appreciation_without_key` 已 PASS，正文 242 字），因此**缺的是一次带新种子的
  Release，不是一行代码**。移动端判据已补上「正文不含未生成标记」，所以这条缺口在真机报告里
  是一条可见的 FAIL 而不是一条静默的 PASS。
- **淘汰是 FIFO 而非 LRU**，索引名 `appreciation_cache_lru` 与行为不符（上节）。
- **闭源供应商的输出条款两家未核实**：OpenAI 部分未核实（403），DeepSeek 完全未核实。
  **当前不阻塞任何事**——随包数据集只用开放权重，用户密钥的输出永不发布——但改变这个姿态之前
  需要有人真的读过两份条款。
- **Linux 上没有持久化承诺。** keyutils 重启即失，见上文；这不是待修缺陷，是那一档的本性，
  产品的处置是如实呈现并在缺失时重新索要。

## 相关文档

- [语料与索引](CORPUS.zh.md)——版权墙的形状，以及为什么 AI 赏析是必需而不是点缀
- [架构](ARCHITECTURE.zh.md)——长任务、事件与取消协议
- [第三方许可](../LICENSES.md)——逐资产的许可与署名
