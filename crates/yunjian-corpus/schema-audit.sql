-- 构建期审计库 `corpus-audit.db` 的 schema。**不随包。**
--
-- 为什么单独一个文件：`defect` 与 `disposition` 是逐条输入记录的处置台账，实测合计
-- 占原随包文件的 67%（defect 50.5% + disposition 16.8%），而用户一行都不需要——
-- 它们回答的是「这次构建丢了什么、为什么丢」，是给排查的人看的。审计库作为 CI 工件
-- 与开发者可选下载存在，不进用户工件。
--
-- 拆库**没有削弱守恒**。todo 14/17 的三条等式逐字保留，只是两端现在分处两个文件，
-- 由 `db::verify_conservation_across_files` 同时打开两份核对。
--
-- `audit_meta` 存在的唯一目的是让两个文件能互相认领：它复制随包库 `corpus_meta` 的
-- 身份三元组与三个计数。没有它的话，拿一份旧审计库配一份新语料库也可能让守恒等式
-- 凑巧成立，而那正是拆库**新引入**的风险。

PRAGMA page_size = 4096;

CREATE TABLE defect (
    id INTEGER PRIMARY KEY NOT NULL,
    stable_id TEXT,
    work_group TEXT,
    reason_code TEXT NOT NULL,
    detail TEXT NOT NULL,
    source TEXT NOT NULL
);

CREATE TABLE disposition (
    source_locator TEXT PRIMARY KEY NOT NULL,
    stable_id TEXT,
    disposition TEXT NOT NULL CHECK (disposition IN ('shipped', 'quarantined', 'excluded'))
) WITHOUT ROWID;

CREATE TABLE audit_meta (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    corpus_version TEXT NOT NULL,
    source_manifest_sha256 TEXT NOT NULL CHECK (length(source_manifest_sha256) = 64),
    poem_count INTEGER NOT NULL CHECK (poem_count >= 0),
    finding_count INTEGER NOT NULL CHECK (finding_count >= 0),
    input_row_count INTEGER NOT NULL CHECK (input_row_count >= 0)
);

-- 审计库是给人查的，加索引不影响随包体积。
CREATE INDEX defect_stable_id_idx ON defect(stable_id);
CREATE INDEX defect_reason_code_idx ON defect(reason_code);
CREATE INDEX disposition_disposition_idx ON disposition(disposition);
