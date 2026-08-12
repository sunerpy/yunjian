[简体中文](../VOICE.zh.md) · English

# Voice

**Yunjian does not assess pronunciation standard.** That is not cautious phrasing but a measured
finding plus a product ruling. This document states the evidence, what the ruling bought, and exactly
where the current capability boundary lies.

## Contents

- [Pronunciation standard is not assessed (the most important statement here)](#pronunciation-standard-is-not-assessed-the-most-important-statement-here)
- [Why: classical-Chinese ASR measures 77.01% CER](#why-classical-chinese-asr-measures-7701-cer)
- [The v1 feedback contract: guided practice, not machine scoring](#the-v1-feedback-contract-guided-practice-not-machine-scoring)
- [Models and licences](#models-and-licences)
- [Licence posture: a voice-enabled distribution is offered under GPL-3.0](#licence-posture-a-voice-enabled-distribution-is-offered-under-gpl-30)
- [The 破读 lexicon and its public-domain sourcing rule](#the-破读-lexicon-and-its-public-domain-sourcing-rule)
- [The ci rhythm table: 2 tunes, and the basis is not a metrical authority](#the-ci-rhythm-table-2-tunes-and-the-basis-is-not-a-metrical-authority)
- [Crate structure and feature boundaries](#crate-structure-and-feature-boundaries)
- [The degradation chain: every failure point returns to typed practice](#the-degradation-chain-every-failure-point-returns-to-typed-practice)
- [What does not exist yet (recorded honestly)](#what-does-not-exist-yet-recorded-honestly)

## Pronunciation standard is not assessed (the most important statement here)

**Yunjian does not assess pronunciation standard.** The product does not produce, the UI does not
show, the documentation does not claim and the MCP output does not contain any form of phonetic score,
tone-contour score, pronunciation accuracy or per-character reading grade.

This statement is enforced by **type boundaries**, not documentation discipline. A workspace-wide
search finds no `pronunciation_score`, `phoneme_score`, `tone_score`, 声韵分 or 调型分 field, type or
function. The actual mechanism is:

- `VoicePracticeFeedback` is defined verbatim as voice-practice feedback carrying no character
  accuracy, no missed-character list and no automatic grade, and has **exactly three** fields:
  `spoke: bool`, `pause_count: usize`, `relative_rhythm: RelativeRhythm`. The test
  `voice_feedback_exposes_only_activity_pauses_and_relative_rhythm` touches only those three.
- `source_guards_keep_voice_derived_text_out_of_typed_scoring` scans the distributed source and
  **forbids** six ways of smuggling a voice-derived type into typed scoring:
  `impl From<BiasedHyp> for TypedAttempt`, `impl Into<TypedAttempt> for BiasedHyp`,
  `impl Deref for BiasedHyp`, `impl AsRef<TypedAttempt> for BiasedHyp`,
  `impl From<VoicePracticeFeedback> for TypedScore` and
  `impl From<TypedScore> for VoicePracticeFeedback`; it also forbids the ASR and application paths
  from calling `TypedAttempt::new`.
- Two **compile-fail** cases pin it at compile time: `voice_feedback_cannot_be_graded.rs` attempts
  `grade_typed(feedback, ...)` (the parameter requires `&TypedScore`, so it does not compile), and
  `voice_feedback_cannot_be_a_typed_score.rs` attempts `feedback.into()`, with the expected error
  being explicitly that `TypedScore: From<VoicePracticeFeedback>` is not implemented.
- `verdict_has_exactly_two_possible_values` asserts `scoring_mode` takes only two values and writes
  `assert_ne!(v, "full", ...)` — character accuracy must never be promoted to a formal score.

**The typed path is unaffected by this constraint** — it is a deterministic comparison, and character
accuracy remains a score there. The two paths are isolated by types rather than by a boolean flag,
precisely so that "a voice result cannot enter typed scoring" holds at compile time.

## Why: classical-Chinese ASR measures 77.01% CER

The evidence is the measured data in [`docs/reports/asr-cer.json`](../reports/asr-cer.json), not an
estimate:

| Item                  | Value                                                                     |
| --------------------- | ------------------------------------------------------------------------- |
| Overall CER           | **0.7701488252278123 (77.01%)**                                           |
| Utterances measured   | 1800 (1800 planned, all measured)                                         |
| Poems / genres        | 50 / 8                                                                    |
| Threshold             | 0.1 (10%)                                                                 |
| TTS models            | `vits-melo-tts-zh_en`, `kokoro-multi-lang-v1_0`                           |
| ASR models            | `sherpa-onnx-whisper-tiny` / `-base` / `-small`                           |
| Channel conditions    | `clean`, `narrowband-8k`, `pink-20db`, `pink-10db`, `slow-110`, `fast-90` |
| Human recordings used | **No** (`human_recordings_used: false`)                                   |
| `scoring_mode`        | `guided_practice`                                                         |

**And 77.01% is an optimistic bound, not a point estimate.** The report states this itself: synthetic
speech has a single speaker, no accent, no elision, no real-room reverberation or far-field
attenuation, and more regular prosody than a human reading. The augmentations approximate **channel
and speaking rate**; they cannot approximate speaker variation. Real speakers can only be worse.

**An optimistic bound is useful in one direction only**: it suffices to **falsify** (the bound exceeds
the threshold by 7.7×, and humans will be worse) but not to **establish** (a bound that passed would
not mean humans pass).

Why it is this bad is explainable rather than a measurement error: a general Mandarin ASR language
model conflicts head-on with classical Chinese — different vocabulary and grammar, every 破读 read
wrongly, and a high density of rare characters.

**What this means for voice scoring:** the original design built voice scoring on "ASR transcript →
align against the source → compute completeness". At 77% CER the **input to that alignment is noise**,
so it is not only character accuracy that cannot be reported — **completeness is unreliable too**.
Setting the verdict to `completeness_only` was therefore itself wrong: it assumed open transcription
could still support completeness, which was never measured. That verdict has been withdrawn.

The reason this is the only available measurement: the sole public Chinese recitation corpus, `MCGA`,
is CC BY-NC-SA-4.0 and released only its test split, and the NC clause excludes this project (see
[`corpus/DENYLIST.md`](../../corpus/DENYLIST.md)). Human-speaker CER is therefore **unmeasured**;
`docs/reports/asr-cer-human.md` is the slot reserved for a future contribution and is **explicitly not
a gate**.

## The v1 feedback contract: guided practice, not machine scoring

The v1 voice interaction is **guided practice**: a per-line TTS demonstration → the user repeats →
feedback covers only whether they spoke, pauses, and relative rhythm → and at the end **the user
selects** the FSRS grade.

This does not cut voice out; the voice interaction is fully present, with "automatic machine scoring"
replaced by "demonstration + rhythm feedback + user self-assessment".

The three fields of `VoicePracticeFeedback` are the whole of the feedback:

| Field             | Type             | Meaning                                |
| ----------------- | ---------------- | -------------------------------------- |
| `spoke`           | `bool`           | whether the user was detected speaking |
| `pause_count`     | `usize`          | number of pauses detected              |
| `relative_rhythm` | `RelativeRhythm` | rhythm relative to the demonstration   |

`RelativeRhythm` has exactly three variants and deliberately carries no number: `Slower`, `Similar`,
`Faster`.

The FSRS grade is chosen directly by the user from the four `FsrsGrade` variants `Again` / `Hard` /
`Good` / `Easy` (the constant `ALL` is declared as every grade a user may pick directly). **No
automatic FSRS grading exists on the voice path**, enforced by
`schedule_source_has_no_voice_automatic_grading_path`, which checks that the scheduler's production
code contains no `VoicePracticeFeedback`, `RelativeRhythm`, `spoke` or `pause_count`.

Session orchestration carries one constraint that **cannot be dropped**: **playback and recording must
not overlap** (play a line → stop playback → record the repetition). Otherwise the system recognizes
its own speaker output perfectly and yields a false 100% coverage.

`scoring_mode` has exactly two legal values and **never includes `"full"`, nor `"completeness_only"`
any longer**:

- `guided_practice` — the v1 contract described above;
- `coverage_advisory` — permitted **only** after an independent keyword-spotting (KWS) spike clears
  pre-frozen thresholds, and even then it may show only coarse information such as "3 of 4 lines
  detected", **without restoring per-character accuracy, a missed-character list or automatic FSRS
  grading**.

Two honesty gates hold this value domain: `verdict_has_exactly_two_possible_values` judges the
constructed verdict, and `the_shipped_report_declares_a_legal_scoring_mode` judges the written report.

## Models and licences

**No model weights are shipped.** Weights are downloaded on demand and verified locally; the repository
holds only identity and licence records ([`models.toml`](../../models.toml), enforced by
`cargo run -p xtask -- verify-models`).

Verdicts are made per **release package**, not per model family — an upstream repository's declaration
covers only its own output, not what it converted or redistributed. This is the same rule the corpus
uses. **The `license` field itself is not trusted**: the gate opens the vendored evidence file and
checks the SPDX marker, and a field that disagrees with its evidence fails.

| Package                                                      | Kind | Role       | Licence    | Evidence form      | Underlying weights                            | Archive size  |
| ------------------------------------------------------------ | ---- | ---------- | ---------- | ------------------ | --------------------------------------------- | ------------- |
| `sherpa-onnx-whisper-tiny`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper tiny                           | 116,204,861 B |
| `sherpa-onnx-whisper-base`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper base                           | 207,557,382 B |
| `sherpa-onnx-whisper-small`                                  | asr  | production | MIT        | `upstream_license` | OpenAI Whisper small                          | 639,387,718 B |
| `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` | asr  | production | Apache-2.0 | `model_card`       | `pfluo/k2fsa-zipformer-chinese-english-mixed` | 511,274,346 B |
| `vits-melo-tts-zh_en`                                        | tts  | production | MIT        | `package_license`  | MyShell.ai MeloTTS                            | 167,006,755 B |
| `kokoro-multi-lang-v1_0`                                     | tts  | production | Apache-2.0 | `package_license`  | `hexgrad/Kokoro-82M`                          | 349,418,188 B |
| `kitten-nano-en-v0_2-fp16`                                   | tts  | **smoke**  | Apache-2.0 | `package_license`  | `KittenML/kitten-tts-nano-0.2`                | 26,586,708 B  |

Per-package digests, the pinned `license_rev` (a 40-hex commit SHA — a branch name moves and is
therefore no pin at all) and the SHA-256 of each vendored evidence copy all live in `models.toml`. The
attribution copies used for distribution are in [`licenses/`](../../licenses/), asserted byte-identical
to the evidence copies by `licenses_directory_holds_a_file_for_every_manifest_entry`. The full
third-party licence inventory is [`LICENSES.md`](../../LICENSES.md).

**Three qualifications that must be stated:**

- **The Whisper conversion packages carry no LICENSE of their own.** The evidence form is
  `upstream_license`: OpenAI's `openai/whisper` LICENSE states that the weights are released under
  MIT, the ONNX export is a format conversion, and the MIT terms carry over. That chain is written out
  in the note rather than left for the reader to infer.
- **`kitten-nano-en-v0_2-fp16` is an English smoke model and never enters the Chinese product path.**
  It remains in the manifest only because the rule "every weight the product touches has a licence
  record" admits no exception.
- **Streaming recognition is licence-clear but not implemented.**
  `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` is the only streaming candidate whose
  licence chain is complete at both ends, but `sherpa-rs` 0.6.8 wraps only the offline recognizer; its
  CER is **NOT MEASURED**.

**Verification overturned a research-phase conclusion.** The original conclusion was that ASR-side
licensing was broadly healthy; measurement proved it false — of six candidates only two families could
be confirmed MIT or Apache-2.0. The FunASR family (SenseVoice / Paraformer) ships under Alibaba's own
_FunASR Model Open Source License Agreement v1.1_, which is **neither MIT nor Apache-2.0**, and most
streaming zipformer conversion packages carry no licence declaration upstream at all. Per-package
verdicts and evidence are in [`models/DENYLIST.md`](../../models/DENYLIST.md); the rejections include
`matcha-icefall-zh-baker` (non-commercial training data), the entire `vits-zh-hf-*` family (11 packages,
no licence declaration anywhere), `aishell3`, `edge-tts` (not a distributable weight at all but a call
to an undocumented endpoint), `MCGA`, `SenseVoice` / `sense-voice` / `paraformer`, four batches of
undeclared streaming packages, and `vosk-model` — which is **licence-clean but excluded on capability**
(a measured CER of 23.54 is too poor), listed precisely to stop it being reintroduced later on the
grounds that its licence is fine.

## Licence posture: a voice-enabled distribution is offered under GPL-3.0

**This is a verifiable fact, not a supposition.** The prebuilt `libsherpa-onnx-c-api.so` exports **50
`espeak_*` symbols** (`nm -D --defined-only ... | grep -c espeak` == 50, **case-sensitively**; using
`-i` overcounts by 14 because of `neSpeak` inside `OfflineSpeakerDiarization`). sherpa-onnx's
`SHERPA_ONNX_ENABLE_TTS` defaults to `ON`, which vendors in `csukuangfj/espeak-ng`, whose `COPYING` is
the **GNU GPL Version 3**.

The rest of the chain is clean: `sherpa-rs` / `sherpa-rs-sys` MIT (verified), sherpa-onnx Apache-2.0,
onnxruntime MIT. But **presence in the distributed binary triggers the obligation**; whether it is
actually called on the Chinese synthesis path is irrelevant.

**The posture actually implemented: the `voice` feature flag is the licence boundary.**

- The `voice` cargo feature is **off by default** (`default = []`). The default build is pure MIT and
  was measured to link no onnxruntime at all (`ldd target/release/yunjian | grep -i onnx` is empty).
- **A voice-enabled distribution must be offered in its entirety under GPL-3.0** (source availability,
  no further restrictions). MIT is one-way compatible with GPL-3.0, so this is not a licence conflict.
- Release artifacts therefore come in two kinds: the default build labelled MIT, the voice build
  labelled GPL-3.0.

**Two TTS packages additionally bundle GPL-3.0 espeak-ng pronunciation data**, declared item by item
under `[[model.bundled]]` in `models.toml`: `kokoro-multi-lang-v1_0` bundles 355 files, and
`kitten-nano-en-v0_2-fp16` bundles it too. `vits-melo-tts-zh_en` **does not** — its Chinese
pronunciation goes through the in-package `dict/` and `lexicon.txt`, making it the only Chinese voice
that touches no GPL-3.0 data.

**One unperformed verification, recorded honestly:** Chinese synthesis uses `lexicon-zh.txt` and does
not go through espeak, so in a Chinese-only deployment kokoro's espeak data could **in principle** be
omitted from the download; **this has not been confirmed by measurement**, so no distribution promise
is made on that basis today.

## The 破读 lexicon and its public-domain sourcing rule

破读 — a character read with its classical rather than modern pronunciation in verse — is carried by the
**project's own** [`data/poyin.tsv`](../../data/poyin.tsv), which **cites no third-party modern
pronunciation material**.

The header is `字 / context / pinyin / 依据 / confidence`, with **89 data rows** at present. Column
semantics:

- `context` — a phrase fragment; `*` means the reading is not context-restricted;
- `pinyin` — tone-marked pinyin; `-` means the row records only a disposition;
- `依据` — the evidence, which **must contain both a locator and the edition relied on**;
- `confidence` — one of `rhyme_attested`, `tone_split`, `engine_default`.

The three canonical cases (also the three most commonly cited 破读):

| Character | context | pinyin | confidence       |
| --------- | ------- | ------ | ---------------- |
| 斜        | 石径斜  | `xiá`  | `rhyme_attested` |
| 衰        | 鬓毛衰  | `cuī`  | `rhyme_attested` |
| 骑        | 一骑    | `jì`   | `tone_split`     |

**The public-domain sourcing rule is enforced by code, not by convention.** The function
`located_evidence` validates every row: non-empty, containing a locator (one of volume, section, page
or sample size), and containing 据/據 together with an edition term. A row failing this yields
`LexiconError::Unlocated` and **names the line number**. The redundant guard test is
`every_poyin_row_carries_located_evidence`, with four negative cases alongside it:
`empty_evidence_is_rejected`, `evidence_without_a_locator_is_rejected`,
`evidence_without_an_edition_is_rejected` and
`an_unlocated_row_is_rejected_with_its_line_number`.

**The evidence resolves to the tone-section level of a rhyme book** (《平水韵》, 《词林正韵》), the same
strictness the commentary pipeline applies to citations: an evidence field that exists but is empty is a
hard error, not a warning.

Two companion tables, each with an independent purpose:

- [`data/polyphone_index.tsv`](../../data/polyphone_index.tsv) (header `字 / 兼收`, **1815 data rows**)
  — a polyphone candidate set **independent of** the 破读 lexicon, derived from characters that a rhyme
  book files under more than one tone section or rhyme group. It exists to avoid circular reasoning:
  checking the lexicon's coverage against the lexicon itself would prove the premise from the
  conclusion.
- [`data/reading_roster.tsv`](../../data/reading_roster.tsv) (header
  `id / 选本 / 作者 / 题目 / 词牌 / 正文 / 依据`, **22 data rows**) — hard-codes the scope of the
  coverage-closure check. The assertion is "every polyphone in the roster has a row in `poyin.tsv`"
  (`assert_coverage`, test `coverage_over_the_roster_is_closed`), and **not a coverage percentage** — a
  percentage can be improved by enlarging the denominator; closure cannot.

**Coverage stated honestly:** the roster is 22 poems, not the whole corpus. Characters outside it fall
back to modern Mandarin defaults (`engine_default`). This half of the data — the 破读 lexicon — meets
its requirement.

## The ci rhythm table: 2 tunes, and the basis is not a metrical authority

**This is an open problem this document must record honestly; do not read it as "the ci patterns are in
place".**

[`data/citune_rhythm.tsv`](../../data/citune_rhythm.tsv) has the header `词牌 / 句式 / 来源 / 依据` and
currently holds **only 2 data rows**:

| Tune     | Source         | Evidence (as measured)                                                                                       |
| -------- | -------------- | ------------------------------------------------------------------------------------------------------------ |
| 念奴娇   | `corpus_modal` | Measured over 135 念奴娇 in the full Song ci corpus; the modal phrasing matches 58 of them, **43.0%**, n=135 |
| 水调歌头 | `corpus_modal` | Measured over 263 水调歌头; the modal phrasing matches 89 of them, **33.8%**, n=263                          |

**Three honest statements:**

1. **Coverage is 2 tunes**, whereas the plan required v1 to cover the tunes appearing in the standard
   Song ci anthology. Every tune outside the table falls back to punctuation.
2. **The basis was substituted.** The plan required public-domain metrical patterns with volume and
   page citations; what is actually used is `source = corpus_modal`, the measured modal phrasing of the
   full Song ci corpus. **This is statistical inference, not a metrical authority**, and a match rate
   below half (43.0% / 33.8%) shows that "modal phrasing" is itself of limited representativeness for
   same-named variant forms.
3. **The root cause is an open provenance question.** The most complete ci-pattern data,
   `Ci_Tunes.json`, was scraped from the commercial site `sou-yun.cn`; its chain of rights is
   unverified and it is withheld (see [Corpus and indexing](CORPUS.md)). This table is the cost of
   withholding it.

The `来源` column admits exactly two values, `citune` and `corpus_modal`, and **v1 has zero `citune`
rows**. Honesty tests pin this: `v1_claims_no_citune_authority`, `only_citune_claims_authority` and
`rhythm_source_identifiers_are_stable`. **Labelling the rows `corpus_modal` is honest, but it does not
satisfy the evidence type the plan specified** — that is left to be resolved by completing the tune
coverage and moving back to a metrical basis.

**The fallback mechanism is complete and asserted.** `RhythmSource` has four variants with stable
identifiers: `CharCount` (`char_count`), `CiTune` (`citune`), `CorpusModal` (`corpus_modal`) and
`Punctuation` (`punctuation`). `segment_ci` adopts the table's phrasing **only when
`spec.pattern.len() == non_empty.len()`**; a missing tune or a clause-count mismatch degrades to
`RhythmSource::Punctuation`.

**The clause-count guard is an addition worth explaining:** same-named variant forms are common in ci,
and forcing a pattern onto a mismatched poem puts the pauses in the wrong places — worse than not
segmenting at all. Two tests cover it:
`a_tune_absent_from_the_table_falls_back_to_punctuation` and
`a_clause_count_mismatch_degrades_instead_of_misplacing_pauses`.

## Crate structure and feature boundaries

The four features of `yunjian-voice`, with `voice` acting as the licence boundary (above):

```toml
default  = []
capture  = ["dep:rodio"]
download = ["dep:ureq", "dep:tar", "dep:bzip2"]
voice    = ["dep:sherpa-rs", "capture", "download"]
```

Modules and their duties (taken from each `//!`):

| Module       | Feature gate                                | Duty                                                                                                                                                    |
| ------------ | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `error`      | none (private)                              | The failure surface of the voice path (`VoiceError`)                                                                                                    |
| `permission` | none                                        | Microphone permission, and the degradation verdict when it is unavailable                                                                               |
| `platform`   | none                                        | The five per-platform floors and where each one comes from                                                                                              |
| `capture`    | `capture`                                   | Microphone capture, producing the 16 kHz mono PCM the recognizer requires                                                                               |
| `augment`    | none                                        | Audio augmentation approximating channel and speaker variation via narrowband round-trip, pink noise and time stretching (used for the CER measurement) |
| `audio`      | none                                        | The capture/playback verdict layer, and how each failure degrades to typed practice                                                                     |
| `models`     | none (`models::transport` needs `download`) | On-demand model download, licence gating and local cache                                                                                                |
| `lexicon`    | none                                        | The 破读 lexicon, the ci rhythm table and the reading roster                                                                                            |
| `prosody`    | none                                        | Reading rhythm: foot segmentation, silence splicing, per-foot timestamps                                                                                |
| `asr`        | `voice`                                     | Offline recognition (Whisper family)                                                                                                                    |
| `tts`        | `voice`                                     | Offline synthesis, including 破读 lexicon injection and wiring into `prosody::FootSynthesizer`                                                          |

Two names are easy to misremember and are stated here: **the registry type is `models::Registry`;
there is no module named `model_registry`, and the crate has no `session` module.**

Three measured native-dependency traps (the full account is in [Voice build](../VOICE-BUILD.zh.md)):

- **`sherpa_rs::read_audio_file` hard-asserts 16 kHz** and cannot read the 24 kHz audio its own TTS
  produces. The most natural verification chain — synthesize, write a WAV, read it back — therefore
  does not work; this crate's tests use `hound` instead.
- **`sherpa-rs`'s `static` feature is unusable on Linux**, and the second obstacle is an upstream
  packaging defect: the static release archive contains **no `lib/` directory** and zero `.a` files.
  Dynamic linking plus an `$ORIGIN` rpath is the path that works.
- **Dynamic linking without an rpath means CI is green and the release artifact exits 127.**
  `cargo test` injects `LD_LIBRARY_PATH` automatically, so tests pass while the release binary fails to
  start. **"Tests pass" is not "the artifact runs"**, so CI has a separate step asserting that
  `./target/release/yunjian` starts on its own.

## The degradation chain: every failure point returns to typed practice

**The destination of degradation is always typed practice — never a zero score, never a panic, never a
hang.** The unifying type is `Practice`, with two variants: `Voice` and
`Typed { reason: DegradeReason, message: String }`, where the latter **must** state the cause and the
way to recover.

`DegradeReason` has nine variants: `FeatureDisabled`, `SystemTooOld`, `PermissionDenied`,
`PermissionRestricted`, `PermissionUndetermined`, `NoInputDevice`, `ModelUnavailable`, `DeviceBusy`,
`CaptureFailed`. `degrade(reason, platform)` unconditionally constructs `Practice::Typed`, and
`explain` produces a message per cause that includes how to recover.

Failure point by failure point:

1. **`voice` not compiled in** — the top-level `practice` checks `is_available()` first and on false
   calls `degrade(DegradeReason::FeatureDisabled, ...)`.
2. **The system is below the floor** — `Preflight::check` returns
   `AudioError::UnsupportedPlatformVersion` first.
3. **Permission unavailable** — `PermissionState::{Denied, Restricted, Undetermined}` map to their
   respective `DegradeReason`s; only `Granted` returns `Practice::Voice`.
4. **Device and audio failures** — the five `AudioError` variants (`PermissionDenied`, `NoDevice`,
   `UnsupportedPlatformVersion`, `DeviceBusy`, `Failed`) land through an **exhaustive**
   `degrade_reason` mapping.
5. **Capture stalls or truncates** — a `recv_timeout` timeout yields `VoiceError::CaptureStalled`, a
   closed channel yields `VoiceError::AudioDevice`, and completion below `MIN_COMPLETION` returns
   `VoiceError::CaptureTruncated` so **half a recording never reaches scoring**.
6. **Model failures** — all ten `ModelError` variants (including `Denied`, `LicenseRefused`,
   `ChecksumMismatch`, `SizeMismatch`) map exhaustively to `DegradeReason::ModelUnavailable`.

Four standing tests keep this chain from being a paper promise, and each **requires a distinct message
per failure** (otherwise "it degraded" cannot be told apart from "it named the right cause"):
`every_audio_error_degrades_to_typed_practice_with_its_own_explanation`,
`every_failure_degrades_to_typed_practice_with_a_specific_message`,
`denied_permission_degrades_to_typed_practice_with_an_explanation` and
`restricted_and_undetermined_also_degrade_but_with_distinct_reasons`.

## What does not exist yet (recorded honestly)

- **Human-speaker CER is unmeasured.** Only the synthetic-plus-augmentation optimistic bound (77.01%)
  exists, so character accuracy on the voice path is **permanently advisory** and will never become a
  score. The gap is a licensing one: the only public Chinese recitation corpus carries an NC clause.
- **Streaming recognition is not wired.** The licence-clear streaming package is in the manifest, but
  `sherpa-rs` 0.6.8 has no streaming wrapper; reaching it requires calling the online API through
  `sherpa_rs_sys`. Its CER is NOT MEASURED.
- **`coverage_advisory` is not open.** It requires an independent KWS spike to clear pre-frozen
  thresholds first (sentence-level recall on complete readings ≥95%, missing-line detection ≥95%,
  silence and noise must not pass as complete, false-complete on an unrelated poem of the same metre
  ≤1%, coverage MAE ≤0.10, coverage must be monotonic across nested samples, repetition must not
  increase the score, reordering must be observable). That spike has not been run. Two known
  limitations will be reflected in the UI: **KWS results carry no confidence**, so the UI may show only
  "detected / unconfirmed" and **must not fabricate "87% confident"**; and a homophone substitution is
  acoustically indistinguishable, so a detection proves only that the expected pronunciation sequence
  was heard, not that the user said the expected characters.
- **Ci pattern coverage is 2 tunes, on a measured-modal rather than public-domain-metrical basis**
  (previous section).
- **iOS supports only `cargo check`.** `sherpa-rs` declares `crate-type = ["cdylib", "rlib"]`, and cargo
  builds every declared lib type for a dependency, while that `cdylib` fails to link on iOS
  (`Undefined symbols for architecture arm64`) — iOS has only static archives and a standalone dylib
  should never have been built. **This is an upstream packaging limitation, not a configuration problem
  here**: `bindgen` and native-artifact acquisition both succeed and `cargo check` passes, so the
  compilation path holds. Producing a usable iOS library requires forking `sherpa-rs` to drop the
  `cdylib` (or getting it fixed upstream).
- **Code behind `--features voice` is in no clippy gate.** `voice-build.yml` only builds and smoke-tests
  without running clippy, and `make lint` does not reach code behind a feature gate. This is a real
  coverage gap, recorded here so it is not mistaken for covered.
- **Four platform verifications have run neither locally nor in CI**, each needing specific conditions:
  the runtime behaviour of a notarized macOS build (needs a paid Apple Developer account and notarization
  credentials), the macOS TCC authorization dialog (needs a signed artifact and a real machine), the
  Android runtime permission dialog (needs a device or emulator) and iOS on-device capture and
  authorization (needs macOS, Xcode and a device with a configured signing identity). One further item
  is **partially verified**: real capture on Windows — the runner enumerated zero input devices, so what
  is verified is the degradation path (reports `NoInputDevice` with an explanation, no panic and no
  hang), while real capture needs a Windows host with a sound card.

## Related documents

- [Voice build](../VOICE-BUILD.zh.md) — native dependency builds across five platforms, linking, licence impact
- [Platform requirements](PLATFORM-REQUIREMENTS.md) — per-platform floors, the microphone permission chain, behaviour below the floor
- [CER report](../reports/asr-cer.md) — the measured character accuracy of classical-Chinese recognition, and why it can only be advisory
- [Third-party licences](../../LICENSES.md) — per-asset licence and attribution
