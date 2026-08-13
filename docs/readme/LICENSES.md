[简体中文](../../LICENSES.md) · English

# Third-party licences and attribution

This document enumerates every third-party asset Yunjian bundles or downloads on demand, its licence,
and how the attribution requirement is satisfied. **Every licence and revision below is taken from a
real manifest file in this repository** ([`models.toml`](../../models.toml),
[`corpus/sources.toml`](../../corpus/sources.toml), the root [`Cargo.toml`](../../Cargo.toml)) rather
than written from memory.

Yunjian's own code is licensed under [MIT](../../LICENSE) (`Copyright (c) 2026 sunerpy`).

## Contents

- [Two distribution artifacts, two licences](#two-distribution-artifacts-two-licences)
- [Voice model weights (downloaded on demand)](#voice-model-weights-downloaded-on-demand)
- [Third-party material bundled inside model packages](#third-party-material-bundled-inside-model-packages)
- [Voice native dependencies (the `voice` feature)](#voice-native-dependencies-the-voice-feature)
- [Corpus data sources (shipped)](#corpus-data-sources-shipped)
- [Rust dependencies](#rust-dependencies)
- [Frontend dependencies](#frontend-dependencies)
- [What is neither shipped nor downloaded](#what-is-neither-shipped-nor-downloaded)
- [How the attribution obligations are satisfied](#how-the-attribution-obligations-are-satisfied)

## Two distribution artifacts, two licences

**This section is the most important statement in the document, not a footnote.**

| Artifact                                                            | cargo feature                    | Overall licence | Basis                                                                                         |
| ------------------------------------------------------------------- | -------------------------------- | --------------- | --------------------------------------------------------------------------------------------- |
| Default build (dictionary + typed recitation + MCP + desktop shell) | `voice` **off** (`default = []`) | **MIT**         | Measured to link no onnxruntime at all: `ldd target/release/yunjian \| grep -i onnx` is empty |
| Voice build (including offline read-aloud and spoken practice)      | `voice` **on**                   | **GPL-3.0**     | The prebuilt sherpa-onnx statically contains GPL-3.0 espeak-ng (below)                        |

**The basis is a verifiable fact, not a supposition.** The prebuilt `libsherpa-onnx-c-api.so` exports
**50 `espeak_*` symbols** (`nm -D --defined-only libsherpa-onnx-c-api.so | grep -c espeak` == 50,
**case-sensitively** — using `-i` overcounts by 14 because of `neSpeak` inside
`OfflineSpeakerDiarization`). sherpa-onnx's `SHERPA_ONNX_ENABLE_TTS` defaults to `ON` and vendors
`csukuangfj/espeak-ng` in through `cmake/espeak-ng-for-piper.cmake`, and that fork's `COPYING` is the
**GNU GPL Version 3**.

The rest of the chain is clean (`sherpa-rs` / `sherpa-rs-sys` MIT, sherpa-onnx Apache-2.0, onnxruntime
MIT), but **presence in the distributed binary triggers the obligation** — whether it is actually called
on the Chinese synthesis path is irrelevant.

MIT is one-way compatible with GPL-3.0, **so this is not a licence conflict**; but a voice-enabled
Yunjian is a combined work and must be offered in its entirety under GPL-3.0 (source availability, no
further restrictions). The project is fully open source on GitHub, so the compliance cost is close to
zero.

## Voice model weights (downloaded on demand)

**No weights are in the installer.** Weights are downloaded on demand and verified byte for byte
against SHA-256; the repository holds only identity and licence records, enforced by
`cargo run -p xtask -- verify-models`.

**MIT and Apache-2.0 only, with no exceptions.** The `license` field itself is not trusted — the gate
opens the vendored evidence file and checks the SPDX marker, and a field that disagrees with its
evidence fails.

| Package                                                      | Kind | Role       | Licence    | Evidence form      | Underlying weights (attribution target)              | Archive bytes |
| ------------------------------------------------------------ | ---- | ---------- | ---------- | ------------------ | ---------------------------------------------------- | ------------- |
| `sherpa-onnx-whisper-tiny`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper tiny                                  | 116,204,861   |
| `sherpa-onnx-whisper-base`                                   | asr  | production | MIT        | `upstream_license` | OpenAI Whisper base                                  | 207,557,382   |
| `sherpa-onnx-whisper-small`                                  | asr  | production | MIT        | `upstream_license` | OpenAI Whisper small                                 | 639,387,718   |
| `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` | asr  | production | Apache-2.0 | `model_card`       | `pfluo/k2fsa-zipformer-chinese-english-mixed`        | 511,274,346   |
| `vits-melo-tts-zh_en`                                        | tts  | production | MIT        | `package_license`  | MyShell.ai MeloTTS (`Copyright (c) 2024 MyShell.ai`) | 167,006,755   |
| `kokoro-multi-lang-v1_0`                                     | tts  | production | Apache-2.0 | `package_license`  | `hexgrad/Kokoro-82M`                                 | 349,418,188   |
| `kitten-nano-en-v0_2-fp16`                                   | tts  | **smoke**  | Apache-2.0 | `package_license`  | `KittenML/kitten-tts-nano-0.2`                       | 26,586,708    |

**The pinned licence evidence and its digests** (`license_rev` is a 40-hex commit SHA — a branch name
moves and is therefore no pin at all):

| Package                                       | Vendored evidence copy                                                               | Copy SHA-256                                                       | `license_rev`                              |
| --------------------------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------ | ------------------------------------------ |
| Whisper tiny / base / small (one shared copy) | `models/licenses/openai-whisper.LICENSE`                                             | `b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d` | `5f86d1d86363843179951550570367b37c5d6f78` |
| streaming-zipformer-bilingual-zh-en           | `models/licenses/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.CARD.md` | `b6f9458f4208ae821beaaf11dc983486916a4089a2f3677b40f9ff06ec4e6440` | `98590b7ed6443e77b714204da2757d75e1a642f4` |
| (its upstream model card)                     | `models/licenses/k2fsa-zipformer-chinese-english-mixed.CARD.md`                      | —                                                                  | `6eb615ae77ecac05c5628d5c8ed7037c14a338d5` |
| vits-melo-tts-zh_en                           | `models/licenses/vits-melo-tts-zh_en.LICENSE`                                        | `88a50e5a02bbc2a5c2f084dc19da751aa97b1690f5fda76cd8005c8634d1ca70` | `a0d5c6a264c0ef92d70d8661d8cc502d79627cd6` |
| kokoro-multi-lang-v1_0                        | `models/licenses/kokoro-multi-lang-v1_0.LICENSE`                                     | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` | `7e9b67b79bfdcbd2b4bc144370345fcceac3cb0c` |
| kitten-nano-en-v0_2-fp16                      | `models/licenses/kitten-nano-en-v0_2-fp16.LICENSE`                                   | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` | `7d14a97d38072576ddca7a673ab5bb49c43bb169` |

**Three qualifications that must be stated:**

- **The Whisper conversion packages carry no LICENSE of their own** and declare no SPDX in their model
  cards. The evidence form is therefore `upstream_license`: OpenAI's `openai/whisper` LICENSE states
  that the weights are released under MIT, the ONNX export is a format conversion of those weights, and
  the MIT terms carry over. **That chain is written into the manifest's note rather than left for the
  reader to infer.**
- **`kitten-nano-en-v0_2-fp16` is an English smoke model**, used only to prove the build and link path
  works; it **never enters the Chinese product path and is never shipped to users**. It remains in the
  manifest only because the rule "every weight the product touches has a licence record" admits no
  exception.
- **The in-package LICENSE of `vits-melo-tts-zh_en` and `kokoro-multi-lang-v1_0` is byte-identical to
  the pinned HuggingFace revision** (verified by `cmp`), so their evidence form is the strongest one,
  `package_license`.

## Third-party material bundled inside model packages

Third-party material bundled inside a release package is declared **item by item**
(`[[model.bundled]]` in `models.toml`), and where a GPL-class entry exists,
`distribution_impact` is mandatory — **the distribution consequence must not live only in somebody's
memory**.

| Host package               | Bundled path     | Material                                                                  | Licence     | Distribution impact                                                                                                                                                                                                                                                                |
| -------------------------- | ---------------- | ------------------------------------------------------------------------- | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kokoro-multi-lang-v1_0`   | `espeak-ng-data` | Pronunciation dictionary data from `csukuangfj/espeak-ng` (**355 files**) | **GPL-3.0** | It is data rather than linked code, but shipping it is still distributing GPL-3.0 material. This is the same licence posture as "the prebuilt sherpa-onnx statically contains GPL-3.0 espeak-ng", with the same conclusion: **the voice build is distributed under GPL-3.0 terms** |
| `kitten-nano-en-v0_2-fp16` | `espeak-ng-data` | As above                                                                  | **GPL-3.0** | It is a smoke model and is never shipped to users, so it does not affect the licence conclusion for release artifacts                                                                                                                                                              |

**`vits-melo-tts-zh_en` bundles no `espeak-ng-data`** — its Chinese pronunciation goes through the
in-package `dict/` (jieba) and `lexicon.txt`, making it the only Chinese voice that touches no GPL-3.0
data.

**One unperformed verification, recorded honestly:** Chinese synthesis uses `lexicon-zh.txt` and does
not go through espeak, so in a Chinese-only deployment kokoro's espeak data could **in principle** be
omitted from the download; **this has not been confirmed by measurement**, so no distribution promise
is made on that basis today, and the table above records the conclusion as "bundled means triggered".

## Voice native dependencies (the `voice` feature)

| Component                                        | Licence     | Notes                                                                                                                                                                                                      |
| ------------------------------------------------ | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sherpa-rs` (`=0.6.8`)                           | MIT         | Third-party Rust bindings, verified                                                                                                                                                                        |
| `sherpa-rs-sys`                                  | MIT         | Downloads the official prebuilt artifact per `dist.json`; `dist.json` hard-codes the sherpa-onnx version inside the crate version, so loosening the version constraint loosens the native artifact version |
| The k2-fsa/sherpa-onnx prebuilt artifact         | Apache-2.0  | **but statically contains the next row**                                                                                                                                                                   |
| `csukuangfj/espeak-ng` (compiled into the above) | **GPL-3.0** | 50 exported `espeak_*` symbols, see above                                                                                                                                                                  |
| onnxruntime (inside the sherpa-onnx artifact)    | MIT         | —                                                                                                                                                                                                          |

## Corpus data sources (shipped)

**The shipped corpus contains only public-domain source text plus MIT-licensed upstream compilation
output.** Per-asset verdicts are in [`corpus/sources.toml`](../../corpus/sources.toml), enforced by
`xtask verify-sources`.

| Source                                                                                | Pinned revision                            | Licence | Vendored LICENSE copy                                    | Copy SHA-256                                                       | What it provides                                                                      |
| ------------------------------------------------------------------------------------- | ------------------------------------------ | ------- | -------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| [`chinese-poetry/chinese-poetry`](https://github.com/chinese-poetry/chinese-poetry)   | `b8594f81a89752241442f2ce267d6f66f96704ee` | MIT     | `corpus/licenses/chinese-poetry.LICENSE`                 | `c195319aeaa3ffcbe16aa5d26eec19eae5a42f84337dd2b3dc3c9d5ccbbd6507` | Tang poetry, Song ci, Yuan qu, Chuci, Shijing, Five Dynasties poetry, metre (strains) |
| [`Werneror/Poetry`](https://github.com/Werneror/Poetry)                               | `4cfe49c06858e00d15f84d192fe5294295f79689` | MIT     | `corpus/licenses/Werneror-Poetry.LICENSE`                | `3c2630eb84efab60868d5195aa656b954f77d3cc1127dc886601e21cfd9fb63b` | Poetry across dynasties (13 bucketed CSVs)                                            |
| [`charlesix59/chinese_word_rhyme`](https://github.com/charlesix59/chinese_word_rhyme) | `ff0e9c13fb037c43e0eaa5dc929c0fe4fa2ffb18` | MIT     | `corpus/licenses/charlesix59-chinese_word_rhyme.LICENSE` | `e1464036d0f0ca738de9ebcb697b8faaf6dc2eafd193dc98555f23b409e87599` | 平水韵, 词林正韵, per-character tone                                                  |

**Verification is per asset rather than per repository** — a repository-level MIT LICENSE grants rights
over that repository's own compilation work and cannot cover content it scraped or transcribed. The
manifest holds **68 asset verdicts**: 42 `public_domain`, 5 `permissive`, 21 `unverified`. Assets marked
`shippable = false` never enter a distributed artifact, and **`unverified` together with
`shippable = true` is a hard failure**.

**Data files authored by this project** (under `data/`) are not third-party assets and are licensed with
the code under MIT, but their **evidence** derives from public-domain rhyme books and public-domain
corpora, with each row citing a locator and the edition relied on:

| File                       | Rows  | Evidence type                                                                                                                            |
| -------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `data/poyin.tsv`           | 89    | 《平水韵》 and 《词林正韵》 to the tone-section level, each row carrying a locator and an edition                                        |
| `data/polyphone_index.tsv` | 1815  | Characters a rhyme book files under more than one tone section or rhyme group                                                            |
| `data/reading_roster.tsv`  | 22    | Anthology plus per-poem evidence                                                                                                         |
| `data/citune_rhythm.tsv`   | **2** | **`corpus_modal`: measured modal phrasing from the full Song ci corpus, not a public-domain metrical authority** (see [Voice](VOICE.md)) |

The historical-commentary seed (`corpus/commentary/sources/`) holds 487 entries covering 398 poems from
10 **pre-modern** works of poetry criticism, each citing a fixed revision of a Wikisource transcription
— pre-modern criticism is out of copyright and is a different legal category from modern appreciation.

## Rust dependencies

All third-party crates in the workspace come from crates.io, with each crate's own declaration
governing its licence (`cargo metadata` lists them, and `cargo-audit` runs as a separate job in CI).
**A few choices bearing directly on licensing or compliance are recorded here, because they are
deliberate decisions rather than defaults:**

| Crate                                        | Relevant note                                                                                                                                                                                       |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rusqlite` (`bundled`)                       | `bundled` cannot be removed: the system SQLite may predate the FTS5 trigram tokenizer (3.34.0)                                                                                                      |
| `keyring-core` plus five per-platform stores | **The `keyring` facade crate is deliberately avoided** — its own documentation says applications should not link it, and it would pull all five platforms' stores into the dependency graph at once |
| `genai` (`0.6`, `rustls-tls`)                | `rig-core` (an agent framework) and `llm-chain` (unmaintained since 2023) are deliberately avoided; hard-coding rustls avoids the OpenSSL C dependency                                              |
| `ferrous-opencc`                             | Appears only in `yunjian-corpus`, so **no conversion dictionary ships at runtime**; `opencc-rust` and `zhconv` with default features are deliberately avoided                                       |
| `pinyin` (`default-features = false`)        | **A correctness constraint, not size trimming**: the tone marks produced by `with_tone` are multi-byte non-ASCII and would silently defeat the downstream byte-wise near-homophone criterion        |
| `inputx-phonetic-edit` (`1`)                 | Resolves to `1.4.0`, whose declared `rust-version = 1.95` is **higher** than the workspace's declared `1.88` — see "known inconsistency" below                                                      |
| `tauri` / `tauri-build`                      | **`tauri-plugin-log` is deliberately avoided**: logging goes through `yunjian_core::init_logger` so all three entry points share one level parser, one redactor and one rolling-file layout         |
| `sha2`                                       | The manifest field is named `license_sha256` and must genuinely be SHA-256 — switching to blake3 would make it impossible to cross-check against `sha256sum` and GitHub's own digests               |

**A known inconsistency, recorded honestly:** the root `Cargo.toml` declares
`rust-version = "1.88"`, inherited by every member, and the README states Rust 1.88+; but
`inputx-phonetic-edit 1.4.0` declares `rust-version = 1.95`. Cargo's behaviour here is a **fallback
rather than a satisfaction** (it prints `Locking ... to latest Rust 1.88 compatible versions`
immediately followed by `Adding inputx-phonetic-edit v1.4.0 (requires Rust 1.95)`). CI (stable) and the
development machine both build, and **no gate reports this**, but a checkout on 1.88–1.94 cannot compile
`yunjian-recite`. This is an unresolved external-promise question rather than a licensing one, recorded
here so it is not mistaken for settled.

## Frontend dependencies

The desktop shell's frontend lives in `app/`, with dependencies declared in `app/package.json` and each
npm package's own declaration governing its licence. The React and Vite ecosystem packages involved are
MIT or Apache-2.0. **The frontend introduces no model weights and no corpus data.**

One measured lesson: **npm package versions are also identifiers that must never be written from
memory** — the first `package.json` written from memory used `@types/react-dom@18.3.9`, which does not
exist (the real version is 18.3.7), and `npm install` failed with `ETARGET`.

## What is neither shipped nor downloaded

Rejected assets have **no read path**, not merely a note on a list saying they are unused:

- **The 17 rejected data sources** are recorded with a reason each in
  [`corpus/DENYLIST.md`](../../corpus/DENYLIST.md); `verify-sources` substring-matches a source's
  `name` / `url` and fails the build on a hit, and additionally asserts that all 14 identifiers in
  `REQUIRED_DENYLIST` are present, so **removing an entry fails the build**. The full list with reasons
  is in [Corpus and indexing](CORPUS.md).
- **Rejected voice models** are recorded in [`models/DENYLIST.md`](../../models/DENYLIST.md), including
  `matcha-icefall-zh-baker` (non-commercial training data), the entire `vits-zh-hf-*` family (11
  packages with no licence declaration), `aishell3`, `edge-tts` (not a distributable weight at all but a
  call to an undocumented endpoint), `MCGA` (CC BY-NC-SA-4.0), `SenseVoice` / `sense-voice` /
  `paraformer` (the FunASR agreement, which is neither MIT nor Apache-2.0), four batches of undeclared
  streaming packages, and `vosk-model` (**licence-clean but excluded on capability**).
- **Withheld assets have no read path in the code**: `yunjian_corpus::rhyme` accepts only paths in the
  `SHIPPED_ASSETS` allowlist, and passing a withheld asset yields an error rather than data. 中华新韵 (a
  2005 modern publication) and the ci patterns scraped from `sou-yun.cn` are both in this category.

## How the attribution obligations are satisfied

**The attribution copies and the evidence copies are two things with different purposes and
byte-identical content; neither may be hand-edited nor rewritten by a formatter** (see `.gitattributes`
and `.oxfmtignore`):

- **`models/licenses/` is evidence**: verified byte for byte by `verify-models` against
  `license_sha256`, proving that the SPDX recorded in the manifest matches the actual content at the
  pinned upstream revision.
- **[`licenses/`](../../licenses/) is attribution**: intended for the distribution, holding the licence
  text of every registered model. File names are the release package names, and the extension follows
  `license_evidence` (`.LICENSE` for a full licence text, `.CARD.md` for a model card).

Their agreement is pinned by `licenses_directory_holds_a_file_for_every_manifest_entry` in
`cargo test -p yunjian-voice models` — a mismatch would mean the licence text handed out with the
distribution and the one that was verified are two different things.

**The corpus side works the same way**: `corpus/licenses/` holds LICENSE copies for the three upstream
sources, and `verify-sources` cases such as `vendored_licenses_match_recorded_hashes` verify "vendored
bytes == recorded digest".

One measured trap is worth recording: **a Windows runner's `core.autocrlf=true` makes any byte-wise
SHA-256 gate report a false failure** (LF becomes CRLF on checkout, so the digest necessarily
disagrees). The fix is `corpus/licenses/** -text` in the repository root `.gitattributes` — `-text` is
stronger than `eol=lf` in that git performs no line-ending conversion at all, so the object-store bytes
equal the worktree bytes. **Removing that line turns several gates red at once.**

## Related documents

- [Corpus and indexing](CORPUS.md) — sources, per-entry exclusion reasons, identity model, commentary admission
- [Voice](VOICE.md) — models and licences, the 破读 lexicon, no pronunciation-standard assessment
- [Voice build](../VOICE-BUILD.zh.md) — native dependency builds across five platforms and the GPL-3.0 impact
- [AI appreciation](AI.md) — why the shipped dataset uses open-weight models only
