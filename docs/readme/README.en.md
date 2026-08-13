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
- [Using with an LLM](#using-with-an-llm)
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
| Rhyme book import   | done     | 平水韵 and 词林正韵 ingested, reverse index derived at build time; 中华新韵 and 词谱 withheld       |
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

Three steps: install, fetch the corpus, search a line.

```bash
# 1. Install (Linux / macOS)
curl -fsSL https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.sh | sh

# 2. Fetch the corpus (~211 MiB; search structures are derived locally on first launch)
yunjian corpus fetch

# 3. Search
yunjian search 明月
```

On Windows, use PowerShell:

```powershell
irm https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.ps1 | iex
```

Both scripts detect the OS and CPU architecture, resolve the matching release asset, and
**verify its SHA-256 before anything lands on disk** — a checksum failure installs nothing.

| Variable              | Default            | Effect                                          |
| --------------------- | ------------------ | ----------------------------------------------- |
| `YUNJIAN_VERSION`     | latest release     | Install a specific version; `v0.1.0` or `0.1.0` |
| `YUNJIAN_INSTALL_DIR` | `$HOME/.local/bin` | Install directory                               |

> [!NOTE]
> The first tagged release (`v0.1.0`) has not been cut yet, so the commands above are waiting on
> it. Until then, build from source: `cargo build --workspace --release -p yunjian-cli` puts the
> binary at `target/release/yunjian`. See [Development](#development).

## Using with an LLM

Results go to stdout, logs go to stderr, and success is decided by the exit code alone. The block
below can be handed to an AI assistant as-is.

<details>
<summary>Commands, output contract, and MCP client configuration</summary>

### Key commands

```bash
yunjian corpus fetch                    # download, verify and materialize the corpus
yunjian corpus status                   # corpus location, version, size, derived-structure state
yunjian search 明月 --limit 10 --json   # full-text and partial-line search
yunjian show <poem-id> --json           # text, tone pattern, rhyme, provenance, cited commentary
yunjian author 李白 --json              # author detail and work list
yunjian rhyme 七阳 --book pingshui      # rhyme-group search; --book is required, no implicit default
yunjian recite <poem-id> --mode cloze   # one typed round; the answer is read from stdin
yunjian recite due                      # items due today; does not read the corpus
yunjian models list                     # voice model registry and local cache state; offline
yunjian mcp                             # host the MCP server on stdio
```

### stdout / stderr / exit codes

**stdout carries results only** (human-readable text, or with `--json` exactly one line of JSON);
**stderr carries logs only**, even under `RUST_LOG=trace`. There are exactly four exit codes, and
machine callers should branch on them alone:

| Code | Meaning              | Correct response                                    |
| ---- | -------------------- | --------------------------------------------------- |
| 0    | success, has results | read `data`                                         |
| 1    | empty result set     | "I looked, there is none" — not an error            |
| 2    | usage error          | change the command (includes unshipped rhyme books) |
| 3    | data unavailable     | supply data, usually `yunjian corpus fetch`         |

**Never conflate 1 and 3**: reading a missing corpus as "there is no 李白 in the library" is the
most expensive mistake on this boundary.

The `--json` envelope has a fixed shape:

```json
{
  "schema_version": 1,
  "command": "search",
  "status": "ok",
  "warnings": [],
  "data": {}
}
```

`status` is `ok` / `empty` / `error`, mapping one-to-one onto the exit codes. When
`status == "error"`, `data` is replaced by `error` carrying a stable `code`, a message, and an
actionable `hint`. Every entry in `warnings` carries a stable `code` (such as `voice_fallback`); the
correct response to an unrecognised `code` is to relay the message verbatim.

### Registering with an MCP client

One command does it:

```bash
yunjian mcp install --client claude     # or --client opencode
```

Writing it by hand also works. **The two client shapes are not interchangeable** — the top-level key
differs, and so does the type of `command`:

Claude Desktop (`claude_desktop_config.json`): `command` is a **string**, arguments go in `args`.

```json
{
  "mcpServers": {
    "yunjian": {
      "command": "yunjian",
      "args": ["mcp"]
    }
  }
}
```

OpenCode (`opencode.json`): `command` is an **array including the argument**, plus `type` and
`enabled`.

```json
{
  "mcp": {
    "yunjian": {
      "type": "local",
      "command": ["yunjian", "mcp"],
      "enabled": true
    }
  }
}
```

Applying one shape to the other client yields an entry that is syntactically valid and semantically
empty: nothing errors, it simply never connects. If `yunjian` is not on `PATH`, replace `"yunjian"`
with an absolute path.

</details>

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
  the corpus. `yunjian mcp install --client <claude|opencode>` writes the client config for you —
  config shapes and `mcp install` are covered in [Using with an LLM](#using-with-an-llm).
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
- [Voice](VOICE.md) — models and licences, the 破读 lexicon, the v1 feedback contract
  (**pronunciation standard is not assessed**)
- [AI appreciation](AI.md) — BYOK, the two-tier cache, the pre-generation policy, labelling duty
- [Platform requirements](PLATFORM-REQUIREMENTS.md) — per-platform floors, the microphone permission
  chain, behaviour below the floor
- [Voice build](../VOICE-BUILD.zh.md) — native dependency builds across five targets, linking, and
  the licensing consequences
- [Third-party licences](LICENSES.md) — every bundled or downloaded third-party asset, its licence
  and its attribution

## Development

Requires Rust 1.95+. `rust-toolchain.toml` tracks stable for normal development, while CI runs a
separate compile gate on exactly Rust 1.95:

```bash
make hooks   # install pre-commit (format on commit) and pre-push (run make ci) hooks
make ci      # the only gate: format check + clippy + tests + MCP conformance + frontend tests
make help    # list every target
```

`cargo run -p xtask -- verify-sources --offline` checks upstream corpus licences and digests
offline; exit code 0 means the environment is correct.

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
