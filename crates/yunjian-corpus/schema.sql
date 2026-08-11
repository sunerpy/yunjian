PRAGMA page_size = 4096;
PRAGMA foreign_keys = ON;

CREATE TABLE author (
    name TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TABLE poem (
    stable_id TEXT PRIMARY KEY NOT NULL,
    content_hash TEXT NOT NULL,
    source_locator TEXT NOT NULL UNIQUE,
    source_locator_kind TEXT NOT NULL CHECK (source_locator_kind IN ('native', 'positional')),
    genre TEXT NOT NULL CHECK (genre IN ('shi', 'ci', 'qu', 'fu', 'wen')),
    title TEXT NOT NULL,
    title_raw TEXT NOT NULL,
    ci_tune TEXT,
    author TEXT NOT NULL REFERENCES author(name),
    dynasty TEXT NOT NULL,
    dynasty_raw TEXT NOT NULL,
    body TEXT NOT NULL,
    body_original TEXT NOT NULL,
    script TEXT NOT NULL CHECK (script IN ('simplified', 'traditional', 'mixed')),
    first_line TEXT NOT NULL,
    last_chars TEXT NOT NULL CHECK (json_valid(last_chars)),
    line_count INTEGER NOT NULL CHECK (line_count >= 0),
    char_count INTEGER NOT NULL CHECK (char_count >= 0),
    provenance_source TEXT NOT NULL,
    provenance_revision TEXT NOT NULL,
    provenance_kind TEXT NOT NULL CHECK (provenance_kind IN ('原文', '集评-PD', 'AI')),
    provenance_license TEXT NOT NULL,
    provenance_license_class TEXT NOT NULL CHECK (provenance_license_class IN ('public_domain', 'permissive')),
    work_group TEXT NOT NULL,
    edition_group TEXT NOT NULL
);

CREATE TABLE commentary (
    id TEXT PRIMARY KEY NOT NULL,
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    text TEXT NOT NULL,
    citation_work TEXT NOT NULL,
    citation_author TEXT NOT NULL,
    citation_dynasty TEXT NOT NULL,
    citation_dynasty_raw TEXT NOT NULL,
    citation_work_completed_by INTEGER NOT NULL CHECK (citation_work_completed_by < 1912),
    citation_source_note TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE rhyme (
    rhyme_book TEXT NOT NULL CHECK (rhyme_book IN ('pingshui', 'cilin', 'xinyun')),
    rhyme_group TEXT NOT NULL,
    tone TEXT NOT NULL CHECK (tone IN ('level', 'rising', 'departing', 'entering', 'oblique')),
    tone_raw TEXT NOT NULL,
    character TEXT NOT NULL,
    PRIMARY KEY (rhyme_book, rhyme_group, tone, character)
) WITHOUT ROWID;

CREATE TABLE poem_rhyme_group (
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    rhyme_book TEXT NOT NULL CHECK (rhyme_book IN ('pingshui', 'cilin', 'xinyun')),
    rhyme_group TEXT NOT NULL,
    tone TEXT NOT NULL CHECK (tone IN ('level', 'rising', 'departing', 'entering', 'oblique')),
    confidence TEXT NOT NULL CHECK (confidence IN ('resolved_by_vote', 'unambiguous', 'unresolved')),
    PRIMARY KEY (poem_id, rhyme_book, rhyme_group, tone)
) WITHOUT ROWID;

CREATE TABLE variant_map (
    src_char TEXT PRIMARY KEY NOT NULL,
    dst_char TEXT NOT NULL
) WITHOUT ROWID;

-- 三张检索结构刻意不在这里：`ngram`、`poem_fts` 与 `poem_last_char` 都是 `poem.body`
-- 的确定性派生物，实测合计占随包文件的绝大部分（全量规模 ngram 约 76%，唐宋规模
-- poem_fts 23.2% + poem_last_char 13.3%）。三者不随包、首启在本机构建，DDL 由运行时
-- crate 持有（`crates/yunjian-core/schema-derived.sql`），因为它们属于运行时而非工件。
-- `last_chars` JSON 列仍在 `poem` 上，句尾字因此不依赖派生结构就能回读。

CREATE TABLE tag (
    name TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TABLE poem_tag (
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    tag TEXT NOT NULL REFERENCES tag(name),
    PRIMARY KEY (poem_id, tag)
) WITHOUT ROWID;

-- `defect` 与 `disposition` 刻意不在这里：它们是纯构建期审计台账，实测合计占
-- 随包文件的 67%，而用户一行都不需要。它们移进 `schema-audit.sql` 描述的
-- `corpus-audit.db`，作为 CI 工件与开发者可选下载，不进用户工件。
-- 守恒断言没有因此变弱，只是两端分处两个文件，见
-- `db::verify_conservation_across_files`。

CREATE TABLE corpus_meta (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    corpus_version TEXT NOT NULL,
    built_at TEXT NOT NULL,
    source_manifest_sha256 TEXT NOT NULL CHECK (length(source_manifest_sha256) = 64),
    poem_count INTEGER NOT NULL CHECK (poem_count >= 0),
    finding_count INTEGER NOT NULL CHECK (finding_count >= 0),
    input_row_count INTEGER NOT NULL CHECK (input_row_count >= 0),
    index_detail_mode TEXT NOT NULL CHECK (index_detail_mode IN ('none', 'column', 'full')),
    derived_indexes TEXT NOT NULL CHECK (derived_indexes IN ('shipped', 'first_launch')),
    shipped_scope TEXT NOT NULL CHECK (shipped_scope IN ('10k', 'tang-song', 'full')),
    builder_sqlite_version TEXT NOT NULL,
    integrity_check TEXT NOT NULL CHECK (integrity_check = 'ok')
);

CREATE INDEX poem_author_idx ON poem(author);
CREATE INDEX poem_dynasty_idx ON poem(dynasty);
CREATE INDEX poem_title_idx ON poem(title);
CREATE INDEX poem_ci_tune_idx ON poem(ci_tune);
CREATE INDEX poem_first_line_idx ON poem(first_line);
CREATE INDEX poem_work_group_idx ON poem(work_group);
CREATE INDEX poem_rhyme_group_idx ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id);
CREATE INDEX poem_tag_idx ON poem_tag(tag, poem_id);
CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character);
-- `commentary` 的主键是 `id`，而作品详情按 `poem_id` 取集评——没有这条索引，那条查询
-- 会退化成扫全表。487 行时无所谓，但集评是会持续增补的，而「详情页扫一张会长大的表」
-- 属于上线后才显现的退化，故在此就建索引。索引本身只有几十 KB，不影响随包体积。
CREATE INDEX commentary_poem_idx ON commentary(poem_id);
