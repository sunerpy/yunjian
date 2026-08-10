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

CREATE TABLE poem_last_char (
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    line_index INTEGER NOT NULL CHECK (line_index >= 0),
    ch TEXT NOT NULL,
    PRIMARY KEY (poem_id, line_index)
) WITHOUT ROWID;

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

CREATE TABLE ngram (
    gram TEXT NOT NULL,
    stable_id TEXT NOT NULL REFERENCES poem(stable_id)
) STRICT;

CREATE TABLE tag (
    name TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TABLE poem_tag (
    poem_id TEXT NOT NULL REFERENCES poem(stable_id),
    tag TEXT NOT NULL REFERENCES tag(name),
    PRIMARY KEY (poem_id, tag)
) WITHOUT ROWID;

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
    builder_sqlite_version TEXT NOT NULL,
    integrity_check TEXT NOT NULL CHECK (integrity_check = 'ok')
);

CREATE INDEX poem_author_idx ON poem(author);
CREATE INDEX poem_dynasty_idx ON poem(dynasty);
CREATE INDEX poem_title_idx ON poem(title);
CREATE INDEX poem_ci_tune_idx ON poem(ci_tune);
CREATE INDEX poem_first_line_idx ON poem(first_line);
CREATE INDEX poem_work_group_idx ON poem(work_group);
CREATE INDEX poem_last_char_idx ON poem_last_char(ch, poem_id);
CREATE INDEX poem_rhyme_group_idx ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id);
CREATE INDEX poem_tag_idx ON poem_tag(tag, poem_id);
CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character);
CREATE INDEX ngram_gram_idx ON ngram(gram, stable_id);
