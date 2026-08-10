# FTS5 索引模式实测报告

> 本文件由 `cargo run -p xtask -- index-spike` 生成，**不要手工编辑**。机器可读版本在 `corpus/reports/index-mode.json`，todo 19 与 24 读的是那一份：建出来的索引与结论不符时构建应当失败。

## 结论

| 项                 | 值                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 选定 `detail` 模式 | **`full`**                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| 辅助 n-gram 候选表 | **启用**                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| 理由               | detail=full+ngram=on 是同时通过「每条契约达到 expect_min_hits」与「每条 p95 <= 150 ms（含外推到 853385 首发布规模）」两条硬门槛的配置中索引字节最小者（28925952 B）；detail=none 在 whole_five_char_line 类上召回不足（fts5: phrase queries are not supported (detail!=full)）；detail=full+ngram=off 召回全对但被延迟门槛否掉（q02-two-char-xiangsi 走 BareLikeFts 在发布规模下外推为 428.0 ms）；n-gram 表在 100000 首规模上把两字查询的 p95 从 52.356 ms 降到 0.525 ms（99.7x）。 |

## 选型规则（事先声明、具有约束力）

选满足下面两条的**最小**配置，体积只在两条都通过时作为 tiebreaker：

1. 每一条契约都达到其 expect_min_hits；
2. 每一条契约的 p95 <= 150 ms，且外推到 853385 首发布规模后依然 <= 150 ms。

第 2 条**含外推到 853385 首的发布规模**。只在 10k 样本上判定的话，六种配置全都能过 150 ms——那句规则一条也筛不掉，等于让抽样规模替产品做决定。外推只对扫描型路径（`BareLikeFts` / `FullScan` / `FullScanFallback`）按规模线性放大，索引定位型路径原样保留，因此这个判定是保守的：只会让扫描型配置更容易被否掉。

Tiebreaker：仅在两条门槛都通过的配置之间，取索引字节最小者。一个靠扫全表拿到正确答案的配置即便体积最小也要被否掉。

## 实测环境与样本

| 项                | 值                      |
| ----------------- | ----------------------- |
| SQLite            | 3.53.2                  |
| `page_size`       | 4096                    |
| 参考机            | linux/x86_64, 32 逻辑核 |
| 每条查询测量      | 3 轮预热 + 25 轮计时    |
| 样本首数          | 10000                   |
| 不同汉字          | 620                     |
| 正文总字数        | 239338                  |
| 嵌入的 fixture 诗 | 19 首                   |
| 合成种子          | `0x59756e4a69616e01`    |

**样本来源**：合成样本，非真实语料。字表与字频取自随仓 19 首公有领域 fixture 诗的实测字频（按频率加权抽样，保留真实汉语的长尾分布），句式沿用五言/七言/长短句三种真实形态，19 首 fixture 诗逐字嵌入以保证契约锚点存在。固定种子的 SplitMix64，同规模下逐字节可复现。不下载、不 vendor 任何上游语料——真实语料入库是 todo 11/12 的工作。

**契约**：`crates/yunjian-core/tests/queries.toml`，schema v1，37 条，18 类。

## 六种配置的实测对照

| 配置                      | 索引字节 | 其中 FTS | 其中 n-gram | n-gram 行数 | 文件字节 | 建库 ms | 命中门槛 | 样本延迟门槛 | 发布规模延迟门槛 | 最差 p95 | 最差外推 p95 |
| ------------------------- | -------- | -------- | ----------- | ----------- | -------- | ------- | -------- | ------------ | ---------------- | -------- | ------------ |
| `detail=none+ngram=off`   | 2359296  | 2359296  | 0           | 0           | 7352320  | 559     | 未通过   | 通过         | 未通过           | 4.966 ms | 423.8 ms     |
| `detail=none+ngram=on`    | 28397568 | 2359296  | 26038272    | 461559      | 33390592 | 2589    | 未通过   | 通过         | 通过             | 3.434 ms | 3.4 ms       |
| `detail=column+ngram=off` | 2887680  | 2887680  | 0           | 0           | 7880704  | 603     | 未通过   | 通过         | 未通过           | 5.041 ms | 430.2 ms     |
| `detail=column+ngram=on`  | 28925952 | 2887680  | 26038272    | 461559      | 33918976 | 2649    | 未通过   | 通过         | 通过             | 3.876 ms | 3.9 ms       |
| `detail=full+ngram=off`   | 2887680  | 2887680  | 0           | 0           | 7880704  | 608     | 通过     | 通过         | 未通过           | 5.015 ms | 428.0 ms     |
| `detail=full+ngram=on`    | 28925952 | 2887680  | 26038272    | 461559      | 33918976 | 2662    | 通过     | 通过         | 通过             | 3.533 ms | 3.5 ms       |

## 召回缺口逐条

这一节是本次 spike 的核心产出：它记录了**本来会静默上线的缺陷**。

### `detail=none+ngram=off`

缺 6 条：

| 契约 id                            | 类别                  | 期望下界 | 实际 | 原因                                                  |
| ---------------------------------- | --------------------- | -------- | ---- | ----------------------------------------------------- |
| `q10-line5-chuangqianmingyueguang` | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q11-line5-bairiyishanjin`         | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q12-line7-lianganyuansheng`       | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q13-line7-gusuchengwai`           | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q14-traditional-guopo`            | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q15-traditional-jutouwangmingyue` | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |

### `detail=none+ngram=on`

缺 6 条：

| 契约 id                            | 类别                  | 期望下界 | 实际 | 原因                                                  |
| ---------------------------------- | --------------------- | -------- | ---- | ----------------------------------------------------- |
| `q10-line5-chuangqianmingyueguang` | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q11-line5-bairiyishanjin`         | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q12-line7-lianganyuansheng`       | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q13-line7-gusuchengwai`           | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q14-traditional-guopo`            | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q15-traditional-jutouwangmingyue` | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |

### `detail=column+ngram=off`

缺 6 条：

| 契约 id                            | 类别                  | 期望下界 | 实际 | 原因                                                  |
| ---------------------------------- | --------------------- | -------- | ---- | ----------------------------------------------------- |
| `q10-line5-chuangqianmingyueguang` | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q11-line5-bairiyishanjin`         | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q12-line7-lianganyuansheng`       | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q13-line7-gusuchengwai`           | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q14-traditional-guopo`            | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q15-traditional-jutouwangmingyue` | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |

### `detail=column+ngram=on`

缺 6 条：

| 契约 id                            | 类别                  | 期望下界 | 实际 | 原因                                                  |
| ---------------------------------- | --------------------- | -------- | ---- | ----------------------------------------------------- |
| `q10-line5-chuangqianmingyueguang` | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q11-line5-bairiyishanjin`         | whole_five_char_line  | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q12-line7-lianganyuansheng`       | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q13-line7-gusuchengwai`           | whole_seven_char_line | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q14-traditional-guopo`            | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |
| `q15-traditional-jutouwangmingyue` | traditional_input     | 1        | 0    | fts5: phrase queries are not supported (detail!=full) |

### `detail=full+ngram=off`

无召回缺口。

### `detail=full+ngram=on`

无召回缺口。

## 延迟违规逐条

### 样本规模（10000 首）实测

- `detail=none+ngram=off`：无条目超出 150 ms 预算。
- `detail=none+ngram=on`：无条目超出 150 ms 预算。
- `detail=column+ngram=off`：无条目超出 150 ms 预算。
- `detail=column+ngram=on`：无条目超出 150 ms 预算。
- `detail=full+ngram=off`：无条目超出 150 ms 预算。
- `detail=full+ngram=on`：无条目超出 150 ms 预算。

### 外推到发布规模（853385 首）

这一节才是选型规则里那条延迟门槛的判据。

- `detail=none+ngram=off`：
  - `q01-two-char-mingyue`（two_char_word）走 `BareLikeFts`，样本实测 4.966 ms，外推 **423.8 ms**
  - `q02-two-char-xiangsi`（two_char_word）走 `BareLikeFts`，样本实测 4.015 ms，外推 **342.6 ms**
  - `q03-two-char-chunfeng`（two_char_word）走 `BareLikeFts`，样本实测 4.957 ms，外推 **423.0 ms**
  - `q19-rare-two-char-xuxi`（rare_char）走 `BareLikeFts`，样本实测 4.828 ms，外推 **412.0 ms**
- `detail=none+ngram=on`：无条目在发布规模下超出预算。
- `detail=column+ngram=off`：
  - `q01-two-char-mingyue`（two_char_word）走 `BareLikeFts`，样本实测 5.041 ms，外推 **430.2 ms**
  - `q02-two-char-xiangsi`（two_char_word）走 `BareLikeFts`，样本实测 4.546 ms，外推 **387.9 ms**
  - `q03-two-char-chunfeng`（two_char_word）走 `BareLikeFts`，样本实测 5.000 ms，外推 **426.7 ms**
  - `q19-rare-two-char-xuxi`（rare_char）走 `BareLikeFts`，样本实测 4.778 ms，外推 **407.7 ms**
- `detail=column+ngram=on`：无条目在发布规模下超出预算。
- `detail=full+ngram=off`：
  - `q01-two-char-mingyue`（two_char_word）走 `BareLikeFts`，样本实测 4.898 ms，外推 **418.0 ms**
  - `q02-two-char-xiangsi`（two_char_word）走 `BareLikeFts`，样本实测 5.015 ms，外推 **428.0 ms**
  - `q03-two-char-chunfeng`（two_char_word）走 `BareLikeFts`，样本实测 4.417 ms，外推 **376.9 ms**
  - `q19-rare-two-char-xuxi`（rare_char）走 `BareLikeFts`，样本实测 3.990 ms，外推 **340.5 ms**
- `detail=full+ngram=on`：无条目在发布规模下超出预算。

### 勉强达标的条目（外推后 > 75 ms 但 <= 150 ms）

这些条目形式上通过了延迟门槛，但它们之所以接近预算，是因为实测走了基表全扫。列出来是为了不让一个已知会在真实规模上吃紧的实现细节被「全部通过」四个字盖住——**todo 17 与 26 必须为它们改用规范化的多对多表（`poem_tag` / 逐句末字表），而不是沿用本 spike 里为了简化而采用的 denormalized 字符串列 + `LIKE`。**

无。

### 被延迟门槛豁免的 `FullScan` 条目

契约自己声明为 `FullScan` 的形态按定义无索引可用，方案要求把它显式标记出来「以便调用方提示用户，而不是静默耗掉几秒」——慢是它已被承认的属性，不是缺陷。逐条记在此处，使豁免可见。

| 契约 id                    | 类别              | 实际路径   | 样本 p95 | 发布规模外推 |
| -------------------------- | ----------------- | ---------- | -------- | ------------ |
| `q36-nofullrun-ming-guang` | no_three_char_run | `FullScan` | 3.247 ms | 277.1 ms     |
| `q37-nofullrun-yue-shuang` | no_three_char_run | `FullScan` | 3.533 ms | 301.5 ms     |

## 逐类命中率

| 类别                  | 条数 | `detail=none+ngram=off`  | `detail=none+ngram=on`   | `detail=column+ngram=off` | `detail=column+ngram=on` | `detail=full+ngram=off`  | `detail=full+ngram=on`   |
| --------------------- | ---- | ------------------------ | ------------------------ | ------------------------- | ------------------------ | ------------------------ | ------------------------ |
| ci_tune_lookup        | 2    | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.016 ms） | 2/2（最差 p95 0.017 ms）  | 2/2（最差 p95 0.016 ms） | 2/2（最差 p95 0.026 ms） | 2/2（最差 p95 0.022 ms） |
| ci_tune_title_lookup  | 2    | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.016 ms） | 2/2（最差 p95 0.024 ms）  | 2/2（最差 p95 0.019 ms） | 2/2（最差 p95 0.020 ms） | 2/2（最差 p95 0.020 ms） |
| first_line_prefix     | 2    | 2/2（最差 p95 0.028 ms） | 2/2（最差 p95 0.026 ms） | 2/2（最差 p95 0.021 ms）  | 2/2（最差 p95 0.026 ms） | 2/2（最差 p95 0.026 ms） | 2/2（最差 p95 0.027 ms） |
| four_char_phrase      | 2    | 2/2（最差 p95 0.075 ms） | 2/2（最差 p95 0.076 ms） | 2/2（最差 p95 0.077 ms）  | 2/2（最差 p95 0.080 ms） | 2/2（最差 p95 0.099 ms） | 2/2（最差 p95 0.090 ms） |
| last_char_lookup      | 2    | 2/2（最差 p95 0.082 ms） | 2/2（最差 p95 0.079 ms） | 2/2（最差 p95 0.110 ms）  | 2/2（最差 p95 0.121 ms） | 2/2（最差 p95 0.083 ms） | 2/2（最差 p95 0.082 ms） |
| no_three_char_run     | 2    | 2/2（最差 p95 3.368 ms） | 2/2（最差 p95 3.434 ms） | 2/2（最差 p95 3.635 ms）  | 2/2（最差 p95 3.876 ms） | 2/2（最差 p95 2.781 ms） | 2/2（最差 p95 3.533 ms） |
| punctuation_only      | 2    | 2/2（最差 p95 0.007 ms） | 2/2（最差 p95 0.009 ms） | 2/2（最差 p95 0.009 ms）  | 2/2（最差 p95 0.007 ms） | 2/2（最差 p95 0.007 ms） | 2/2（最差 p95 0.007 ms） |
| rare_char             | 2    | 2/2（最差 p95 4.828 ms） | 2/2（最差 p95 0.036 ms） | 2/2（最差 p95 4.778 ms）  | 2/2（最差 p95 0.045 ms） | 2/2（最差 p95 3.990 ms） | 2/2（最差 p95 0.038 ms） |
| rhyme_group_query     | 2    | 2/2（最差 p95 0.013 ms） | 2/2（最差 p95 0.013 ms） | 2/2（最差 p95 0.016 ms）  | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.016 ms） | 2/2（最差 p95 0.014 ms） |
| tag_query             | 2    | 2/2（最差 p95 0.018 ms） | 2/2（最差 p95 0.015 ms） | 2/2（最差 p95 0.023 ms）  | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.025 ms） | 2/2（最差 p95 0.016 ms） |
| three_char_phrase     | 2    | 2/2（最差 p95 0.035 ms） | 2/2（最差 p95 0.043 ms） | 2/2（最差 p95 0.046 ms）  | 2/2（最差 p95 0.044 ms） | 2/2（最差 p95 0.057 ms） | 2/2（最差 p95 0.053 ms） |
| title_lookup          | 2    | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.016 ms）  | 2/2（最差 p95 0.035 ms） | 2/2（最差 p95 0.017 ms） | 2/2（最差 p95 0.022 ms） |
| traditional_input     | 2    | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms）  | 0/2（最差 p95 0.000 ms） | 2/2（最差 p95 0.101 ms） | 2/2（最差 p95 0.102 ms） |
| two_char_author       | 2    | 2/2（最差 p95 0.020 ms） | 2/2（最差 p95 0.024 ms） | 2/2（最差 p95 0.020 ms）  | 2/2（最差 p95 0.026 ms） | 2/2（最差 p95 0.020 ms） | 2/2（最差 p95 0.024 ms） |
| two_char_word         | 3    | 3/3（最差 p95 4.966 ms） | 3/3（最差 p95 0.074 ms） | 3/3（最差 p95 5.041 ms）  | 3/3（最差 p95 0.075 ms） | 3/3（最差 p95 5.015 ms） | 3/3（最差 p95 0.093 ms） |
| variant_char_input    | 2    | 2/2（最差 p95 0.057 ms） | 2/2（最差 p95 0.036 ms） | 2/2（最差 p95 0.041 ms）  | 2/2（最差 p95 0.045 ms） | 2/2（最差 p95 0.057 ms） | 2/2（最差 p95 0.035 ms） |
| whole_five_char_line  | 2    | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms）  | 0/2（最差 p95 0.000 ms） | 2/2（最差 p95 0.079 ms） | 2/2（最差 p95 0.091 ms） |
| whole_seven_char_line | 2    | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms） | 0/2（最差 p95 0.000 ms）  | 0/2（最差 p95 0.000 ms） | 2/2（最差 p95 0.124 ms） | 2/2（最差 p95 0.120 ms） |

## 两字查询的 n-gram 收益与规模投射

`%明月%` 只有两个字面字符，FTS5 推不出任何 trigram 约束，「索引 LIKE」因此退化成对整个 body 列的虚表全扫——用户最常输入的形态反而最慢。下表是同一条查询在三条物理路径上的 p95。

| 样本首数 | 走 n-gram 候选表 | 裸 LIKE（FTS 虚表） | 裸 LIKE（基表） | 加速  | n-gram 行数 | n-gram 字节 |
| -------- | ---------------- | ------------------- | --------------- | ----- | ----------- | ----------- |
| 10000    | 0.083 ms         | 6.037 ms            | 4.022 ms        | 72.7x | 461559      | 26038272    |
| 50000    | 0.260 ms         | 25.926 ms           | 16.376 ms       | 99.7x | 2304752     | 130187264   |
| 100000   | 0.525 ms         | 52.356 ms           | 34.236 ms       | 99.7x | 4609093     | 262377472   |

## `EXPLAIN QUERY PLAN` 证据

方案禁止在没有 `EXPLAIN QUERY PLAN` 的情况下声称一条 LIKE 路径是「索引化」的。下面按选定配置逐条列出。`SCAN … VIRTUAL TABLE INDEX 0:L0` 里的 `L0` 是 FTS5 接受了 LIKE 约束的标记，`M1` 是 MATCH 约束，两者都不是无约束全扫；而打在基表上的 `SCAN poem` 才是真正的全表扫描。

| 契约 id                            | 归一化后              | 期望计划 | 实际路径 | 命中 | p95      | EXPLAIN QUERY PLAN                                                                                                   |
| ---------------------------------- | --------------------- | -------- | -------- | ---- | -------- | -------------------------------------------------------------------------------------------------------------------- |
| `q01-two-char-mingyue`             | `明月`                | Ngram    | Ngram    | 25   | 0.093 ms | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| `q02-two-char-xiangsi`             | `相思`                | Ngram    | Ngram    | 4    | 0.046 ms | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| `q03-two-char-chunfeng`            | `春风`                | Ngram    | Ngram    | 9    | 0.047 ms | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| `q04-two-char-author-libai`        | `李白`                | Meta     | Meta     | 6    | 0.024 ms | `SEARCH poem USING INDEX poem_author_idx (author=?)`                                                                 |
| `q05-two-char-author-dufu`         | `杜甫`                | Meta     | Meta     | 3    | 0.018 ms | `SEARCH poem USING INDEX poem_author_idx (author=?)`                                                                 |
| `q06-three-char-mingyueguang`      | `明月光`              | Match    | Match    | 1    | 0.053 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q07-three-char-gurenxi`           | `故人西`              | Match    | Match    | 1    | 0.045 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q08-four-char-bairiyishan`        | `白日依山`            | Like     | Like     | 1    | 0.090 ms | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q09-four-char-haishangmingyue`    | `海上明月`            | Like     | Like     | 1    | 0.080 ms | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q10-line5-chuangqianmingyueguang` | `床前明月光`          | Match    | Match    | 1    | 0.088 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q11-line5-bairiyishanjin`         | `白日依山尽`          | Match    | Match    | 1    | 0.091 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q12-line7-lianganyuansheng`       | `两岸猿声啼不住`      | Match    | Match    | 1    | 0.111 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q13-line7-gusuchengwai`           | `姑苏城外寒山寺`      | Match    | Match    | 1    | 0.120 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q14-traditional-guopo`            | `国破山河在`          | Match    | Match    | 1    | 0.102 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q15-traditional-jutouwangmingyue` | `举头望明月`          | Match    | Match    | 1    | 0.066 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q16-variant-cechengfeng`          | `侧成峰`              | Match    | Match    | 1    | 0.035 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q17-variant-bingsaichuan`         | `冰塞川`              | Match    | Match    | 1    | 0.033 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q18-rare-yixuxi`                  | `噫吁嚱`              | Match    | Match    | 1    | 0.033 ms | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)`                                     |
| `q19-rare-two-char-xuxi`           | `吁嚱`                | Ngram    | Ngram    | 1    | 0.038 ms | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| `q20-title-jingyesi`               | `静夜思`              | Meta     | Meta     | 1    | 0.022 ms | `SEARCH poem USING INDEX poem_title_idx (title=?)`                                                                   |
| `q21-title-chunwang`               | `春望`                | Meta     | Meta     | 1    | 0.022 ms | `SEARCH poem USING INDEX poem_title_idx (title=?)`                                                                   |
| `q22-citune-niannujiao`            | `念奴娇`              | Meta     | Meta     | 1    | 0.020 ms | `SEARCH poem USING INDEX poem_ci_tune_idx (ci_tune=?)`                                                               |
| `q23-citune-shuidiaogetou`         | `水调歌头`            | Meta     | Meta     | 1    | 0.022 ms | `SEARCH poem USING INDEX poem_ci_tune_idx (ci_tune=?)`                                                               |
| `q24-citune-title-chibihuaigu`     | `念奴娇·赤壁怀古`     | Meta     | Meta     | 1    | 0.020 ms | `SEARCH poem USING INDEX poem_title_idx (title=?)`                                                                   |
| `q25-citune-title-mingyuejishiyou` | `水调歌头·明月几时有` | Meta     | Meta     | 1    | 0.020 ms | `SEARCH poem USING INDEX poem_title_idx (title=?)`                                                                   |
| `q26-firstline-chuangqian`         | `床前`                | Meta     | Meta     | 2    | 0.027 ms | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)`                                        |
| `q27-firstline-guoposhan`          | `国破山`              | Meta     | Meta     | 1    | 0.019 ms | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)`                                        |
| `q28-lastchar-shuang`              | `霜`                  | Meta     | Meta     | 146  | 0.082 ms | `SEARCH poem_last_char USING COVERING INDEX poem_last_char_idx (ch=?)`                                               |
| `q29-lastchar-liu`                 | `流`                  | Meta     | Meta     | 138  | 0.079 ms | `SEARCH poem_last_char USING COVERING INDEX poem_last_char_idx (ch=?)`                                               |
| `q30-rhyme-xiapingqiyang`          | `下平七阳`            | Meta     | Meta     | 1    | 0.014 ms | `SEARCH poem USING INDEX poem_rhyme_idx (ANY(rhyme_book) AND rhyme_group=?)`                                         |
| `q31-rhyme-xiapingshiersqin`       | `下平十二侵`          | Meta     | Meta     | 1    | 0.012 ms | `SEARCH poem USING INDEX poem_rhyme_idx (ANY(rhyme_book) AND rhyme_group=?)`                                         |
| `q32-tag-sixiang`                  | `思乡`                | Meta     | Meta     | 2    | 0.014 ms | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)`                                                          |
| `q33-tag-biansai`                  | `边塞`                | Meta     | Meta     | 1    | 0.016 ms | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)`                                                          |
| `q34-punct-comma-period`           | ``                    | Empty    | Empty    | 0    | 0.007 ms | `SCAN poem USING COVERING INDEX sqlite_autoindex_poem_1`                                                             |
| `q35-punct-mixed`                  | ``                    | Empty    | Empty    | 0    | 0.007 ms | `SCAN poem USING COVERING INDEX sqlite_autoindex_poem_1`                                                             |
| `q36-nofullrun-ming-guang`         | `明%光`               | FullScan | FullScan | 31   | 3.247 ms | `SCAN poem`                                                                                                          |
| `q37-nofullrun-yue-shuang`         | `月%霜`               | FullScan | FullScan | 102  | 3.533 ms | `SCAN poem`                                                                                                          |

## 下游怎么消费这份结论

- **todo 19** 建 `poem_fts` 时的 `detail=` 取 `chosen_mode`（本次为 `full`），不得硬编码；建完后应比对本文件，不一致即让构建失败。
- **todo 24** 的 `len < 3` 分支按 `ngram_aux_enabled`（本次为 `true`）决定是否走辅助候选表；`len > 3` 分支是否可用 `MATCH` 取决于 `chosen_mode` 是否支持 phrase 查询，运行时从 `corpus_meta.index_detail_mode` 读，不得假定。
- **todo 22** 的语料 CI 逐条跑 `crates/yunjian-core/tests/queries.toml`，任何一条结果变化而契约未同步修改即失败。
