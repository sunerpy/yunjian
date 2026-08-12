CREATE TABLE IF NOT EXISTS appreciation_shipped (
    stable_id TEXT NOT NULL,
    template_version TEXT NOT NULL,
    model TEXT NOT NULL,
    model_license TEXT NOT NULL,
    grounding_digest TEXT NOT NULL,
    text TEXT NOT NULL,
    generated_at INTEGER NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0, 1)),
    PRIMARY KEY (stable_id, template_version)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS appreciation_cache (
    key BLOB PRIMARY KEY,
    stable_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    template_version TEXT NOT NULL,
    corpus_version TEXT NOT NULL,
    grounding_digest TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    tokens_in INTEGER,
    tokens_out INTEGER
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS appreciation_cache_lru
ON appreciation_cache(created_at, key);

CREATE INDEX IF NOT EXISTS appreciation_cache_poem
ON appreciation_cache(stable_id);

CREATE INDEX IF NOT EXISTS appreciation_cache_template
ON appreciation_cache(template_version);
