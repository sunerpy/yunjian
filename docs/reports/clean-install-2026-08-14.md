# 净机安装验收 · 2026-08-14

> [!WARNING]
> **`all_pass` = `false`。**
> `all_pass` 的语义是**零 FAIL 且零 NOT EXECUTED**，因此只要有任何一条未执行
> 它就是 `false`。它**不能**被读成「这个产品哪里都跑得起来」。
>
> **未执行 1 条：**
>
> - `shipped_dataset_is_model_output` — 随包数据集的正文是开放权重模型的真实输出（`generation_executed=true`）

## 净环境

| 项         | 值                                                                        |
| ---------- | ------------------------------------------------------------------------- |
| 镜像       | `ubuntu:24.04`                                                            |
| 镜像摘要   | `sha256:561618e2c15bf2397621dd04f96926663a3b5616c189cf7e38db7e82f5c538ea` |
| 容器内系统 | Ubuntu 24.04.4 LTS                                                        |
| 内核       | `Linux 6.17.0-1019-aws x86_64`                                            |
| 先前状态   | 用户主目录 0 个条目（0 = 无缓存、无模型、无语料）                         |
| 断网手段   | docker run --network none（网络接口只有 lo，路由表 0 条）                 |
| 应用版本   | `0.1.0`                                                                   |
| 提交       | `ed55845094f2087492f267c014b48ef4df42139d`                                |

> [!NOTE]
> 净机是**容器**而不是全新物理机：内核与宿主共用。因此「无先前状态」「无 GPU」
> 「首启派生真的跑了一遍」这些事实在这里成立；而依赖特定发行版包管理、桌面会话
> 或真实硬件的事实不成立，本报告不对它们下结论。

## 汇总

声明 17 条 · PASS 16 · FAIL 0 · NOT EXECUTED 1

## 工件清单

| 工件                                | 字节      | SHA-256                                                            |
| ----------------------------------- | --------- | ------------------------------------------------------------------ |
| `appreciations.json`                | 8446      | `fa5dcd517794007614b0ad71ee287b77cb9d7283636962fcc81f0cf80f44b3d7` |
| `appreciations.json.sha256`         | 85        | `bae06ced9d523c36d74ee20eaf0e36e353966eabef04834c19af51af804eb94c` |
| `assets_manifest.json`              | 508       | `03efe907d2298b141623ac6025865bfaae7097d03eca6eff5e352037344bf380` |
| `yunjian-corpus-0.1.0.db.gz`        | 221353721 | `18d2e438d43bec61b7549a96b635c9a2a25e95badd31634fc9da74fad961c723` |
| `yunjian-corpus-0.1.0.db.gz.sha256` | 93        | `8f63219ffa2005e6b1109d0593f7abad85b9c34ad82cc0e3362ef65deb879b8b` |

## 逐条断言

| 断言                                                                                                                   | 段       | 裁决             | 依据                                                                                                                                                                                                                                                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------------- | -------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `corpus_artifact_checksum`<br>语料工件的 SHA-256 与随附 `.sha256` 一致（`sha256sum -c`）                               | 宿主     | **PASS**         | yunjian-corpus-0.1.0.db.gz（221353721 字节）实测 sha256 18d2e438d43bec61b7549a96b635c9a2a25e95badd31634fc9da74fad961c723 与 yunjian-corpus-0.1.0.db.gz.sha256 记录一致                                                                                                                                                                                 |
| `seed_artifact_checksum`<br>赏析种子的 SHA-256 与随附 `.sha256` 一致                                                   | 宿主     | **PASS**         | appreciations.json（8446 字节）实测 sha256 fa5dcd517794007614b0ad71ee287b77cb9d7283636962fcc81f0cf80f44b3d7 与 appreciations.json.sha256 记录一致                                                                                                                                                                                                      |
| `assets_manifest_accepted_by_app_parser`<br>统一资产清单被应用运行期那个解析器接受（`AssetsManifest::parse`）          | 宿主     | **PASS**         | `AssetsManifest::parse` 接受 /tmp/yunjian-t75-mirror/corpus-v0.1.0/assets_manifest.json；语料 schema 2 / corpus 0.1.0，种子模板 1.0.0 / 16 条                                                                                                                                                                                                          |
| `install_script_installs`<br>净机上 `install.sh` 校验摘要后装出可执行的 `yunjian`                                      | 联网容器 | **PASS**         | install.sh 退出 0，yunjian 0.1.0 已装到 /root/.local/bin/yunjian                                                                                                                                                                                                                                                                                       |
| `search_before_fetch_exits_3`<br>未取语料就 `search` 退出 3（而非 1）且消息指名 `corpus fetch`                         | 联网容器 | **PASS**         | 退出码 3 且消息指名 corpus fetch：2026-08-14T16:21:41.041665235Z INFO yunjian_core::logger: 日志已初始化 level="info" json=false timezone="local" to_file=true dir=/root/.local/share/yunjian/logs 2026-08-14T16:21:41.041831571Z ERROR yunjian_cli: 命令失败 code=CorpusUnavailable 语料库错误：找不                                                  |
| `corpus_fetch_downloads_both`<br>`corpus fetch` 下载并校验语料与赏析种子两件工件后退出 0                               | 联网容器 | **PASS**         | corpus fetch 退出 0：语料库：/root/.local/share/yunjian/corpus/corpus.db 数据版本：0.1.0 · schema 2 · 474162 首 · 构建于 2026-08-10T00:00:00Z 索引形态：detail=full · 派生分发=first_launch · 随包范围=tang-song 来源：本次从归档落地（本次运行落地） 派生结构：就绪                                                                                   |
| `assets_status_reports_both`<br>`assets status --json` 报出语料版本、种子版本与记录数                                  | 联网容器 | **PASS**         | assets status --json：{"schema_version":1,"command":"assets.status","status":"ok","warnings":[],"data":{"corpus_path":"/root/.local/share/yunjian/corpus/corpus.db","corpus_version":"0.1.0","record_count":16,"seed_corpus_version":"0.1.0","seed_template_version":"1.0.0","stale_count":0}}                                                         |
| `search_returns_results`<br>`search 明月` 返回结果并退出 0                                                             | 联网容器 | **PASS**         | search 明月 退出 0：{"schema_version":1,"command":"search","status":"ok","warnings":[],"data":{"filters":{"author":null,"dynasty":null,"rhyme_book":null},"hits":[{"author":"冯延巳","dynasty":"唐","matched_line_index":8,"poem_id":"28f16b71dd668b70","snippet":{"highlights":[{"end":2,"start":0}],"text":"明月"},                                  |
| `recite_scores_round`<br>`recite <id> --mode cloze` 对一轮打字作答给出评分并退出 0                                     | 联网容器 | **PASS**         | recite 28f16b71dd668b70 --mode cloze 退出 0 并给出评分：{"schema_version":1,"command":"recite","status":"ok","warnings":[],"data":{"answer":"床前明月光疑是地上霜举头望明月低头思故乡","author":"冯延巳","database":"/root/.local/share/yunjian/recite.db","dynasty":"唐","first_attempt":true,"                                                       |
| `mcp_handshake_and_tools_list`<br>`yunjian mcp` 完成 `initialize` 握手并回应 `tools/list`                              | 联网容器 | **PASS**         | initialize 回了 serverInfo，tools/list 回了工具表（退出码 0）：rmcp search_poem                                                                                                                                                                                                                                                                        |
| `shipped_hit_without_key`<br>**没有配置任何 API key** 时，随包集里的作品返回 `source=shipped`                          | 联网容器 | **PASS**         | 未配置任何 key，appreciate_poem(062f574ab2986a9b) 返回 source=shipped：{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"rmcp","version":"3.1.2"}}} {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"disclosure\":\"本结果包                                    |
| `cold_poem_without_key_asks_for_config`<br>没有 key 时，随包集外的作品如实返回 `configuration_required` 并给出设置路径 | 联网容器 | **PASS**         | 冷诗 28f16b71dd668b70 无 key 时返回 configuration_required 并给出设置路径：{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"rmcp","version":"3.1.2"}}} {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"disclosure\":\"本                                      |
| `offline_no_network_proved`<br>对照实验证明断网容器确实无网络（向宿主镜像的 TCP 连接失败）                             | 断网容器 | **PASS**         | 对照实验：用产品自己的 HTTP 客户端访问 http://172.17.0.1:18075 失败（corpus fetch 退出 3：2026-08-14T16:33:36.120453662Z INFO yunjian_core::logger: 日志已初始化 level="info" json=false timezone="local" to_file=true dir=/root/.local/share/yunjian/logs 2026-08-14T16:33:36.131513933Z ERROR yunj                                                   |
| `offline_dictionary_commands`<br>断网下 `search` / `show` / `author` / `rhyme` / `corpus status` 全部退出 0            | 断网容器 | **PASS**         | 断网下全部退出 0：[search 明月 --limit 3=0][author 李白=0][rhyme 七阳 --book pingshui=0][corpus status=0][show 28f16b71dd668b70=0]                                                                                                                                                                                                                     |
| `provider_zero_calls_for_shipped_poem`<br>provider 调用计数器确认随包集的作品发生 **0** 次模型调用                     | 宿主     | **PASS**         | `xtask provider-calls` 实测 0 次调用，来源 shipped；随包首 00001539ed2f02b5。**用的是 fixture 种子**（正文 `随包赏析 fixture 正文（验证缓存路径用，非模型输出，永不发布）`），因此本条证明的是缓存路径，不是产品内容                                                                                                                                   |
| `provider_one_call_for_cold_poem`<br>provider 调用计数器确认冷诗发生**恰好 1** 次模型调用（重复请求后仍是 1）          | 宿主     | **PASS**         | 冷诗 0000168330a10ebb 首次解析 1 次调用（来源 generated），重复请求后累计仍为 1（第二次走用户缓存）                                                                                                                                                                                                                                                    |
| `shipped_dataset_is_model_output`<br>随包数据集的正文是开放权重模型的真实输出（`generation_executed=true`）            | 宿主     | **NOT EXECUTED** | 数据集清单 `generation_executed=false`：未提供 --endpoint：本机没有可达的开放权重推理运行时，故只跑管道与门禁。管线、门禁与溯源字段照常校验且 16 条记录齐备，但每条正文是未生成标记，因此「用户看到的是模型写的赏析」这件事本次没有被执行。可执行条件：一个可达的本地开放权重运行时（MIT 或 Apache-2.0 权重），即 `xtask pregenerate --endpoint <URL>` |
