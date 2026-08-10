[简体中文](../CORPUS.zh.md) · English

# Corpus and indexing

> **Placeholder.** Filled in by todo 72. What follows is the set of measured conclusions later work
> must honour.

## Upstream sources

**Three** upstream sources are verified and usable (not four, as an early draft claimed):
`chinese-poetry/chinese-poetry`, `Werneror/Poetry`, `charlesix59/chinese_word_rhyme`. Per-asset
licence verdicts live in [`corpus/sources.toml`](../../corpus/sources.toml); rejections and their
reasons in [`corpus/DENYLIST.md`](../../corpus/DENYLIST.md).

Verification is per **file**: a repository-level MIT LICENSE grants rights over that repository's own
compilation work and cannot cover content it scraped or transcribed. Applying that rule flagged 10
files carrying modern annotation, appreciation or encyclopedia-style entries inside a single MIT
repository. All are withheld and never shipped.

## Identity model

`stable_id` is minted from a **content-independent** source locator, never from a hash of the body:
upstream data is known to carry thousands of uncorrected transcription errors, and an identity that
moves with the content would take appreciation caches and review history down with it. The registry
is an append-only event log (`Mint` / `ContentChanged` / `Alias`), and a shift detector **fails the
build** rather than silently reassigning a run of ids.

## Index selection (measured, binding)

Verdict: **`detail=full` with the auxiliary n-gram table enabled.** The machine-readable verdict is
[`corpus/reports/index-mode.json`](../../corpus/reports/index-mode.json), with a human-readable `.md`
beside it. A built index that disagrees with it should fail the build.

Two measurements drove it:

- `detail=none` and `detail=column` return **`hits=0`** for whole five-character lines, whole
  seven-character lines, and traditional-script input — exactly the kind of silent defect a
  three-character smoke test passes over. Planning leaned toward `none` on a third party's 2x size
  saving; measurement rejected it.
- The n-gram table takes p95 for a two-character query like 明月 from 4.97 ms to 0.074 ms (**67x**),
  at a cost of 2.36 MB → 28.9 MB of index. `%明月%` has only two literal characters, so FTS5 can
  derive no trigram constraint and an "indexed LIKE" degrades to a virtual-table full scan below
  three characters.

## To be written

Build pipeline stages, 繁简 normalization and `variant_map`, the three rhyme books, the per-entry
citation requirement for historical commentary, release artifact checksums and the import path.
