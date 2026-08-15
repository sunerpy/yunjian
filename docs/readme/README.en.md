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
- [Quick start](#quick-start)
- [Using with an LLM](#using-with-an-llm)
- [Feature overview](#feature-overview)
- [Content provenance and licensing](#content-provenance-and-licensing)
- [Documentation](#documentation)
- [Development](#development)

## Project status

Implemented on `main` **and covered by tests**:

| Component           | Status      | Notes                                                                                                        |
| ------------------- | ----------- | ------------------------------------------------------------------------------------------------------------ |
| Engineering base    | done        | 8-crate workspace, pinned versions, runtime `config.toml` discovery, tracing to stderr only                  |
| stdout ban          | done        | Clippy denies `println!` and `std::io::stdout` workspace-wide, one CLI exit                                  |
| Licence gate        | done        | `verify-sources` / `verify-models` check every asset's licence and SHA-256, MIT / Apache-2.0 only            |
| Corpus pipeline     | done        | Canonical records, append-only `stable_id`, 平水韵 and 词林正韵 ingested, dedup and conflict verdicts        |
| Index selection     | measured    | FTS5 `detail` mode and the n-gram table settled by measurement, not by decree                                |
| Corpus artifact     | done        | 474k Tang–Song poems bundled (211 MiB gzip), search structures derived on first launch                       |
| Voice base          | verified    | Native build and linking on five targets, 16 kHz mono capture on Linux, per-platform permissions             |
| Classical CER       | measured    | Measured on synthetic speech; the verdict is that CER is advisory only, never a score                        |
| Core search         | done        | Body, title, author, dynasty, first line, last character, tag and rhyme search plus poem detail              |
| Command line        | done        | `yunjian search/show/author/rhyme/corpus`, stable `--json` envelope, exit codes 0/1/2/3                      |
| MCP server          | done        | `yunjian mcp` over stdio with three read-only tools; `mcp install` writes both client shapes                 |
| AI appreciation     | done        | BYOK in the OS keychain only, cancellable streaming, two-tier cache, open-weight pre-generation              |
| Recitation practice | done        | Cloze / first-character / masking modes on one scoring kernel, FSRS review scheduling                        |
| Read-aloud and ASR  | done        | Per-音步 TTS with the 破读 lexicon, streaming dual-decode recognition; `voice` is off by default             |
| Desktop app         | done        | Tauri v2 + React: custom titlebar, reading, settings, recitation and voice, non-blocking IPC                 |
| The mobile app      | façade done | `yunjian-mobile` covers all four domain crates; no binding is built while the device verdict is undetermined |

**Not implemented, or not yet verified**:

- **The mobile shell and distribution** — the device verdict remains `undetermined`; the binding, UI and distribution pipeline still need physical Android / iOS devices, `adb` and signing identities.
- **Real-machine desktop acceptance** — of 20 pre-declared assertions, 3 PASS and 17 NOT EXECUTED on Linux (under a GPU-less container plus Xvfb, WebKitGTK composites into a GL surface X cannot read back); Windows / macOS have no interactive session and no signing identity.
- **The first tag** — the release pipeline now covers five CLI targets, desktop installers on three platforms, updater signatures, and per-asset SHA-256 files, but no first release tag has been cut; real-machine desktop acceptance and signing credentials remain release prerequisites.
- **The bundled appreciation dataset** — the pipeline and its gates exist, but no open-weight inference is available here, so `dataset/` holds only a README; not one line was fabricated.
- **The 词谱 line-pattern table** — only 念奴娇 and 水调歌头 are covered, and from the measured mode of 全宋词 rather than a public-domain 词谱; every other tune degrades honestly to punctuation.

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

Both scripts detect the OS and CPU architecture, resolve the matching release asset, and **verify its SHA-256 before anything lands on disk** — a checksum failure installs nothing. Environment variables, private-repository access, the five CLI archives and the desktop installers are in [Installation and release artifacts](INSTALL.md).

> [!NOTE]
> The first tagged release (`v0.1.0`) has not been cut yet, so the commands above are waiting on it.
> Until then build from source with `cargo build --workspace --release -p yunjian-cli`; the binary
> lands at `target/release/yunjian`.

## Using with an LLM

Results go to stdout, logs go to stderr, and success is decided by the exit code alone. The block
below can be handed to an AI assistant as-is.

<details>
<summary>Commands, output contract, and MCP client configuration</summary>

### Key commands

```bash
yunjian corpus fetch                    # download, verify and materialize the corpus
yunjian search 明月 --limit 10 --json   # full-text and partial-line search
yunjian show <poem-id> --json           # text, tone pattern, rhyme, provenance, cited commentary
yunjian author 李白 --json              # author detail and work list
yunjian rhyme 七阳 --book pingshui      # rhyme-group search; --book is required, no implicit default
yunjian recite <poem-id> --mode cloze   # one typed round; the answer is read from stdin
yunjian mcp                             # host the MCP server on stdio
```

`corpus status`, `recite due`, `models list` and every subcommand option are in [CLI](../CLI.zh.md).

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

**Never conflate 1 and 3**: reading a missing corpus as "there is no 李白 in the library" is the most
expensive mistake on this boundary.

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

`yunjian mcp install --client claude` (or `--client opencode`) does it in one command. Writing it by
hand also works, but **the two client shapes are not interchangeable** — the top-level key differs,
and so does the type of `command`:

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

This is the **intended** shape; actual progress is always [Project status](#project-status).

- **Offline search.** A read-only SQLite file: no network, no login, no account. 474k Tang–Song poems ship by default, the full 900k is an in-app download. See [Corpus and indexing](CORPUS.md).
- **AI appreciation, bring your own key.** The key lives in the OS keychain, never in an environment variable and never in a config file. It works fully without a key: the bundled set is generated by open-weight models only. See [AI appreciation](AI.md).
- **Recitation practice.** Three typed modes (blanked, first-character hint, masked) share one scoring kernel. Character accuracy on the voice path is **always an estimate, never a score** — speech recognition on classical Chinese is not reliable enough to grade you.
- **Read-aloud.** Per-foot synthesis with silence spliced in on the Rust side, using the project's own public-domain-sourced classical reading table; outside its coverage it falls back to modern Mandarin. See [Voice](VOICE.md).
- **MCP server.** `yunjian mcp` speaks stdio so clients like Claude Desktop and OpenCode can query your library directly. Generated output is labelled as AI-written and never written back into the corpus.
- **Multiple surfaces.** Tauri v2 + React on the desktop; a CLI with machine-readable output; the mobile framework is chosen by real-device measurement.

## Content provenance and licensing

In one line: **the copyright wall is the architecture** — every open dataset carrying modern
annotation, translation or appreciation has a licence chain that does not hold up, so 云笺 combines
only public-domain source text, cited pre-modern commentary, and clearly labelled AI appreciation.

- **The code** is [MIT](../../LICENSE) licensed. **The shipped corpus contains only public-domain source text plus MIT-licensed upstream compilation output**: per-asset verdicts in [`corpus/sources.toml`](../../corpus/sources.toml), rejections with reasons in [`corpus/DENYLIST.md`](../../corpus/DENYLIST.md). Verification is per file, not per repository.
- **Appreciation text is AI-generated, not scholarship**, rendered in a different visual register from cited commentary and disclosed as unreviewed; the bundled set uses open weights, never a closed API.
- **The `voice` feature is off by default** and a default build is pure MIT; with it enabled the artifact statically contains GPL-3.0 espeak-ng, so a distributed voice build must be offered under GPL-3.0 as a whole. No model weights are bundled.
- **Character accuracy on the voice path is advisory, never a score** — see the [CER report](../reports/asr-cer.md).
- The full verdict chain and per-asset attribution are in [Content provenance and licensing](PROVENANCE.md) and [Third-party licences](LICENSES.md).

## Documentation

- [Architecture](ARCHITECTURE.md) — layering, why `yunjian-core` never learns Tauri exists, the mobile escape hatch
- [Corpus and indexing](CORPUS.md) — build pipeline, the `stable_id` identity model, the measured FTS5 index selection
- [CLI](../CLI.zh.md) — subcommands, the `--json` envelope schema, the four exit codes, stdout/stderr split
- [AI appreciation](AI.md) — BYOK, the two-tier cache, the pre-generation policy, labelling duty
- [Voice](VOICE.md) — models and licences, the 破读 lexicon, the v1 feedback contract (**pronunciation standard is not assessed**); builds in [Voice build](../VOICE-BUILD.zh.md)
- [Installation and release artifacts](INSTALL.md) — installer variables, private repos, archives, installers
- [Content provenance and licensing](PROVENANCE.md) — how the copyright wall shapes the architecture
- [Development](DEVELOPMENT.md) — the gate, commands needing a prerequisite artifact, release credentials
- [Platform requirements](PLATFORM-REQUIREMENTS.md) — per-platform floors, the microphone permission chain
- [CER report](../reports/asr-cer.md) — measured classical-Chinese ASR accuracy and why it stays advisory
- [Third-party licences](LICENSES.md) — every bundled or downloaded asset, its licence, its attribution

## Development

Requires Rust 1.95+. `make hooks` installs the commit and push hooks, `make ci` is the only gate
(format + clippy + tests + MCP conformance + frontend tests), and `make help` lists every target.

A few xtask subcommands consume large gitignored artifacts (the corpus database, voice models) that a fresh checkout does not have; they do not exit bare but name what is missing and which command produces it. The full list, release credentials, CI runner status and the commit conventions are in [Development](DEVELOPMENT.md).

Three hard rules: **commit subjects are Chinese imperatives in Conventional Commits form**; **no `println!`** (the same binary hosts an MCP stdio server, and one stray line on stdout corrupts the protocol stream); **no secrets in `config.toml`**, and never ingest third-party modern annotation, translation or appreciation. Issues and pull requests are welcome — read [Corpus and indexing](CORPUS.md) before touching a corpus source.
