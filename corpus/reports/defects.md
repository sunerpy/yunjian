# 语料缺陷报告

本文件由 `cargo run -p xtask -- corpus-quality` 生成，**不要手改**。

两份工件的区别是这一阶段的全部要点：本文件与 `defects.json` 是**一行一个 finding**，一条记录可以合法地带多个；`dispositions.json` 才是**一行一条输入记录**。守恒只建立在后者上。

## 处置台账

| 项 | 数 |
| --- | ---: |
| 输入记录数 | 66 |
| shipped（可分发） | 54 |
| quarantined（隔离留档） | 3 |
| excluded（策略排除） | 9 |
| poem_count | 54 |

守恒式：`54 + 3 + 9 == 66`。

## 逐原因码 finding 数

| 原因码 | finding 数 |
| --- | ---: |
| `lossy_char` | 3 |
| `conversion_unstable` | 1 |
| `duplicate_in_group` | 6 |
| `conflicting_attribution` | 4 |
| `suspect_length` | 2 |
| `unknown_dynasty` | 0 |
| `empty_body` | 1 |
| `excluded_by_policy` | 6 |
| `restricted_license` | 0 |
| `rhyme_unresolved` | 0 |

**finding 总数 23 与记录数无关**，不要相加。

## 明细

### `lossy_char`（3 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| — | — | werneror | 《五郊乐章 赤帝徵音 雍和》的 内容 含 CJK 上下文中的半角 `?`，原字不可恢复；整条隔离留档，不进主表也不自动补字 |
| — | — | werneror | 《小重山 予可自解?霜脂粉也》的 题目、内容 含 CJK 上下文中的半角 `?`，原字不可恢复；整条隔离留档，不进主表也不自动补字 |
| — | — | werneror | 《狐援辞》的 内容 含 CJK 上下文中的半角 `?`，原字不可恢复；整条隔离留档，不进主表也不自动补字 |

### `conversion_unstable`（1 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| 33a6b424e09a868f | f24b015e8905 | quality-fixture | 经繁体往返后不同：暱暱儿女语，恩怨相尔汝。 划然变轩昂，勇士赴敌场。 |

### `duplicate_in_group`（6 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| — | 3b39ba5f68e2 | werneror | 《帝京篇十首 一》与 chinese-poetry 已收录作品同组（判重键 3b39ba5f68e2），按逐来源取舍不重复入库 |
| — | 604856534eb6 | werneror | 《关雎》与 chinese-poetry 已收录作品同组（判重键 604856534eb6），按逐来源取舍不重复入库 |
| 18ac9c381444dc78 | 99c1e5516651 | quality-fixture | 《殘句》与同组另 1 条记录重出（互见），全部保留只作标注；同组 locator：quality-fixture:aa11bb22-cc33-4d44-8e55-6f7788990011、quality-fixture:bb22cc33-dd44-4e55-8f66-778899001122 |
| 1ba071390604dc00 | 71d8ae0a7348 | quality-fixture | 《赤壁》与同组另 1 条记录重出（互见），全部保留只作标注；同组 locator：quality-fixture:0c2f2b0e-4a5f-4d21-9d3c-6d0b1f7e5a11、quality-fixture:7b9a1d64-2c33-4f08-8a17-3e5c9b2d4f88 |
| c0e8db5167fc289d | 99c1e5516651 | quality-fixture | 《殘句》与同组另 1 条记录重出（互见），全部保留只作标注；同组 locator：quality-fixture:aa11bb22-cc33-4d44-8e55-6f7788990011、quality-fixture:bb22cc33-dd44-4e55-8f66-778899001122 |
| f115833557832456 | 71d8ae0a7348 | quality-fixture | 《赤壁》与同组另 1 条记录重出（互见），全部保留只作标注；同组 locator：quality-fixture:0c2f2b0e-4a5f-4d21-9d3c-6d0b1f7e5a11、quality-fixture:7b9a1d64-2c33-4f08-8a17-3e5c9b2d4f88 |

### `conflicting_attribution`（4 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| 18ac9c381444dc78 | 99c1e5516651 | quality-fixture | 《殘句》同一作品分组内出现 2 个作者：乙、甲。本阶段不自动裁定归属，只报告。 |
| 1ba071390604dc00 | 71d8ae0a7348 | quality-fixture | 《赤壁》同一作品分组内出现 2 个作者：李商隱、杜牧。本阶段不自动裁定归属，只报告。 |
| c0e8db5167fc289d | 99c1e5516651 | quality-fixture | 《殘句》同一作品分组内出现 2 个作者：乙、甲。本阶段不自动裁定归属，只报告。 |
| f115833557832456 | 71d8ae0a7348 | quality-fixture | 《赤壁》同一作品分组内出现 2 个作者：李商隱、杜牧。本阶段不自动裁定归属，只报告。 |

### `suspect_length`（2 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| 18ac9c381444dc78 | 99c1e5516651 | quality-fixture | 《殘句》正文只有 3 个汉字，少于 4，不足以做韵脚与平仄分析 |
| c0e8db5167fc289d | 99c1e5516651 | quality-fixture | 《殘句》正文只有 3 个汉字，少于 4，不足以做韵脚与平仄分析 |

### `empty_body`（1 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| — | — | chinese-poetry | 无传作者 没有小传正文 |

### `excluded_by_policy`（6 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| — | — | werneror | 当代.csv：已知近现代/当代分桶，保护期未过（已知近现代/当代分桶，保护期未过；已数出 2 行，全部不入库） |
| — | — | werneror | 当代.csv：已知近现代/当代分桶，保护期未过（已知近现代/当代分桶，保护期未过；已数出 2 行，全部不入库） |
| — | — | chinese-poetry | 整文件为现代编者撰写（字段 desc），无古典正文可分离，按 sources.toml 的 shippable=false 排除 |
| — | — | chinese-poetry | 整文件为现代编者撰写（字段 desc），无古典正文可分离，按 sources.toml 的 shippable=false 排除 |
| — | — | werneror | 未来.csv：不在古典朝代白名单上（不在古典朝代白名单上；已数出 2 行，全部不入库） |
| — | — | werneror | 未来.csv：不在古典朝代白名单上（不在古典朝代白名单上；已数出 2 行，全部不入库） |

