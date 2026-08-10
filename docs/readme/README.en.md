[简体中文](../../README.md) · English

# 云笺 yunjian

An offline Chinese classical poetry toolkit: a local SQLite corpus, search, AI appreciation,
recitation practice, and an MCP server that lets an AI assistant query your poetry library directly.

[![CI](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml/badge.svg)](https://github.com/sunerpy/yunjian/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

> [!IMPORTANT]
> **This project is in early development and has no usable release yet.** The feature overview below
> describes the intended shape; for what actually exists today see [Project status](#project-status).
> Do not expect `cargo install` to give you a working poetry program.

## Contents

- [Project status](#project-status)
- [Why it is built this way](#why-it-is-built-this-way)
- [Quick start](#quick-start)
- [Feature overview](#feature-overview)
- [Content provenance and licensing](#content-provenance-and-licensing)
- [Documentation](#documentation)
- [Development](#development)

## Project status

Implemented on `main` **and covered by tests**:

| Component           | Status   | Notes                                                                                               |
| ------------------- | -------- | --------------------------------------------------------------------------------------------------- |
| Workspace skeleton  | done     | Cargo workspace of 8 crates, dependency versions pinned centrally                                   |
| Config and logging  | done     | Runtime `config.toml` discovery; tracing to stderr and a rolling file only                          |
| stdout ban          | done     | Clippy denies `println!` and `std::io::stdout` workspace-wide, one CLI exit                         |
| Licence gate        | done     | `xtask verify-sources` checks upstream licences and SHA-256 per asset                               |
| Corpus record model | done     | Canonical records plus an append-only `stable_id` registry                                          |
| Index selection     | measured | FTS5 `detail` mode and n-gram table settled by measurement, see [Corpus](CORPUS.md)                 |
| Voice build path    | verified | Native dependency build and linking proven on five targets, see [Voice build](../VOICE-BUILD.zh.md) |

**Not implemented yet** (fully specified, but not a line of product code): poetry search, AI
appreciation, recitation practice and FSRS review, offline read-aloud and speech recognition, the
MCP server, the command-line tool, the Tauri desktop app, the mobile app.

## Why it is built this way

In one line: **the copyright wall is the architecture.**

Across the open poetry datasets that exist, every one carrying modern annotation, translation or
appreciation has a licence chain that does not hold up: scraped from commercial sites, marked
academic-use-only, or covered by a repository-level LICENSE that cannot possibly grant rights over
transcribed content. Verifying this in practice flagged 10 files carrying modern vernacular
annotation _inside_ a single MIT repository.

So 云笺 combines exactly three things:

1. **Public-domain source text** — pre-modern works, out of copyright;
2. **Pre-modern commentary with a per-entry citation** — a Song critic writing about a Tang poem is
   itself public domain;
3. **Clearly labelled AI appreciation** — generated with your own API key, always tagged as AI text.

The AI feature is not a nice-to-have. It is the only lawful way to fill the hole the copyright wall
leaves behind.

## Quick start

> Only a source build is possible today, and the resulting binary has no usable subcommands yet.
> This section is currently a **developer** quick start.

Requires Rust 1.88+ (`rust-toolchain.toml` is checked in, so `rustup` installs the right version):

```bash
git clone https://github.com/sunerpy/yunjian.git
cd yunjian
cargo build --workspace
```

Run the same gate CI runs (format check + clippy + tests):

```bash
make ci
```

Verify upstream corpus licences and digests (offline, no network):

```bash
cargo run -p xtask -- verify-sources --offline
```

A per-asset licence verdict for every source plus exit code 0 means the environment is correct.

## Feature overview

This is the **intended** shape. Actual progress is always [Project status](#project-status).

- **Offline search.** The corpus is a read-only SQLite file: no network, no login, no account. An
  FTS5 trigram index is paired with a 1/2-character n-gram candidate table, because two-character
  queries like 明月 dominate real usage and trigram can derive no constraint below three characters.
- **AI appreciation, bring your own key.** The key lives in the OS keychain, never in an environment
  variable and never in a config file. It works fully without a key: a pre-generated set of
  appreciations ships for the well-known poems.
- **Recitation practice.** Three typed modes (blanked, first-character hint, masked) share one
  scoring kernel; spoken practice adds completeness and fluency. Character accuracy on the voice
  path is **always an estimate, never a score** — speech recognition on classical Chinese is not
  reliable enough to grade you.
- **Read-aloud.** Per-foot synthesis with silence spliced in on the Rust side, using the project's
  own public-domain-sourced classical reading table; outside its coverage it falls back to modern
  Mandarin.
- **MCP server.** `yunjian mcp` speaks stdio so clients like Claude Desktop and OpenCode can query
  your library directly. Generated output is labelled as AI-written and is never written back into
  the corpus.
- **Multiple surfaces.** Tauri v2 + React on the desktop; a CLI with machine-readable output; the
  mobile framework is chosen by real-device measurement.

## Content provenance and licensing

**The code** is [MIT](../../LICENSE) licensed.

**The shipped corpus contains only public-domain source text plus MIT-licensed upstream compilation
output.** Per-asset licence verdicts live in [`corpus/sources.toml`](../../corpus/sources.toml) and
are enforced by `xtask verify-sources`; rejected datasets and the reason for each rejection are
recorded in [`corpus/DENYLIST.md`](../../corpus/DENYLIST.md). Verification is per **file**, not per
repository — a repository's MIT LICENSE cannot license content it scraped.

**Appreciation text is AI-generated, not scholarship.** The UI renders it in a different visual
register from cited commentary and discloses that it is unreviewed. AI-generated poems are labelled
as such and never enter the corpus or the appreciation tables.

**Note on the voice feature's licensing:**

- The `voice` cargo feature is **off by default**. A default build is pure MIT and, as measured,
  links no onnxruntime at all.
- With `voice` enabled, the prebuilt sherpa-onnx artifact **statically contains GPL-3.0 espeak-ng**
  (50 exported `espeak_*` symbols, measured). MIT is one-way compatible with GPL-3.0, so this is not
  a conflict — but **a distributed voice-enabled build must be offered under GPL-3.0 as a whole.**
- Releases therefore come in two flavours: default builds labelled MIT, voice builds labelled
  GPL-3.0. Details in [Voice build](../VOICE-BUILD.zh.md).

No model weights are bundled. Voice models are downloaded on demand and only under a verified MIT or
Apache-2.0 licence.

## Documentation

- [Architecture](ARCHITECTURE.md) — layering, why `yunjian-core` never learns Tauri exists, the
  mobile escape hatch
- [Corpus and indexing](CORPUS.md) — build pipeline, the `stable_id` identity model, the measured
  FTS5 index selection
- [Voice build](../VOICE-BUILD.zh.md) — native dependency builds across five targets, linking, and
  the licensing consequences

## Development

```bash
make hooks   # install pre-commit (format on commit) and pre-push (run make ci) hooks
make help    # list every target
```

Conventions:

- **Commit subjects are Chinese imperatives in Conventional Commits form**, e.g.
  `feat(core): 添加韵部检索`.
- **No `println!`.** Logs go through `tracing` to stderr and a rolling file; the same binary hosts an
  MCP stdio server, and one stray line on stdout corrupts the protocol stream.
- **No secrets in `config.toml`.** The top-level config sets `deny_unknown_fields`, so pasting an
  `api_key` in errors out instead of being silently discarded.
- **Never ingest third-party modern annotation, translation or appreciation** — see above and
  `corpus/DENYLIST.md`.

Issues and pull requests are welcome. Read [Corpus and indexing](CORPUS.md) before touching a corpus
source.
