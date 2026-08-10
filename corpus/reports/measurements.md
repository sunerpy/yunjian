# 索引体积与查询延迟实测（真实语料）

由 `cargo run -p xtask -- corpus-measure` 生成。**表内所有数字均为实测值**；未实测的规模显式标为 `NOT MEASURED` 并附阻塞原因，不以估算值填充。

## 预算与结论

- 声明预算：随包工件 gzip 后 <= **250 MB**，查询 p95 <= **150 ms**
- 结论：`within_budget = false`（发布规模已实测：true）
- 超预算：tang-song（474162 首）gzip 1768 MB 超体积预算，最差 p95 16.383 ms；full（896127 首）gzip 3351 MB 超体积预算，最差 p95 40.365 ms。占字节最多的是 `ngram_gram_idx`（38.5%），不是正文——按朝代缩小集合只能按比例缩小整个文件，削不掉这一项。已采用缓解措施 `no_measured_subset_fits_budget`。

### 采用的缓解措施

- 稳定键：`no_measured_subset_fits_budget`
- 措施：已实测的任何子集都不满足预算，因此「限制默认集」无法解决问题。必须由人决定：进一步缩小默认集（例如仅唐诗），或带论证地提高预算。本子命令不擅自提高预算。
- 实现方：todo 21（需先由人裁决默认集范围或预算）

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

| 规模 | 状态 | 首数 | 原始正文 MiB | poem 表 MiB | poem_fts MiB | FTS/poem | ngram MiB | ngram 行 | 索引/原文 | VACUUM 前 MiB | VACUUM 后 MiB | gzip MiB | 最差 p95 ms | 体积预算 | 延迟预算 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10k | 实测 | 10000 | 2.16 | 10.41 | 6.89 | 0.66x | 68.42 | 1184326 | 34.79x | 297.07 | 281.03 | 91.23 | 0.889 | 通过 | 通过 |
| tang-song | 实测 | 474162 | 101.78 | 494.72 | 228.65 | 0.46x | 3316.18 | 55730018 | 34.83x | 4764.38 | 4464.63 | 1768.51 | 16.383 | **超出** | 通过 |
| full | 实测 | 896127 | 198.33 | 940.38 | 442.70 | 0.47x | 6474.10 | 108657086 | 34.88x | 9092.85 | 8522.71 | 3351.91 | 40.365 | **超出** | 通过 |

## 字节去了哪里（逐表，占比降序）

只看 poem / poem_fts / ngram 三项会误判：`disposition` 台账记的是全部**输入**记录（含被排除的），与随包首数无关，却可能占掉文件的大半。优化随包体积必须先看这张表。

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

