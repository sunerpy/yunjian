[简体中文](../ARCHITECTURE.zh.md) · English

# Architecture

> **Placeholder.** Filled in by todo 72, which will cover the layering boundaries, why
> `yunjian-core` never learns Tauri exists, and the two candidate paths for mobile. What follows is
> only the set of constraints already settled.

## Settled boundaries

- **`yunjian-core` contains no `tauri::` types and no shell assumptions.** This is the only reason
  the mobile escape hatch stays free: swapping the shell must not touch the poetry logic.
- **Search is a SQLite file, not a search engine.** An FTS5 trigram index on `rusqlite`'s bundled
  SQLite, paired with a 1/2-character n-gram candidate table. No tantivy (`MmapDirectory` is
  unsupported on Android), no jieba or lindera (character-level indexing is the correct granularity
  for classical Chinese).
- **Logs go to stderr and a rolling file, never stdout.** The same binary hosts an MCP stdio server.
- **`stable_id` is the only user-facing key**, minted from a content-independent source locator, with
  an append-only event-log registry. See [Corpus and indexing](CORPUS.md).

## To be written

Layer diagram, crate dependency directions, IPC and streaming cancellation, the two branches of the
mobile gate.
