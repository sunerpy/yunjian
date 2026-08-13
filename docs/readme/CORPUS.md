[简体中文](../CORPUS.zh.md) · English

# Corpus and indexing

This document records where the corpus comes from, how each asset's licence was judged, why each
exclusion was made, the normalization pipeline, the identity model and the defect baseline.
**Every licence and revision below is taken from a real manifest file in this repository, and open
questions are labelled as open.**

## Contents

- [Upstream sources: three, each with a pinned revision and licence](#upstream-sources-three-each-with-a-pinned-revision-and-licence)
- [The exclusion list: seventeen entries, each with its reason](#the-exclusion-list-seventeen-entries-each-with-its-reason)
- [Why no modern annotation, translation or appreciation is ever ingested](#why-no-modern-annotation-translation-or-appreciation-is-ever-ingested)
- [Normalization pipeline](#normalization-pipeline)
- [Identity model: why `stable_id` must be separate from `content_hash`](#identity-model-why-stable_id-must-be-separate-from-content_hash)
- [Defect reports and the drift baseline](#defect-reports-and-the-drift-baseline)
- [Index selection (measured, binding)](#index-selection-measured-binding)
- [The shipped artifact: what is in it and what is not](#the-shipped-artifact-what-is-in-it-and-what-is-not)
- [Rhyme books (measured, settled)](#rhyme-books-measured-settled)
- [Historical commentary: a located citation is an admission requirement](#historical-commentary-a-located-citation-is-an-admission-requirement)
- [What does not exist yet (recorded honestly)](#what-does-not-exist-yet-recorded-honestly)

## Upstream sources: three, each with a pinned revision and licence

**Three** upstream sources are verified and usable — not four, as an early draft claimed. The
originally-planned fourth, `jkak/pingShuiYun`, was measured to carry no LICENSE at any revision and is
rejected (see below). Taken from [`corpus/sources.toml`](../../corpus/sources.toml):

| Source                                                                                | Pinned revision                            | Repository licence | Vendored LICENSE copy                                    | Copy SHA-256                                                       |
| ------------------------------------------------------------------------------------- | ------------------------------------------ | ------------------ | -------------------------------------------------------- | ------------------------------------------------------------------ |
| [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry)   | `b8594f81a89752241442f2ce267d6f66f96704ee` | MIT                | `corpus/licenses/chinese-poetry.LICENSE`                 | `c195319aeaa3ffcbe16aa5d26eec19eae5a42f84337dd2b3dc3c9d5ccbbd6507` |
| [`Werneror/Poetry`](https://github.com/Werneror/Poetry)                               | `4cfe49c06858e00d15f84d192fe5294295f79689` | MIT                | `corpus/licenses/Werneror-Poetry.LICENSE`                | `3c2630eb84efab60868d5195aa656b954f77d3cc1127dc886601e21cfd9fb63b` |
| [`charlesix59/chinese_word_rhyme`](https://github.com/charlesix59/chinese_word_rhyme) | `ff0e9c13fb037c43e0eaa5dc929c0fe4fa2ffb18` | MIT                | `corpus/licenses/charlesix59-chinese_word_rhyme.LICENSE` | `e1464036d0f0ca738de9ebcb697b8faaf6dc2eafd193dc98555f23b409e87599` |

**What is pinned is a revision, not a branch.** Branch names move, which is the same as not pinning.
`xtask verify-sources` in networked mode verifies "upstream bytes == vendored bytes == recorded
digest"; `--offline` verifies only the latter two.

**Verification is per asset, not per repository.** The manifest holds 68 asset verdicts, and the
three `license_class` values are distributed as **42 `public_domain`, 5 `permissive`, 21
`unverified`**:

- `public_domain` — the underlying work is pre-modern and out of copyright;
- `permissive` — the repository's own compilation or computation output, licensed by its LICENSE;
- `unverified` — the chain of rights is not established (modern publications, content scraped from a
  commercial site, modern prose of unclear origin).

Assets marked `shippable = false` never enter a distributed artifact, and **`unverified` together
with `shippable = true` is a hard failure**.

This granularity rule is not fastidiousness: applying it **immediately flagged 10 files inside a
single MIT repository** — `五代诗词/huajianji/` (48 of 50 records carry modern vernacular `notes`),
`五代诗词/nantang/`, `水墨唐诗/` (152 of 176 records carry a modern appreciation in `prologue`), and
5 files under `蒙学/` carrying modern encyclopedia-style `abstract` fields. All are marked
`unverified` and `shippable = false`.

**The counter-examples matter just as much, and the criterion is not the field name.** In
`幽梦影/youmengying.json`, 209 of 219 `comment` entries are Qing-dynasty remarks by the author's
friends; the `desc` fields in `全唐诗/authors.*.json` and the `biography` fields in `御定全唐詩` are
classical-Chinese biographical notes from the original books. All are public domain and shippable. The
only workable criterion is **whether the text itself is a pre-modern imprint or modern vernacular
prose**. `御定全唐詩`'s `notes` field exists structurally but all 88 entries are empty strings — it
looks dangerous and is in fact empty.

## The exclusion list: seventeen entries, each with its reason

The full list is [`corpus/DENYLIST.md`](../../corpus/DENYLIST.md). This section restates every entry
**because the reasons are the record of why the corpus is defensible** — a rejection list without
reasons is one the next person simply deletes.

| Rejected identifier                 | Reason                                                                                                                                                                                                                                                         |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `huajianji`                         | No LICENSE file in the repository; its `notes` field exists but was empty to begin with                                                                                                                                                                        |
| `VMIJUNV`                           | No LICENSE; the README states "for study and exchange only"; the repository is also missing Tang/Song/Ming/Qing, with the complete data only on a file-sharing service                                                                                         |
| `xcc3641/chinese-gushiwen`          | No LICENSE; the bundled `audioUrl` values are also dead                                                                                                                                                                                                        |
| `Provinm/chinese-poetry-simplified` | No LICENSE. This project performs its own traditional/simplified conversion and does not need it                                                                                                                                                               |
| `THUNLP-AIPoet`                     | None of the datasets carry a LICENSE, and they are "released for academic use only" — academic permission cannot cover this project's distribution                                                                                                             |
| `THU-CRRD`                          | Also academic-use-only. Its `pingshui_amb.pkl` is the **only** polyphone rhyme-group disambiguation data available, and still cannot be used                                                                                                                   |
| `byj233/ChinesePoetryLibrary`       | Declares MPL-2.0 while the README requires a licence for commercial use — self-contradictory, and an unresolved contradiction means unusable                                                                                                                   |
| `StewartXiang/poetry_with_labels`   | GPL-3.0, which would infect the whole application; the content was itself scraped from a commercial poetry site in 2017                                                                                                                                        |
| `sheepzh/poetry`                    | The LICENSE says MIT while the README forbids any commercial use — self-contradictory; the content is also modern poetry                                                                                                                                       |
| `Poetry_CN`                         | An OpenDataLab dataset whose platform terms state "academic purposes only, no commercial use"                                                                                                                                                                  |
| `OpenDataLab`                       | As above, rejected **at the provider level** so that the same content cannot return under a different dataset name                                                                                                                                             |
| `yht050511/gushiwen`                | Scraped from a commercial poetry site (2022-12) whose terms reserve copyright to the publisher and forbid reproduction without permission                                                                                                                      |
| `Tianyijian/GushiWenSpider`         | The scraper itself, hard-coding that site's translation endpoint; its output has the same origin as the entry above                                                                                                                                            |
| `MCGA`                              | A recitation audio corpus under CC BY-NC-SA-4.0 (the NC clause excludes this project), and only the test split was released                                                                                                                                    |
| `jkak/pingShuiYun`                  | **No LICENSE file at the pinned revision.** The plan listed it as MIT; measurement proved otherwise: `/license` returns 404, and both `LICENSE` and `LICENSE.md` have zero commits across the repository's 11 commits, with no licence statement in the README |
| `caoxingyu/chinese-gushiwen`        | Upstream is gone (HTTP 404); the licence cannot be verified                                                                                                                                                                                                    |
| `javayhu/poetry`                    | Upstream is gone (HTTP 404); the licence cannot be verified                                                                                                                                                                                                    |

Three mechanism notes about this list:

- **Matching applies only to a source's `name` / `url`, never to an asset's `path`.** This is
  deliberate: the rejected standalone repository `huajianji` shares its name with the subdirectory
  `五代诗词/huajianji/` inside the MIT repository, and matching identifiers against paths would kill
  the latter — which is handled by per-asset verdicts, not by the rejection list.
- **One list alone is not enough, because deleting a line would let a source through.**
  `verify-sources` therefore additionally asserts that all 14 identifiers in `REQUIRED_DENYLIST`
  appear in `DENYLIST.md`; removing an entry fails the build. That scenario was measured to exit 1.
- **A rejection does not have to cost a capability.** `jkak/pingShuiYun` was to provide the
  character → (tone, rhyme group) reverse index; the same capability is now derived at build time by
  inverting `Pingshui_Rhyme.json` from `charlesix59/chinese_word_rhyme` — one fewer data source, and
  the index is necessarily consistent with the rhyme data actually shipped.

## Why no modern annotation, translation or appreciation is ever ingested

The `chinese-poetry` maintainer's position in issue #227 matches this project's: derived works built
on the poems (appreciation, recitation commentary, translation) are respected and not collected, while
the poems themselves are public domain. Issue #76 further bounds the MIT grant: it covers only "the
derived compilation work product".

Legitimate analytical content therefore has exactly two sources: pre-modern **historical poetry
criticism** (public domain, each entry citing its source) and **clearly-labelled AI appreciation**.
This is why the AI feature is not a nice-to-have — it fills precisely the hole the copyright wall
leaves.

## Normalization pipeline

The pipeline's shape is dictated by measured facts, not by convenience.

**One: conversion happens only at build time; no conversion dictionary ships at runtime.** The
full-text index is built on `NormalizedRecord::body` alone (simplified). A user typing 「國破山河在」
finds the poem **not** by running a converter at runtime and **not** via a second traditional index
column, but through the `variant_map` produced by the same build: a `(src_char, dst_char)` table that
goes into the corpus and rewrites queries character by character at runtime. Two hard constraints
follow — no second index column (CJK trigram already inflates 2.2–2.6×, and duplicating a column
would blow the size budget outright); and `yunjian-core` depends on no conversion crate
(`ferrous-opencc` appears only in `yunjian-corpus`'s dependencies).

**Two: the original glyphs are kept byte for byte, because conversion amplifies upstream
transcription errors.** Upstream issue #261 records roughly 4,278 probable errors, one class of which
is visually-confusable miscoding: 「傅」 recorded as 「傳」 becomes 「传」 after conversion, making the
error harder to spot afterwards. So `NormalizedRecord::body_original` is byte-identical to the input,
and any record whose conversion is not round-trip stable receives a `conversion_unstable` finding.

**Three: `Script::Mixed` covers about 40% of the full Song poetry set (143,882 / 357,448), and this is
not an oversensitive detector.** The bodies genuinely mix `却/卻`, `烟/煙`, `峰/峯`, `凉/涼`,
`里/裏`. Any assumption that a given collection is uniformly traditional is false.

**Four: four cleaning and classification stages, each with its own failure mode and reason code:**

- **Fixing the double sentence-splitting semantics.** Splitting is now two functions with distinct
  meanings: `split_rhyme_feet` splits only on `。！？` and newlines, serving last-character, rhyme-foot
  and last-character search; `split_metrical_lines` splits on `，。！？；` and newlines and serves
  **only** genre classification. Merging them pollutes rhyme-group voting — splitting 《静夜思》 on
  commas yields four candidates 光/霜/月/乡, of which 「月」 is an entering tone in a different rhyme
  group from 光/霜/乡.
- **Placeholder-body detection.** Upstream contains whole-entry placeholder strings such as
  `无正文。`; they are **not** an empty body and `empty_body` does not catch them. The criterion is
  **whole-string equality** after joining the body (not substring — a substring test on 「空。」 would
  kill poems whose real body contains 「空」). Reason code `placeholder_body`; matches are quarantined
  rather than silently dropped.
- **Glued-line splitting.** Some records pack multiple lines into a single array element, which makes
  line count, first line and last characters all wrong. Splitting is on `。！？`, with a closing quote
  immediately after the terminal punctuation folded into that line. Reason code `glued_lines`.
- **Structural genre classification.** `poem.form` takes `wujue` / `qijue` / `wulv` / `qilv` /
  `yuefu` / `ci` / `irregular` / `unknown`, with an explicit, auditable precedence order.
  **Unequal line lengths yield `irregular`; nothing is guessed.** The yuefu marker is an additional
  dimension (the boolean `is_yuefu`) and does **not** override the structural verdict: 《黄鹤楼》
  becomes `form=qilv, is_yuefu=false` and 《将进酒》 becomes `form=irregular, is_yuefu=true`, both
  correct.

## Identity model: why `stable_id` must be separate from `content_hash`

`stable_id` is minted from a **content-independent** source locator
(`mint_stable_id(identity_anchor, first_seen_corpus_version)`), while `content_hash` is computed from
`(author, dynasty, title, body)`. They are **two independent fields** and both live on the canonical
record.

**The reason for keeping them apart is a verifiable fact, not a design preference.** Upstream data is
known to contain thousands of pending corrections (issue #261, roughly 4,278), so corrections **will**
happen. If identity were a function of content, a single typo fix would:

- change that poem's user-facing key;
- so appreciation caches and review history (`appreciation_shipped`, `appreciation_cache` and FSRS
  records all key on `stable_id`) would no longer match;
- and none of this **would raise an error** — it would present as "the poem I memorized is gone".

Hence the hard rule: **never use a content-derived identifier as a user-facing key.**
`content_hash` exists for the opposite purpose — detecting that content changed, so the registry can
record a `ContentChanged` event.

The registry `corpus/id_registry.jsonl` is an append-only event log with three legal events:

- `Mint { source_locator, stable_id, content_hash, at_corpus_version }`
- `ContentChanged { stable_id, from_content_hash, to_content_hash, at_corpus_version }`
- `Alias { stable_id, from_source_locator, to_source_locator, reason, at_corpus_version }`

**When the displacement detector sees a whole run of ids shift, it fails the build** rather than
silently reassigning — a silent renumbering is the one failure mode that can destroy every user's data
at once while leaving no trace.

Deduplication and identity grouping are **deliberately different strategies**: grouping is
conservative, deduplication is aggressive. `work_group` (`compute_work_group(body)`, **excluding the
author**) makes conflicting attributions detectable — upstream issue #232 shows one 《赤壁》 attributed
to both 杜牧 and 李商隐. `edition_group` (`compute_edition_group(author, body)`) marks textual variants
**without deleting them**. **A single body hash for deduplication is deliberately not used**: it would
silently merge exactly the conflicting-attribution cases that need to be visible.

Only `全唐诗/poet.*` and `strains/json/*` carry native upstream `id` values; `宋词`, `元曲`, `楚辞`,
`诗经` and `五代诗词` do not. So "prefer the upstream native key" covers only a small fraction, and the
displacement detector applies far more widely than the plan assumed.

## Defect reports and the drift baseline

**Two artifacts with different semantics; they are not interchangeable:**

- [`corpus/reports/defects.json`](../../corpus/reports/) — **one row per finding**. A single record may
  legitimately produce several findings: a poem that is a duplicate, has a conflicting attribution and
  a suspect length produces three.
- `corpus/reports/dispositions.json` — **one row per input record**, valued only `Shipped`,
  `Quarantined` or `Excluded`.

**The conservation identity must rest on the disposition ledger, not on findings.** Because "records
that are kept also produce findings" and "one record can produce three findings" are both true,
`poem_count + defect_count == input_rows` is **arithmetically false** — the left side counts one record
three times. The correct invariant is:

```text
count(shipped) + count(quarantined) + count(excluded) == input_rows
poem_count == count(shipped)
```

enforced by `QualityReport::check_conservation`. Splitting out `corpus-audit.db` preserved all three
identities verbatim; the two sides simply live in two files, checked together by
`db::verify_conservation_across_files`, which additionally requires both files to claim the same build
(equal `schema_version` / `corpus_version` / `source_manifest_sha256` triple) — otherwise pairing an
old audit database with a new corpus could satisfy the equation by coincidence.

The **drift baseline** [`corpus/reports/baseline.json`](../../corpus/reports/baseline.json) holds a
per-reason-code expected count plus tolerance, generated by
`xtask corpus-quality --write-baseline`. Currently `scope = "fixtures"`, `input_rows = 67`,
`poem_count = 54`:

| Reason code               | Expected | Tolerance |
| ------------------------- | -------- | --------- |
| `lossy_char`              | 3        | 10%       |
| `conversion_unstable`     | 1        | 10%       |
| `duplicate_in_group`      | 6        | 10%       |
| `conflicting_attribution` | 4        | 10%       |
| `suspect_length`          | 2        | 10%       |
| `unknown_dynasty`         | 0        | 10%       |
| `empty_body`              | 1        | 10%       |
| `placeholder_body`        | 1        | 10%       |
| `glued_lines`             | 47       | 10%       |
| `excluded_by_policy`      | 6        | **0%**    |
| `restricted_license`      | 0        | **0%**    |
| `rhyme_unresolved`        | 0        | 10%       |

Two caveats stated plainly:

- **`restricted_license` and `excluded_by_policy` have a 0% tolerance**, because they are licence
  verdicts and "slightly more" has no acceptable meaning there.
- **Tolerances floor to an integer, so small counts require exact equality in practice.** The point of
  the baseline is that an upstream bump cannot silently degrade data quality: if a number moves,
  somebody has to explain why.

One real corpus shape is worth recording, because it makes any "every poem derives at least one line"
assertion fail on real data while fixtures never catch it: **176 poems in the Tang–Song set have a
body consisting of a single `。`** (upstream empty records whose `body` is non-empty and therefore pass
the quality gate). Coverage criteria must count poems with actual body characters, using the same
`content_chars` folder rather than re-implementing the punctuation set in SQL.

## Index selection (measured, binding)

The verdict is **`detail=full` plus the n-gram auxiliary table**. The machine-readable verdict is
[`corpus/reports/index-mode.json`](../../corpus/reports/index-mode.json), with a human-readable
version alongside it; a build whose index disagrees with it should fail.

Two decisive measurements:

- `detail=none` and `detail=column` return **`hits=0`** for whole five-character lines, whole
  seven-character lines and traditional-script input — exactly the kind of silent defect a
  three-character smoke test waves through. Planning leaned towards `none` on the strength of a third
  party's size figures (a 2× saving); measurement rejected it.
- The n-gram auxiliary table takes the p95 of a two-character query such as 明月 from 4.97 ms to
  0.074 ms (**67×**), at a cost of 2.36 MB → 28.9 MB of index. The cause is that `%明月%` contains only
  two literal characters, from which FTS5 can derive no trigram constraint, so "indexed LIKE" degrades
  to a virtual-table scan below three characters.

### What the verdict means

[`corpus/reports/index-mode.json`](../../corpus/reports/index-mode.json) describes the **index shape
the runtime must have**, not the shape of the shipped artifact. The build stamps `chosen_mode` into
`corpus_meta.index_detail_mode`, and first launch builds `poem_fts` according to that column. The
verdict therefore keeps its teeth: changing it changes the index actually built at runtime, and the 37
contract queries go red at once.

## The shipped artifact: what is in it and what is not

The release is **two files plus a description**, produced by `xtask corpus-build` and
`xtask corpus-package`:

| File                             | Contents                                                        | Shipped?                                         |
| -------------------------------- | --------------------------------------------------------------- | ------------------------------------------------ |
| `yunjian-corpus-<version>.db.gz` | Bodies, authors, rhyme books, rhyme feet, variant map, metadata | Yes (211 MiB, 474,162 Tang–Song poems)           |
| `corpus-audit.db`                | The `defect` + `disposition` build-time ledger                  | No (CI artifact and optional developer download) |
| `manifest.json`                  | Compatibility range, digests, sizes, measured conclusions       | Yes (sidecar)                                    |

### Two classes of thing were moved out of the shipped artifact

**One: the build-time audit ledger.** `defect` (per-record data defects) and `disposition` (per-input
disposition) answer "what did this build drop, and why" — a question for whoever is investigating, not
for the user. They were measured at **67% of the original file** (defect 50.5% + disposition 16.8%).

**Two: three derivable search structures.** `ngram`, `poem_fts` and `poem_last_char` are all
**deterministic derivations** of `poem.body`: given the same `poem` table, any machine derives
identical rows. The application builds them locally on first launch (`yunjian_core::derive`).

| Stage                                                                | Measured on 474,162 Tang–Song poems |
| -------------------------------------------------------------------- | ----------------------------------- |
| Shipped database (neither the three structures nor the audit tables) | 603 MiB, gzip **211 MiB**           |
| First-launch derivation of `ngram` (55.73 M rows)                    | 487.5 s                             |
| First-launch derivation of `poem_last_char` (5.03 M rows)            | 30.1 s                              |
| First-launch build of `poem_fts`                                     | 53.1 s                              |
| **First-launch total**                                               | **571.8 s**                         |
| Runtime file after derivation                                        | 4,464 MiB                           |

**This is not a feature reduction.** Once first launch completes, all 37 contracts in
`crates/yunjian-core/tests/queries.toml` behave exactly as they did when the structures shipped, and
two-character queries still use the `ngram_gram_idx` covering index; the worst p95 is 22.0 ms against a
150 ms budget. `crates/yunjian-corpus/tests/first_launch_contracts.rs` pins that statement as a gate,
with a falsifiable control: remove the three structures and two-character queries fail even to
prepare.

### The size budget: 250 → 300 MB, and honestly, it was not forced

`xtask corpus-package` runs five abort assertions **before** writing any file (integrity `ok`; the
shipped database contains no diagnostic tables and no derived structures; cross-file conservation
holds; the measured conclusion is `within_budget`; the database's shape matches that conclusion), then
checks a sixth after writing (the final gzip is within budget, and if not, the files just written are
deleted), and finally **decompresses and reads back** to confirm the artifact's `corpus_meta` matches
the manifest item by item.

The budget was raised from the plan's 250 MB to **300 MB**. **Recorded honestly: after moving out the
two classes of content above, the artifact measures 211 MiB, so the original 250 MB would also have
fit** — the increase is headroom, not a way to make the current artifact pass. The corpus will grow
(new public-domain sources, commentary), and a budget that hugs the current output turns into a false
alarm at the next expansion.

The full corpus (896,127 poems) still loses no poem; it is an optional in-app download. Artifacts are
published on their own `corpus-v*` tag, separate from application releases — a corpus revision should
not force an application release, and vice versa.

## Rhyme books (measured, settled)

**The product ships two rhyme books**: 平水韵 for shi, 词林正韵 for ci. The schema slot for the third,
中华新韵 (`rhyme_book = xinyun`), has existed from day one but **is not distributed**.

### Per-asset verdicts

The source is `charlesix59/chinese_word_rhyme` (MIT, revision `ff0e9c13`). The repository is MIT as a
whole, but that licence covers only its own compilation work, so verdicts are made per file:

| Asset                                              | Underlying work                              | Verdict                    | Measured size                                  |
| -------------------------------------------------- | -------------------------------------------- | -------------------------- | ---------------------------------------------- |
| `data/Pingshui_Rhyme.json`                         | 平水韵, a pre-modern rhyme book              | Public domain, **shipped** | 105 groups / 10,671 entries / 8,232 characters |
| `data/Cilin_Rhyme.json`                            | 词林正韵 (Qing, 戈载, 1821)                  | Public domain, **shipped** | 19 parts / 5,575 entries / 5,037 characters    |
| `data/Word_Tune.json`                              | Per-character tone derived from 平水韵       | Derivative, **shipped**    | 8,232 characters                               |
| `data/Xinyun_Rhyme.json` and its four-tone variant | 中华新韵 (published 2005)                    | **Withheld**               | 14 parts / 7,693 entries                       |
| `data/Ci_Tunes.json`                               | Ci metrical patterns scraped from sou-yun.cn | **Withheld**               | 19.6 MB                                        |
| `data/Ci_Catalog.json`, `data/Word_Explain.json`   | As above, scraped from sou-yun.cn            | **Withheld**               | —                                              |

Withheld assets have **no read path in the code**: `yunjian_corpus::rhyme` accepts only paths in the
`SHIPPED_ASSETS` allowlist, and passing a withheld asset yields an error rather than data.
`xtask verify-sources` provides an independent gate: any asset with
`license_class = "unverified"` marked `shippable = true` fails verification and is named.

### Open provenance questions

Two remain unresolved. Both are handled the same way (withheld) for different reasons:

1. **The chain of rights for `Ci_Tunes.json` (the ci metrical patterns) is unverified.** The upstream
   repository ships its own `crawler/getTunes.py`, targeting the commercial site `sou-yun.cn`. The
   repository's MIT cannot license scraped content, and whether `sou-yun.cn` permits redistribution is
   **unverified**. This is the most complete ci-pattern data available, and withholding it has a real
   cost: ci phrasing is instead carried by the project's own `data/citune_rhythm.tsv`. **Recorded
   honestly: that table currently covers only 2 tunes and its basis is measured modal phrasing from
   the full Song ci corpus, not a public-domain metrical authority** — see [Voice](VOICE.md).
2. **中华新韵 is a 2005 modern publication.** Its content is very likely still in copyright, regardless
   of whether the upstream repository carries an MIT file.

Once either chain of rights is established, adding it is **a data change, not a migration** — the
enum slot, the error type and the query signature are already in place.

### Three implementation details that matter

**One: the two books nest in opposite orders.** 平水韵 is `tone → rhyme group → [characters]`;
词林正韵 is `part → tone → [characters]`. Two parsers, one output row shape
(`RhymeEntry { book, rhyme_group, tone, tone_raw, character }`). 词林正韵 merges the rising and
departing tones into "oblique" and this is **not** split apart — upstream does not carry that
information, and splitting it would be invention.

**Two: the per-character reverse index is derived at build time, not imported.** The plan intended to
use `baseCharDict.json` from `jkak/pingShuiYun`; that repository was measured to carry no LICENSE at
any revision and was rejected. The reverse index is instead obtained by inverting 平水韵, producing
records equivalent in shape to the rejected repository's (`临 -> [(平, 十二侵), (去, 二十七沁)]`). This
is in fact better: the index is necessarily consistent with the rhyme data actually shipped. 1,823
characters were measured to belong to more than one distinct (tone, rhyme group) pair.

**Three: `Word_Tune.json` is a cross-check, not an authority.** Its 8,232 character keys match the
distinct character count of 平水韵 exactly, so it is evidently a per-character reduction of the same
data — but the two disagree in **157 places**, all of the same shape: the reverse index finds both a
level and an oblique reading while upstream records only "oblique". Take 「空」: it appears in
上平一东, 上声一董 and 去声一送, so it genuinely has two readings, and upstream marks it oblique.
Trusting upstream would judge 「空山不见人」 metrically defective, so the tone dimension follows the
reverse index and the 157 disagreements go into the quality report.

### A withheld book is an error, never "does not rhyme"

A query for `rhyme_book = xinyun` returns the typed `Error::RhymeBookUnavailable` and **never an empty
result set**. The reason is that "not found" and "does not rhyme" are different statements about
metre: with an empty set, a caller cannot distinguish "these two characters are in different 中华新韵
groups" from "we do not have 中华新韵 at all", and missing data would be presented to the user as a
negative verdict — a false statement about metre.

### Two measured numbers that contradict the usual figure

- **平水韵 has 105 rhyme groups here, not the commonly-cited 106.** This upstream copy is missing the
  rising-tone group 「三讲」 (the keys jump from `二肿` straight to `四纸`). The assertion hard-codes
  105, so if upstream ever adds it, the test fails — making that a data change somebody sees.
- **"平水韵 has thirty rhyme groups" is true under exactly one reading**: the departing-tone section
  has exactly 30. Per tone the counts are 上平 15 / 下平 15 / 上声 28 / 去声 30 / 入声 17.

## Historical commentary: a located citation is an admission requirement

This channel exists for a legal reason. Datasets carrying **modern** annotation, translation or
appreciation have no defensible chain of rights, whereas **pre-modern** poetry criticism is itself out
of copyright — a Song critic writing about Tang poetry and a modern appreciation essay are two legal
categories. So the shipped corpus can only ever be the combination "public-domain source text +
pre-modern commentary with per-entry citations + clearly-labelled AI appreciation", and commentary is
the second of those.

Entries live under [`corpus/commentary/sources/`](../../corpus/commentary/); `index.json` is an
aggregate produced by `cargo xtask commentary-index`, whose `--check` mode is its drift gate. Four
admission rules, any one of which failing **rejects the entry with a named reason** — there is no such
answer as "validation failed":

- **All four `citation` fields are required.** Accepting a non-empty `work` alone would let an
  unverifiable "as some poetry-talk says" through, and auditability is the entire reason this pipeline
  exists.
- **`dynasty` must normalize to one of fifteen pre-1912 canonical keys.** The vocabulary stops at Qing,
  so 现代 / 当代 / 民国 cannot enter at the type level; `work_completed_by` adds a 1912 upper bound,
  catching modern publications disguised under a pre-modern-sounding title.
- **`source_note` must contain both a volume/chapter locator and the edition relied on.** The locator
  criterion is "one of 卷/则/条/篇/章 with a numeral adjacent" — checking only the keyword would let
  「卷帙浩繁」 pass, and checking only a numeral would miss 「卷上」.
- **The `poem` triple must resolve to exactly one `stable_id`.** Seed files carry the human-checkable
  (author, title, first line) and the build resolves it; zero matches and multiple matches both fail
  hard. Hand-writing a `stable_id` would hard-code a content address into human-maintained data, and
  one upstream reordering would make all of them wrong.

**No dataset's `Comment`-style field is ever bulk-imported.** The only dataset verified to contain
pre-modern poetry criticism carries no LICENSE of its own and states "for study and exchange only", so
it can serve only as a **pointer** to the original public-domain source, which is then cited directly;
not one word of its text has entered this repository.

The current seed set holds 487 entries covering 398 poems, drawn from 10 pre-modern works of poetry
criticism, each entry citing a **fixed revision** of a Wikisource transcription so it can be verified
character by character offline.

## What does not exist yet (recorded honestly)

- **The application-side download and landing path is not wired.** `manifest.json` already carries
  `sha256`, `size_bytes`, `min_app_version` and `schema_version`, but no code consumes them yet, and
  the import path for the shipped appreciation seed does not exist either.
- **The real publishing path of `corpus-release.yml` is unverified.** The workflow passes `actionlint`,
  but "the artifact rebuilt in CI is byte-identical to the local one" has **not** been verified — it
  depends on build reproducibility holding across hosts. The first real release must compare digests.
- **The size of the full set (896,127 poems) as an optional download is unmeasured.** `corpus-build`
  currently produces only Tang–Song (`SHIPPED_DEFAULT_SCOPE` is a constant and deliberately rejects
  `--scale`), and `measurements.json` holds only pre-split rows for the `full` scale.
- **The first-launch duration is a single measurement on a reference machine.** 571.8 s was measured on
  a 32-logical-core NVMe build machine, 85% of it in the n-gram stage. Mobile will be considerably
  slower, and until it is measured on a real device, **"about ten minutes" must not appear in
  user-facing copy**.

## Related documents

- [Architecture](ARCHITECTURE.md) — layering, search routing, corpus resolution and atomic materialization
- [AI appreciation](AI.md) — the hole the copyright wall leaves, filled by clearly-labelled AI text
- [Third-party licences](../../LICENSES.md) — per-asset licence and attribution
