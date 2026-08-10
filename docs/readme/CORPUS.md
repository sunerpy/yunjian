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

## Rhyme books (measured, settled)

**The product ships two rhyme books**: 平水韵 for 诗 and 词林正韵 for 词. The schema slot for a
third, 中华新韵 (`rhyme_book = xinyun`), exists from day one but **is not shipped**.

### Per-asset verdicts

The source is `charlesix59/chinese_word_rhyme` (MIT, revision `ff0e9c13`). The repository is MIT as a
whole, but that licence covers only its own compilation work, so verdicts are per file:

| Asset                                            | Underlying work                        | Verdict                    | Measured size                          |
| ------------------------------------------------ | -------------------------------------- | -------------------------- | -------------------------------------- |
| `data/Pingshui_Rhyme.json`                       | 平水韵, a pre-modern rhyme book        | public domain, **shipped** | 105 groups / 10,671 rows / 8,232 chars |
| `data/Cilin_Rhyme.json`                          | 词林正韵 (戈载, 1821)                  | public domain, **shipped** | 19 parts / 5,575 rows / 5,037 chars    |
| `data/Word_Tune.json`                            | per-character 平仄 derived from 平水韵 | derived, **shipped**       | 8,232 chars                            |
| `data/Xinyun_Rhyme.json` + four-tone edition     | 中华新韵 (published 2005)              | **withheld**               | 14 parts / 7,693 rows                  |
| `data/Ci_Tunes.json`                             | 词谱 scraped from sou-yun.cn           | **withheld**               | 19.6 MB                                |
| `data/Ci_Catalog.json`, `data/Word_Explain.json` | likewise scraped from sou-yun.cn       | **withheld**               | —                                      |

Withheld assets have **no read path in the code**: `yunjian_corpus::rhyme` accepts only paths on the
`SHIPPED_ASSETS` allow-list, so passing a withheld asset yields an error rather than data. A separate
gate in `xtask verify-sources` fails verification, naming the asset, the moment anything with
`license_class = "unverified"` is marked `shippable = true`.

### Open provenance questions

Both are unresolved. The disposition is the same (withheld); the reasons differ:

1. **The authorization chain for `Ci_Tunes.json` (词谱) is unverified.** The upstream repository ships
   `crawler/getTunes.py`, which scrapes the commercial site `sou-yun.cn`. The repository's MIT cannot
   convey rights over scraped content, and whether `sou-yun.cn` permits redistribution is
   **unverified**. This is the most complete 词谱 data available (per-character 平仄, 句读, stanza
   breaks), so withholding it has a real cost: 词 句读 in todo 51 instead comes from a
   project-authored `data/citune_rhythm.tsv` whose every row must cite a public-domain 词谱 with a
   volume and page locator.
2. **中华新韵 is a 2005 modern publication.** Its content is very likely still in copyright,
   regardless of whether the upstream repository carries an MIT file.

Once either clears, adding it is a **data change, not a migration** — the enum slot, the error type
and the query signatures are already in place.

### Three implementation details that matter

**One, the two books nest in opposite order.** 平水韵 is `声部 -> 韵部 -> [字]`; 词林正韵 is
`部 -> 声 -> [字]`. Two parsers, one output row (`RhymeEntry { book, rhyme_group, tone, tone_raw,
character }`). 词林正韵 merges 上声 and 去声 into 仄声 and that merge is **preserved** — upstream
does not carry the distinction, so splitting it would be fabrication.

**Two, the per-character reverse index is derived at build time, not imported.** The plan originally
called for `jkak/pingShuiYun`'s `baseCharDict.json`; todo 9 measured that repository as having no
LICENSE at any revision, so it is denied. The reverse index is obtained by inverting 平水韵 instead,
and the result is equivalent to the denied repository's record shape
(`临 -> [(平, 十二侵), (去, 二十七沁)]`). This is the better arrangement: the index cannot disagree
with the rhyme-group data actually shipped. Measured: 1,823 characters belong to more than one
distinct (tone, group).

**Three, `Word_Tune.json` is a cross-check, not an authority.** Its 8,232 keys are exactly the
distinct characters of 平水韵, so it is that book's per-character reduction — but the two disagree in
**157 places**, all of one shape: the reverse index finds both level and oblique membership while
upstream records only 仄. 空, for instance, appears in 上平一东, 上声一董 and 去声一送, so it
genuinely reads both ways, yet upstream marks it oblique. Trusting upstream would judge
「空山不见人」 metrically wrong, so the tone dimension follows the reverse index and the 157
divergences go into the quality report.

### A withheld book is an error, never "does not rhyme"

A query against `rhyme_book = xinyun` returns a typed `Error::RhymeBookUnavailable` and **never an
empty result set**. "Not found" and "does not rhyme" are different claims about 格律: given an empty
set, a caller cannot distinguish "these characters are in different groups in 中华新韵" from "we do
not have 中华新韵 at all", and absent data would be presented as a negative judgment — a false
statement about the metre.

### Two measured numbers that contradict the usual figure

- **平水韵 here has 105 rhyme groups, not the usual 106.** This copy is missing 上声「三讲」 (the keys
  jump from `二肿` straight to `四纸`). The assertion pins 105, so an upstream fix breaks the test and
  becomes a visible data change.
- **"平水韵 has 30 rhyme groups" holds under exactly one reading**: 去声部 has exactly 30. The
  per-声部 breakdown is 上平 15 / 下平 15 / 上声 28 / 去声 30 / 入声 17.

## To be written

Build pipeline stages, 繁简 normalization and `variant_map`, the per-entry citation requirement for
historical commentary, release artifact checksums and the import path.
