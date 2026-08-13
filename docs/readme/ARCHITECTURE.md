[简体中文](../ARCHITECTURE.zh.md) · English

# Architecture

This document records the boundaries that are already in place and the reasoning behind them.
**Anything not yet wired is labelled as such rather than described as if it existed.**

## Contents

- [Workspace layout](#workspace-layout)
- [Why the core depends on no shell](#why-the-core-depends-on-no-shell)
- [Search routing: three branches, each with a reason to exist](#search-routing-three-branches-each-with-a-reason-to-exist)
- [Corpus resolution and atomic materialization](#corpus-resolution-and-atomic-materialization)
- [Long operations, events and cancellation: one protocol for the whole workspace](#long-operations-events-and-cancellation-one-protocol-for-the-whole-workspace)
- [IPC: what is settled and what is not yet wired](#ipc-what-is-settled-and-what-is-not-yet-wired)
- [Logging and the stdout ban](#logging-and-the-stdout-ban)

## Workspace layout

The root `Cargo.toml` lists 9 members (`resolver = "3"`):

| Crate            | Workspace deps                                                         | Features                                      | Purpose (from the crate-level `//!`)                                                                    |
| ---------------- | ---------------------------------------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `yunjian-core`   | **none**                                                               | none declared                                 | Config loading, logger init, unified error type, stable identity — domain logic independent of the host |
| `yunjian-corpus` | `yunjian-core`                                                         | none declared                                 | Ingestion, cleaning, stable-id minting, FTS5 index construction for public-domain corpora               |
| `yunjian-ai`     | `yunjian-core`                                                         | none declared                                 | AI appreciation generation, prompt-template versioning, multi-provider adaptation                       |
| `yunjian-recite` | `yunjian-core`                                                         | none declared                                 | Alignment scoring kernel and FSRS review scheduling                                                     |
| `yunjian-voice`  | **none**                                                               | `default=[]`, `capture`, `download`, `voice`  | Per-foot synthesis, silence-spliced rhythm control, audio capture                                       |
| `yunjian-mcp`    | `yunjian-core`, `yunjian-ai`                                           | `http` (no `default`)                         | MCP server, stdio by default                                                                            |
| `yunjian-cli`    | `yunjian-core`, `yunjian-ai`, optional `yunjian-mcp` / `yunjian-voice` | `default=["mcp"]`, `mcp`, `mcp-http`, `voice` | The `yunjian` binary; stdout carries results only, stderr carries logs only                             |
| `yunjian-app`    | `yunjian-core`                                                         | none declared                                 | Desktop shell; **also plays the `src-tauri` role** in Tauri's convention                                |
| `xtask`          | `yunjian-core`, `yunjian-corpus`, `yunjian-ai`, `yunjian-voice`        | `default=[]`, `voice`                         | Repository task runner (`cargo xtask <subcommand>`)                                                     |

Two layout details are easy to get wrong, so they are stated here:

- **There is no `app/src-tauri/` directory.** `app/` is the React frontend (`index.html`, `src/`,
  `vite.config.ts`); the Tauri-side manifest and configuration live in `crates/yunjian-app/`
  (`tauri.conf.json`, `tauri.macos.conf.json`), whose `Cargo.toml` states explicitly that the crate
  takes the `src-tauri` role.
- **The desktop executable is `yunjian-desktop`, not `yunjian`.** The latter is already taken by
  `yunjian-cli`'s `[[bin]]`; a name collision would overwrite one with the other and cargo does not
  report this particular conflict.

Neither `yunjian-core` nor `yunjian-voice` depends on any other workspace crate. For the former this
is the deliberate layering rule (next section); for the latter the effect is that the voice stack can
be cut out independently of the corpus and the scoring kernel.

## Why the core depends on no shell

The crate-level doc of `yunjian-core` states the constraint as non-negotiable: the crate never learns
that Tauri exists, introduces no `tauri::` types, and assumes no desktop host, so that the mobile
implementation space stays open.

Two **real** tests enforce it, and both judge the `Cargo.toml` direct dependencies rather than source
strings:

- `dependency_manifest_excludes_shell_and_rejected_search_engines` parses the crate's own
  `[dependencies]` and asserts the absence of `tauri`, `tantivy`, `jieba-rs`, `lindera` and
  `opencc-rust`.
- `this_crate_declares_no_tauri_dependency` strips comments after `#` line by line and then asserts
  the code portion contains no `tauri`.

**Judging the dependency rather than the source is deliberate**: without the dependency, any
`use tauri::...` fails to compile, so the gate blocks a whole class of problems rather than one way of
writing them. The scope must be stated honestly: these gates verify the **direct** dependency
boundary, not a `cargo tree` audit of the full transitive graph.

What the boundary buys is concrete: the mobile shell choice (one shared framework vs native code over
the same library) can be switched without touching any poetry logic. The plan hangs that choice on a
real on-device measurement rather than betting on one side early.

Two related constraints live at the same layer, for the same reason:

- **Search is a SQLite file, not a search engine.** No tantivy (its `MmapDirectory` is unsupported on
  Android), no jieba or lindera (the correct segmentation granularity for classical Chinese is the
  character, not the word).
- **No traditional/simplified conversion dictionary at runtime.** `ferrous-opencc` appears only in
  `yunjian-corpus`'s dependencies; conversion happens at build time, and at runtime the `variant_map`
  produced by the same build rewrites queries character by character.

## Search routing: three branches, each with a reason to exist

Routing enters at `yunjian_core::search::query::plan_query(handle, query)`, with two threshold
constants: `TRIGRAM_CHARS = 3` and `WHOLE_LINE_MIN_CHARS = 5`. The `QueryPlan` branches on the
wildcard-free path:

| Query length | Plan                                                                                         | Which table                                                         | Why this branch must exist                                                                                                                                                                 |
| ------------ | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1–2 chars    | `NgramCandidates`; falls back to `FullScan` with a reason when derived structures are absent | `ngram` (index `ngram_gram_idx`), then verified against `poem.body` | FTS5 trigram **derives no constraint at all** below three characters, so `LIKE '%明月%'` degrades to a virtual-table scan. And 明月 / 相思 / 李白 are exactly the most common real queries |
| 3 chars      | `Match` (phrase expression)                                                                  | `poem_fts MATCH`                                                    | Exactly one trigram — the shortest form FTS5 can consume directly                                                                                                                          |
| 4 chars      | `Like`                                                                                       | `poem_fts ... WHERE f.body LIKE`                                    | The trigram index can constrain it, but the phrase-matching benefit does not justify additionally requiring `detail=full`                                                                  |
| ≥5 chars     | `Match` when `index_detail_mode() == "full"`, otherwise `Like`                               | `poem_fts`                                                          | Whole-line search needs position information; once `detail` is not `full`, phrase matching is unavailable at the SQLite level, and the honest response is to fall back rather than error   |

Patterns containing `%` or `_` take a separate path: when the longest literal run is shorter than 3
characters the plan is `FullScan` with the warning that the pattern has no three consecutive literal
characters and therefore cannot constrain the trigram index; otherwise it is `Like`.

**The three search structures are deterministic derivations of `poem.body`**, named by the constant
`DERIVED_TABLES = ["ngram", "poem_fts", "poem_last_char"]`. They are not shipped in the artifact; the
application builds them locally on first launch (`yunjian_core::derive`). The reason is size — see
[Corpus and indexing](CORPUS.md).

The `detail` setting of `poem_fts` is a measured verdict, not a guess, recorded in
`corpus/reports/index-mode.json`: `chosen_mode = "full"`, `ngram_aux_enabled = true`. Two
measurements ruled out the alternatives: `detail=none` fails outright on a whole five-character line
with `phrase queries are not supported (detail!=full)`; with the n-gram table off, two-character
queries extrapolate to 428.0 ms at release scale. The verdict is pinned by
`chosen_configuration_violates_neither_gate` in `crates/yunjian-core/tests/index_mode_verdict.rs`,
and the runtime value comes from `corpus_meta.index_detail_mode` — so **changing the verdict changes
the index that actually gets built**, and the contracts go red immediately.

## Corpus resolution and atomic materialization

The call chain is `CorpusHandle::open` → `open_with_progress` → `resolve` → `connect_read_only` →
`open_corpus` → `CorpusMeta::read` → `ensure_derived`.

`resolve` has three tiers, and **an explicitly-given tier that is missing is an error, never a silent
downgrade**:

1. the explicit `cfg.path`;
2. `cfg.data_dir.join("corpus.db")`;
3. a `.db.gz` archive → read the digest expectation from `manifest.json` → sweep stale temporaries →
   `materialize`.

Read-only opening is **two** safeguards, not one: `Connection::open_with_flags(path,
OpenFlags::SQLITE_OPEN_READ_ONLY)` makes the descriptor unwritable, and `PRAGMA query_only = true`
also closes off any later `ATTACH`. Each worker opens its own read-only connection, because
`rusqlite::Connection` is not `Sync`.

Materialization runs "verify → same-directory temp file → `fsync` → validate before rename →
`rename` → `fsync` the directory":

1. compare the archive's byte length;
2. compare `sha256_of_file` against the expected digest, stating explicitly on failure that **no file
   was written**;
3. mint a unique `.tmp` name in the target's own directory (same directory is required: `rename`
   across filesystems is not atomic);
4. decompress, then `writer.flush()` followed by `sync_all()`;
5. `validate_materialized` verifies the schema and `corpus_meta` over a read-only connection —
   **before the rename**;
6. `rename(temp, target)`, then a best-effort `sync_all()` on the parent directory.

The failure path removes the temporary file; even if removal fails, a later run will not mistake it
for `corpus.db`, and `sweep_stale_temps` clears anything matching the prefix plus the `.tmp` suffix
before the next materialization.

## Long operations, events and cancellation: one protocol for the whole workspace

`yunjian_core::operation` is the single workspace-wide protocol for long-operation events,
cancellation and resource release. The five variants of `Event<P, I>` have distinct duties:

- `Progress(P)` — a **coalescible** progress snapshot;
- `Item(I)` — an increment that **must not be dropped**;
- `Done` / `Cancelled` / `Failed { message }` — three terminal events; `message` is already redacted.

The consumer side is `OperationHandle<P, I>`, the producer side is `OperationReporter<P, I>`, and the
queue bound is `EVENT_QUEUE_CAPACITY = 256`.

This protocol lives in `yunjian-core` rather than in a shell as a direct consequence of the
no-shell rule: streaming AI appreciation, first-launch corpus derivation and model download are all
long operations, and they must run identically under the CLI and MCP where no Tauri exists.
`yunjian-ai`'s streaming appreciation uses `OperationHandle` as its only outward handle and then
propagates cancellation into the HTTP stream through `tokio_util::sync::CancellationToken`.

## IPC: what is settled and what is not yet wired

**Settled and already in place:**

- The desktop shell is `crates/yunjian-app`, and logging goes through `yunjian_core::init_logger`
  rather than Tauri's official log plugin — so all three entry points (CLI, MCP stdio, desktop GUI)
  share one level parser, one credential redactor and one rolling-file layout. A plugin would add a
  second format and a second filtering semantics, and the difference between two logging conventions
  only surfaces when logs matter most.
- The init order cannot be swapped: `init_config` first (without config the level and directories are
  unknown), then `init_logger`; the returned guard must be bound to a named local that lives until
  process exit — assigning it to `_` drops it immediately and stops the background file writer early.
- The event, progress and cancellation protocol for long operations already exists and is
  shell-independent (previous section).

**Not yet wired, recorded honestly:** the only Tauri construction code in
`crates/yunjian-app/src/lib.rs` today is

```rust
tauri::Builder::default()
    .run(tauri::generate_context!())
    .expect("启动 Tauri 应用失败");
```

There is **no** `#[tauri::command]` anywhere in the repository, no `tauri::generate_handler!` or
`.invoke_handler(...)`, no `tauri::ipc::Channel`, no `.emit(...)` event stream and no
`spawn_blocking`. The crate doc itself describes the IPC command table as something a later task
wires up. This section therefore **cannot** claim that Channel streaming or the event system is in
use; that is future work, and when it lands, the `Progress` / `Item` split of `OperationHandle` is
what it maps onto.

One measured trap in the platform-override configuration is worth recording at the architecture
level: `app.windows` is an array and is **replaced wholesale** under RFC 7396 rather than merged
field by field, so omitting any geometry field silently reverts macOS to the serde defaults
(800×600, no minimum size) while Linux and Windows behave normally.

## Logging and the stdout ban

**Logs go to stderr and a rolling file, never to stdout.** The reason is that the same binary hosts an
MCP stdio server, and one line of noise on stdout destroys the protocol stream.

The ban is scoped to the **whole workspace**, not to `yunjian-mcp` alone: a `println!` inside
`yunjian-core` reached from an MCP tool handler damages the protocol stream exactly as much as one
written in the MCP crate, and "the MCP code path" is not a concept a lint can express. So the
mechanism is a workspace-level `deny` plus one exemption (the CLI presentation module), referenced by
every member through `[lints] workspace = true`, **with no exceptions**.

Three measured boundaries:

- clippy's `print_stdout` only catches macros; a direct `std::io::stdout().write_all()` slips past, so
  `clippy.toml` additionally lists `std::io::stdout` under `disallowed-methods`.
- `std::println` is **deliberately not** in `disallowed-macros`: `print_stdout` has a built-in
  exemption for build scripts and `disallowed_macros` does not, so adding it would break the only
  channel a build script has for talking to cargo — and a build script's stdout is captured by cargo
  and never reaches the process stdout anyway.
- `std::process::Stdio::inherit()` is not blocked, a known residual gap: a child process inheriting
  stdout can pollute the protocol stream just as well. It is unblocked because the MCP conformance
  harness and existing subprocess tests both need `Command`. The current compensation is convention
  only (write `Stdio::null()` / `Stdio::piped()` explicitly), not a mechanism.

## Related documents

- [Corpus and indexing](CORPUS.md) — sources and licences, identity model, index measurements, commentary admission
- [Voice](VOICE.md) — models and licences, the 破读 lexicon, the v1 feedback contract (no pronunciation-standard assessment)
- [AI appreciation](AI.md) — BYOK, the two-tier cache, the pre-generation policy and its current state
- [Platform requirements](PLATFORM-REQUIREMENTS.md) — per-platform floors and the microphone permission chain
- [Third-party licences](../../LICENSES.md) — per-asset licence and attribution
