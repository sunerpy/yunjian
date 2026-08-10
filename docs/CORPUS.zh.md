简体中文 · [English](readme/CORPUS.md)

# 语料与索引

> **占位文档。** 完整内容由 todo 72 填充。当前记录已经实测定案、后续 todo 必须遵守的结论。

## 上游数据源

已核实可用的上游是 **3 个**（不是早期草案里写的 4 个）：`chinese-poetry/chinese-poetry`、
`Werneror/Poetry`、`charlesix59/chinese_word_rhyme`。逐资产许可判定见
[`corpus/sources.toml`](../corpus/sources.toml)，拒绝清单与理由见
[`corpus/DENYLIST.md`](../corpus/DENYLIST.md)。

判定粒度是**单个文件**：一份仓库级 MIT LICENSE 只能授予该仓库自身整理工作的权利，
盖不住它抓取或转录来的内容。这条规则落地时在一个 MIT 仓库内部命中了 10 个夹带现代注释、
赏析或百科式条目的文件，全部扣留、不分发。

## 身份模型

`stable_id` 从**与内容无关**的 source locator 铸造，绝不从正文哈希得来——上游数据已知存在
数千处待修正的错字，内容一变身份就跟着变的方案会连带毁掉赏析缓存与复习历史。注册表是
append-only 事件日志（`Mint` / `ContentChanged` / `Alias`），位移探测器发现整段 id 平移时
**让构建失败**而不是静默重新分配。

## 索引选型（已实测，具有约束力）

结论：**`detail=full` + 启用 n-gram 辅助表**。机器可读的 verdict 在
[`corpus/reports/index-mode.json`](../corpus/reports/index-mode.json)，人类可读版在同目录的
`.md`；建出来的索引与之不符时构建应当失败。

两条关键实测：

- `detail=none` 与 `detail=column` 在整句五言、整句七言、繁体输入三类上 **`hits=0`**——
  这正是一条三字冒烟测试会全绿放过的静默缺陷。规划阶段按第三方博客数据倾向 `none`
  （2 倍体积节省），实测否掉了它。
- n-gram 辅助表把「明月」这类两字查询的 p95 从 4.97 ms 降到 0.074 ms（**67 倍**），
  代价是索引 2.36 MB → 28.9 MB。原因是 `%明月%` 只有两个字面字符，FTS5 推不出任何
  trigram 约束，所谓「索引 LIKE」在 1-2 字下退化成虚表全扫。

## 待补

构建管线各阶段、繁简规范化与 `variant_map`、三部韵书、历代集评的逐条出处要求、发布产物的
校验与导入路径。
