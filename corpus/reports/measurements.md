# 索引体积与查询延迟实测（真实语料）

由 `cargo run -p xtask -- corpus-measure` 生成。**表内所有数字均为实测值**；未实测的规模显式标为 `NOT MEASURED` 并附阻塞原因，不以估算值填充。

## 预算与结论

- 声明预算：随包工件 gzip 后 <= **300 MB**，查询 p95 <= **150 ms**
- 结论：`within_budget = true`（发布规模已实测：true）
- 随包形态实测 1 行（tang-song（474162 首）gzip 211 MB、首启派生 571.8 s、审计库另存 240 MB），全部 gzip <= 300 MB 且最差 p95 <= 150 ms，预算内。默认随包 tang-song，全量作为应用内可选下载。另有 3 行含 ngram 与审计表的实测保留在报告里，它们是拆分决策的依据而不是候选发布物。发布上限规模（full）已实测，缩小随包默认集的依据来自真实数字。

## 参考机

| 项 | 值 |
| --- | --- |
| 平台 | linux/x86_64，32 逻辑核 |
| CPU | Intel(R) Xeon(R) 6975P-C |
| 内存 | 61.8 GiB |
| 磁盘 | nvme0n1=SSD/NVMe, nvme1n1=SSD/NVMe |
| SQLite | 3.53.2 |
| 索引模式 | detail=full，n-gram 辅助表=true |
| 测量轮次 | 预热 3 + 计时 25 |

## 逐规模实测

**「形态」列决定这一行怎么读。** 标「含 ngram + 审计表」的行是拆分前的实测，当前构建器已不再产出那种文件——它们留在这里是因为正是那些数字（`ngram` 约 76%、两张审计台账合计 67%）促成了拆分，删掉它们此后就没有东西能说明为什么必须拆。预算只对「随包形态」的行成立。`ngram MiB` 与 `ngram 行` 在随包形态下量的是**首启构建之后**的运行期体积，延迟同样在首启之后测——那才是用户实际经历的性能。

| 规模 | 形态 | 状态 | 首数 | 原始正文 MiB | poem 表 MiB | poem_fts MiB | FTS/poem | ngram MiB | ngram 行 | 索引/原文 | VACUUM 前 MiB | VACUUM 后 MiB | gzip MiB | 审计库 MiB | 首启派生 s | 最差 p95 ms | 体积预算 | 延迟预算 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10k | 随包形态（去派生结构、去审计表） | NOT MEASURED | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| tang-song | 随包形态（去派生结构、去审计表） | 实测 | 474162 | 101.78 | 494.72 | 228.66 | 0.46x | 3517.44 | 55730018 | 36.81x | 624.77 | 603.84 | 211.81 | 240.12 | 571.8 | 22.009 | 通过 | 通过 |
| full | 随包形态（去派生结构、去审计表） | NOT MEASURED | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| 10k | 含 ngram + 审计表 | 实测 | 10000 | 2.16 | 10.41 | 6.89 | 0.66x | 68.42 | 1184326 | 34.79x | 297.07 | 281.03 | 91.23 | — | — | 0.889 | 通过 | 通过 |
| tang-song | 含 ngram + 审计表 | 实测 | 474162 | 101.78 | 494.72 | 228.65 | 0.46x | 3316.18 | 55730018 | 34.83x | 4764.38 | 4464.63 | 1768.51 | — | — | 16.383 | **超出** | 通过 |
| full | 含 ngram + 审计表 | 实测 | 896127 | 198.33 | 940.38 | 442.70 | 0.47x | 6474.10 | 108657086 | 34.88x | 9092.85 | 8522.71 | 3351.91 | — | — | 40.365 | **超出** | 通过 |

### 未实测的规模与阻塞原因

- **10k**（唐宋集合按 stable_id 排序的确定性前 1 万首）：本次运行未请求该规模（未传 --scale 10k）。要补测：在同一参考机上追加该规模重跑。
- **full**（chinese-poetry 全部可分发资产，加 Werneror 全部古典朝代分桶）：本次运行未请求该规模（未传 --scale full）。要补测：在同一参考机上追加该规模重跑。

## 字节去了哪里（逐表，占比降序）

只看 poem / poem_fts / ngram 三项会误判：`disposition` 台账记的是全部**输入**记录（含被排除的），与随包首数无关，却可能占掉文件的大半——这正是把它移出随包工件的依据。随包形态的行里这两张台账已经不在字节账内。

### 规模 tang-song（474162 首）

| 表/索引 | MiB | 占文件 |
| --- | --- | --- |
| `poem` | 425.65 | 70.5% |
| `poem_rhyme_group` | 40.73 | 6.7% |
| `poem_rhyme_group_idx` | 31.81 | 5.3% |
| `poem_first_line_idx` | 26.99 | 4.5% |
| `sqlite_autoindex_poem_2` | 23.75 | 3.9% |
| `poem_title_idx` | 15.56 | 2.6% |
| `sqlite_autoindex_poem_1` | 11.35 | 1.9% |
| `poem_work_group_idx` | 9.52 | 1.6% |
| `poem_author_idx` | 7.48 | 1.2% |
| `poem_dynasty_idx` | 5.43 | 0.9% |
| `poem_ci_tune_idx` | 4.10 | 0.7% |
| `rhyme` | 0.66 | 0.1% |

### 规模 10k（10000 首）

| 表/索引 | MiB | 占文件 |
| --- | --- | --- |
| `defect` | 141.82 | 50.5% |
| `disposition` | 47.18 | 16.8% |
| `ngram_gram_idx` | 34.78 | 12.4% |
| `ngram` | 33.64 | 12.0% |
| `poem` | 9.00 | 3.2% |
| `poem_fts_data` | 6.75 | 2.4% |
| `poem_last_char` | 1.38 | 0.5% |
| `poem_last_char_idx` | 1.38 | 0.5% |
| `poem_rhyme_group` | 0.86 | 0.3% |
| `poem_rhyme_group_idx` | 0.67 | 0.2% |
| `rhyme` | 0.66 | 0.2% |
| `poem_first_line_idx` | 0.54 | 0.2% |

### 规模 tang-song（474162 首）

| 表/索引 | MiB | 占文件 |
| --- | --- | --- |
| `ngram_gram_idx` | 1680.13 | 37.6% |
| `ngram` | 1636.05 | 36.6% |
| `poem` | 425.65 | 9.5% |
| `poem_fts_data` | 223.02 | 5.0% |
| `defect` | 141.82 | 3.2% |
| `poem_last_char` | 63.82 | 1.4% |
| `poem_last_char_idx` | 63.81 | 1.4% |
| `disposition` | 46.52 | 1.0% |
| `poem_rhyme_group` | 40.73 | 0.9% |
| `poem_rhyme_group_idx` | 31.81 | 0.7% |
| `poem_first_line_idx` | 26.99 | 0.6% |
| `sqlite_autoindex_poem_2` | 23.75 | 0.5% |

### 规模 full（896127 首）

| 表/索引 | MiB | 占文件 |
| --- | --- | --- |
| `ngram_gram_idx` | 3282.23 | 38.5% |
| `ngram` | 3191.86 | 37.5% |
| `poem` | 811.30 | 9.5% |
| `poem_fts_data` | 432.02 | 5.1% |
| `defect` | 145.94 | 1.7% |
| `poem_last_char` | 125.28 | 1.5% |
| `poem_last_char_idx` | 125.27 | 1.5% |
| `poem_rhyme_group` | 78.89 | 0.9% |
| `disposition` | 68.02 | 0.8% |
| `poem_rhyme_group_idx` | 61.62 | 0.7% |
| `poem_first_line_idx` | 48.12 | 0.6% |
| `sqlite_autoindex_poem_2` | 37.36 | 0.4% |

## 八条代表性查询的逐条延迟

等值类探针的绑定值取**库内最高频值**（近似最坏情形，三个规模可比），因此绑定值随规模变化是预期的。已知口径限制：在 `tang-song` 与 `full` 上，被最多首共用的首句恰好是 Werneror 的 utf8mb4 缺字记录（`□` 替换字符，`corpus/sources.toml` 已记载该上游缺陷）。它仍然是真实存在的正文、仍然走 trigram 约束路径、仍然有真实命中，所以这条延迟是有效的；但它量的是「缺字占位串」而不是一句常见诗句。该条 p95 距 150 ms 预算有两个数量级余量，结论不受影响。

### 规模 tang-song（474162 首）

| 查询 | 类型 | 命中 | p50 ms | p95 ms | EXPLAIN QUERY PLAN |
| --- | --- | --- | --- | --- | --- |
| two_char_ngram | 两字查询「明月」经 n-gram 覆盖索引 | 7291 | 17.159 | 22.009 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| three_char_match | 三字 FTS5 MATCH（trigram）「明月光」 | 80 | 0.042 | 0.053 | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| full_line_like | 整句 LIKE（trigram 约束），绑定库内最高频值「□□□□□□□，□□□□□□□。」 | 54 | 0.552 | 0.753 | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| author_lookup | 作者等值（B-tree），绑定库内最高频值「陸游」 | 9272 | 10.667 | 12.872 | `SEARCH poem USING INDEX poem_author_idx (author=?)` |
| first_line_prefix | 首句前缀（有序区间，非 LIKE），绑定库内最高频值「平生」 | 1452 | 2.028 | 2.360 | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)` |
| rhyme_group_join | 韵部连接，绑定库内最高频值「四支」 | 43237 | 17.653 | 19.146 | `SEARCH g USING COVERING INDEX poem_rhyme_group_idx (rhyme_book=? AND rhyme_group=?) / SEARCH p USING COVERING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| tag_filter | 标签过滤（规范化多对多表），poem_tag 表为空，本条为零命中基线 | 0 | 0.006 | 0.006 | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)` |
| cold_open_first_query | 冷启动后首查（每轮重开连接，不预热）「明月」 | 7291 | 16.871 | 17.828 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |

### 规模 10k（10000 首）

| 查询 | 类型 | 命中 | p50 ms | p95 ms | EXPLAIN QUERY PLAN |
| --- | --- | --- | --- | --- | --- |
| two_char_ngram | 两字查询「明月」经 n-gram 覆盖索引 | 156 | 0.124 | 0.151 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| three_char_match | 三字 FTS5 MATCH（trigram）「明月光」 | 3 | 0.009 | 0.010 | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| full_line_like | 整句 LIKE（trigram 约束），绑定库内最高频值「一双十指玉纤纤，不是风流物不拈。」 | 2 | 0.042 | 0.054 | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| author_lookup | 作者等值（B-tree），绑定库内最高频值「陸游」 | 202 | 0.050 | 0.056 | `SEARCH poem USING INDEX poem_author_idx (author=?)` |
| first_line_prefix | 首句前缀（有序区间，非 LIKE），绑定库内最高频值「平生」 | 27 | 0.012 | 0.020 | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)` |
| rhyme_group_join | 韵部连接，绑定库内最高频值「四支」 | 923 | 0.239 | 0.253 | `SEARCH g USING COVERING INDEX poem_rhyme_group_idx (rhyme_book=? AND rhyme_group=?) / SEARCH p USING COVERING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| tag_filter | 标签过滤（规范化多对多表），poem_tag 表为空，本条为零命中基线 | 0 | 0.004 | 0.004 | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)` |
| cold_open_first_query | 冷启动后首查（每轮重开连接，不预热）「明月」 | 156 | 0.669 | 0.889 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |

### 规模 tang-song（474162 首）

| 查询 | 类型 | 命中 | p50 ms | p95 ms | EXPLAIN QUERY PLAN |
| --- | --- | --- | --- | --- | --- |
| two_char_ngram | 两字查询「明月」经 n-gram 覆盖索引 | 7291 | 14.396 | 15.798 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| three_char_match | 三字 FTS5 MATCH（trigram）「明月光」 | 80 | 0.042 | 0.049 | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| full_line_like | 整句 LIKE（trigram 约束），绑定库内最高频值「□□□□□□□，□□□□□□□。」 | 54 | 0.538 | 0.670 | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| author_lookup | 作者等值（B-tree），绑定库内最高频值「陸游」 | 9272 | 9.602 | 11.555 | `SEARCH poem USING INDEX poem_author_idx (author=?)` |
| first_line_prefix | 首句前缀（有序区间，非 LIKE），绑定库内最高频值「平生」 | 1452 | 1.921 | 1.945 | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)` |
| rhyme_group_join | 韵部连接，绑定库内最高频值「四支」 | 43237 | 15.597 | 16.383 | `SEARCH g USING COVERING INDEX poem_rhyme_group_idx (rhyme_book=? AND rhyme_group=?) / SEARCH p USING COVERING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| tag_filter | 标签过滤（规范化多对多表），poem_tag 表为空，本条为零命中基线 | 0 | 0.004 | 0.004 | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)` |
| cold_open_first_query | 冷启动后首查（每轮重开连接，不预热）「明月」 | 7291 | 14.520 | 15.321 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |

### 规模 full（896127 首）

| 查询 | 类型 | 命中 | p50 ms | p95 ms | EXPLAIN QUERY PLAN |
| --- | --- | --- | --- | --- | --- |
| two_char_ngram | 两字查询「明月」经 n-gram 覆盖索引 | 17749 | 37.730 | 38.601 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| three_char_match | 三字 FTS5 MATCH（trigram）「明月光」 | 275 | 0.454 | 0.484 | `SCAN f VIRTUAL TABLE INDEX 0:M1 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| full_line_like | 整句 LIKE（trigram 约束），绑定库内最高频值「□□□□□□□，□□□□□□□。」 | 109 | 0.813 | 0.883 | `SCAN f VIRTUAL TABLE INDEX 0:L0 / SEARCH p USING INTEGER PRIMARY KEY (rowid=?)` |
| author_lookup | 作者等值（B-tree），绑定库内最高频值「陸游」 | 9272 | 10.229 | 11.882 | `SEARCH poem USING INDEX poem_author_idx (author=?)` |
| first_line_prefix | 首句前缀（有序区间，非 LIKE），绑定库内最高频值「平生」 | 2040 | 3.197 | 3.358 | `SEARCH poem USING INDEX poem_first_line_idx (first_line>? AND first_line<?)` |
| rhyme_group_join | 韵部连接，绑定库内最高频值「四支」 | 80068 | 30.329 | 32.525 | `SEARCH g USING COVERING INDEX poem_rhyme_group_idx (rhyme_book=? AND rhyme_group=?) / SEARCH p USING COVERING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |
| tag_filter | 标签过滤（规范化多对多表），poem_tag 表为空，本条为零命中基线 | 0 | 0.004 | 0.004 | `SEARCH poem_tag USING COVERING INDEX poem_tag_idx (tag=?)` |
| cold_open_first_query | 冷启动后首查（每轮重开连接，不预热）「明月」 | 17749 | 37.453 | 40.365 | `SEARCH n USING COVERING INDEX ngram_gram_idx (gram=?) / SEARCH p USING INDEX sqlite_autoindex_poem_1 (stable_id=?)` |

