# 语料缺陷报告

本文件由 `cargo run -p xtask -- corpus-quality` 生成，**不要手改**。

两份工件的区别是这一阶段的全部要点：本文件与 `defects.json` 是**一行一个 finding**，一条记录可以合法地带多个；`dispositions.json` 才是**一行一条输入记录**。守恒只建立在后者上。

## 处置台账

| 项 | 数 |
| --- | ---: |
| 输入记录数 | 67 |
| shipped（可分发） | 54 |
| quarantined（隔离留档） | 4 |
| excluded（策略排除） | 9 |
| poem_count | 54 |

守恒式：`54 + 4 + 9 == 67`。

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
| `placeholder_body` | 1 |
| `glued_lines` | 47 |
| `excluded_by_policy` | 6 |
| `restricted_license` | 0 |
| `rhyme_unresolved` | 0 |

**finding 总数 71 与记录数无关**，不要相加。

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

### `placeholder_body`（1 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| ae6832b94cfb6b1e | 3ff0da8b24eb | quality-fixture | 《占位正文样本》正文是上游缺失内容占位串，留档不进主表 |

### `glued_lines`（47 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| 05b7361d3da588f0 | 2da89ac5cf85 | werneror | 《人月圆·宴北人张侍御家有感》的上游正文含粘连结构行，已按句读拆分 |
| 05ecb688cb901683 | 24c05196c1cc | werneror | 《题李俨《黄菊赋》》的上游正文含粘连结构行，已按句读拆分 |
| 07de31c2f17743d7 | 604856534eb6 | chinese-poetry | 《国风/周南/关雎》的上游正文含粘连结构行，已按句读拆分 |
| 0fbaf24d1b75a0a2 | 51280ec8ddbf | chinese-poetry | 《望远行·碧砌花光照眼》的上游正文含粘连结构行，已按句读拆分 |
| 101eb61cbd46853a | f4c88128b0e2 | chinese-poetry | 《蜀先主庙》的上游正文含粘连结构行，已按句读拆分 |
| 156ba08ffafc742b | 2703c8502bcb | chinese-poetry | 《增廣賢文/上集》的上游正文含粘连结构行，已按句读拆分 |
| 16f2669247478639 | bfe40b61fe04 | werneror | 《于太原召侍臣赐宴守岁》的上游正文含粘连结构行，已按句读拆分 |
| 1ba071390604dc00 | 71d8ae0a7348 | quality-fixture | 《赤壁》的上游正文含粘连结构行，已按句读拆分 |
| 1c5aeb3f980134f3 | 8a9b6ae606ed | chinese-poetry | 《帝京篇十首 三》的上游正文含粘连结构行，已按句读拆分 |
| 20484c9311de44a0 | 00eb09e45b98 | chinese-poetry | 《幽梦影 其2 评1》的上游正文含粘连结构行，已按句读拆分 |
| 25c9a3e556b53f2b | 1dd8bb84d762 | chinese-poetry | 《橫吹曲辭 洛陽陌》的上游正文含粘连结构行，已按句读拆分 |
| 25e915632ba23a22 | 5484175bfa6d | chinese-poetry | 《导引》的上游正文含粘连结构行，已按句读拆分 |
| 33507adaa6c8e49c | f2545ad08a09 | werneror | 《杂诗二首 其一》的上游正文含粘连结构行，已按句读拆分 |
| 33a6b424e09a868f | f24b015e8905 | quality-fixture | 《听颖师弹琴》的上游正文含粘连结构行，已按句读拆分 |
| 3c227d9b440baf92 | 57427ccc1ed9 | werneror | 《春晓》的上游正文含粘连结构行，已按句读拆分 |
| 457a3755a11fc441 | 74e8af3890aa | chinese-poetry | 《幽梦影 其1》的上游正文含粘连结构行，已按句读拆分 |
| 4b3fa894b8035be7 | ad1b4489c0e5 | chinese-poetry | 《橫吹曲辭 長安道》的上游正文含粘连结构行，已按句读拆分 |
| 4ccb09f067e4f6ab | cc0e44d4e3e9 | chinese-poetry | 《帝京篇十首 二》的上游正文含粘连结构行，已按句读拆分 |
| 534bc11cc8dbf3c9 | d1bd2acf9097 | werneror | 《巴谣歌》的上游正文含粘连结构行，已按句读拆分 |
| 582f0e38c093fb12 | 431b895ea6c8 | chinese-poetry | 《菩萨蛮 其二》的上游正文含粘连结构行，已按句读拆分 |
| 585c8d213e0d1b1a | 1e5c6ebce27f | chinese-poetry | 《句》的上游正文含粘连结构行，已按句读拆分 |
| 5fe5e39d8854787d | b7a42e8a42b1 | chinese-poetry | 《橫吹曲辭 長安道》的上游正文含粘连结构行，已按句读拆分 |
| 69a2814531a0ab33 | e4ddba4d6794 | chinese-poetry | 《国风/周南/葛覃》的上游正文含粘连结构行，已按句读拆分 |
| 6ac728499301d4a3 | 85a25223e852 | chinese-poetry | 《聲律啓蒙/上卷/一 東》的上游正文含粘连结构行，已按句读拆分 |
| 724a3398fb965147 | f2eaa9efd548 | chinese-poetry | 《幼學瓊林/卷一/天文》的上游正文含粘连结构行，已按句读拆分 |
| 74262a5aae2e10e7 | a555eb56df60 | chinese-poetry | 《新嫁娘》的上游正文含粘连结构行，已按句读拆分 |
| 87b2d1dcbc9e85fd | 9dfb3709271a | werneror | 《三秦民谣》的上游正文含粘连结构行，已按句读拆分 |
| 8d76b29aee4c79fa | adde0ad9ff47 | chinese-poetry | 《诈妮子调风月・混江龙》的上游正文含粘连结构行，已按句读拆分 |
| 8d941edd5ace35b0 | 5b76e0a35e77 | werneror | 《传国玺》的上游正文含粘连结构行，已按句读拆分 |
| 908e0dedf7aa1786 | 4883d2ec4070 | chinese-poetry | 《宋太祖 小传》的上游正文含粘连结构行，已按句读拆分 |
| 9b7c7230a83e051d | 3b39ba5f68e2 | chinese-poetry | 《帝京篇十首 一》的上游正文含粘连结构行，已按句读拆分 |
| 9cb3a4178df09a0f | c33b701ac97c | chinese-poetry | 《幽梦影 其2 评2》的上游正文含粘连结构行，已按句读拆分 |
| b12fe7fcf18447de | 7fdc63a657df | chinese-poetry | 《幼學瓊林/卷一/地輿》的上游正文含粘连结构行，已按句读拆分 |
| ba8e3791252dfd51 | 2543de9c9c94 | chinese-poetry | 《古文觀止/卷一・周文/周鄭交質》的上游正文含粘连结构行，已按句读拆分 |
| c58c710fee2c55d0 | c5bfcbc952f4 | chinese-poetry | 《诈妮子调风月・仙吕/点绛唇》的上游正文含粘连结构行，已按句读拆分 |
| c6176af4956b3e74 | 55a6dd3e3b4d | chinese-poetry | 《日詩》的上游正文含粘连结构行，已按句读拆分 |
| d36d216a9a557013 | 57c238e7dc4b | chinese-poetry | 《幽梦影 其1 评2》的上游正文含粘连结构行，已按句读拆分 |
| d8b098cc275c7aa3 | f1c53dbc89b6 | chinese-poetry | 《幽梦影 其2》的上游正文含粘连结构行，已按句读拆分 |
| d94d81a06f65a3c4 | 7c39403b0f97 | chinese-poetry | 《聲律啓蒙/上卷/二 冬》的上游正文含粘连结构行，已按句读拆分 |
| e0c0da6f3710364b | d22d35a45cfd | chinese-poetry | 《应天长·一钩初月临妆镜》的上游正文含粘连结构行，已按句读拆分 |
| e16b0aab3d0d7933 | 525fbc62c9d5 | werneror | 《国人谚》的上游正文含粘连结构行，已按句读拆分 |
| f115833557832456 | 71d8ae0a7348 | quality-fixture | 《赤壁》的上游正文含粘连结构行，已按句读拆分 |
| f29e4d51557c8c7f | b94e8f9034dd | chinese-poetry | 《古文觀止/卷一・周文/鄭伯克段於鄢》的上游正文含粘连结构行，已按句读拆分 |
| f31e32da9ddaa743 | 0c2715015f7c | chinese-poetry | 《离骚》的上游正文含粘连结构行，已按句读拆分 |
| f3fe679d44eb6b44 | 3ceba19ea231 | chinese-poetry | 《菩萨蛮 其一》的上游正文含粘连结构行，已按句读拆分 |
| f8f5687a11011eb9 | c5e3a127fa74 | chinese-poetry | 《六州》的上游正文含粘连结构行，已按句读拆分 |
| f9fb0358d44d6402 | 4b9713c054d7 | chinese-poetry | 《太宗皇帝 小传》的上游正文含粘连结构行，已按句读拆分 |

### `excluded_by_policy`（6 条）

| stable_id | work_group | source | 详情 |
| --- | --- | --- | --- |
| — | — | werneror | 当代.csv：已知近现代/当代分桶，保护期未过（已知近现代/当代分桶，保护期未过；已数出 2 行，全部不入库） |
| — | — | werneror | 当代.csv：已知近现代/当代分桶，保护期未过（已知近现代/当代分桶，保护期未过；已数出 2 行，全部不入库） |
| — | — | chinese-poetry | 整文件为现代编者撰写（字段 desc），无古典正文可分离，按 sources.toml 的 shippable=false 排除 |
| — | — | chinese-poetry | 整文件为现代编者撰写（字段 desc），无古典正文可分离，按 sources.toml 的 shippable=false 排除 |
| — | — | werneror | 未来.csv：不在古典朝代白名单上（不在古典朝代白名单上；已数出 2 行，全部不入库） |
| — | — | werneror | 未来.csv：不在古典朝代白名单上（不在古典朝代白名单上；已数出 2 行，全部不入库） |

