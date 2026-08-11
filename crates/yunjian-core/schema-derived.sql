-- 首启在本机派生的检索结构。**均不随包。**
--
-- 三张结构共同的性质：它们都是 `poem.body` 的**确定性派生物**。给定同一份 `poem`
-- 表，任何机器上派生出来的内容逐行相同，因此它们属于运行时而非工件。
--
-- 实测（唐宋 474162 首，见 `corpus/reports/measurements.json`）：
--
-- | 结构 | 随包时占文件 |
-- |---|---:|
-- | `ngram` + 覆盖索引 | 76%（全量规模） |
-- | `poem_fts` | 23.2% |
-- | `poem_last_char` + 索引 | 13.3% |
--
-- 三者移出工件后，唐宋随包库 4464 MiB -> 604 MiB，gzip 446 MiB -> 213 MiB。
-- 代价是首启一次本机构建，耗时由 `derive::build_derived_indexes` 实测记录。
--
-- `poem_fts` 的 DDL 刻意不在这里：它的 `detail` 模式来自实测裁决，随构建写进
-- `corpus_meta.index_detail_mode`，首启时按那一列建表——所以它必须是格式化出来的。
-- 裁决因此仍然有牙齿：改掉它就改掉了运行时真正建出来的索引形态。

CREATE TABLE ngram (
    gram TEXT NOT NULL,
    stable_id TEXT NOT NULL REFERENCES poem(stable_id)
) STRICT;

CREATE TABLE poem_last_char (
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    line_index INTEGER NOT NULL CHECK (line_index >= 0),
    ch TEXT NOT NULL,
    PRIMARY KEY (poem_id, line_index)
) WITHOUT ROWID;

CREATE INDEX ngram_gram_idx ON ngram(gram, stable_id);
CREATE INDEX poem_last_char_idx ON poem_last_char(ch, poem_id);
