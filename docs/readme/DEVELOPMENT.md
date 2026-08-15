# Development

Requires Rust 1.95+. `rust-toolchain.toml` tracks stable for normal development, while CI runs a
separate compile gate on exactly Rust 1.95.

## Three commands

```bash
make hooks   # install pre-commit (format on commit) and pre-push (run make ci) hooks
make ci      # the only gate: format check + clippy + tests + MCP conformance + frontend tests
make help    # list every target
```

`cargo run -p xtask -- verify-sources --offline` checks upstream corpus licences and digests offline;
exit code 0 means the environment is correct.

## Commands that need a prerequisite artifact

A few xtask subcommands consume large gitignored artifacts that do not exist in a fresh checkout.
They **do not exit bare**: each names what is missing and which command produces it.

| Command                       | Prerequisite artifact                                              | How to supply it                                           |
| ----------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------- |
| `xtask corpus-package`        | `corpus/build/release/corpus.db` plus its `corpus-audit.db`        | Run `xtask corpus-build`, or take a `corpus-v*` release    |
| `xtask pregenerate`           | The same DB (opened read-only); the `dataset/README.md` disclosure | As above                                                   |
| `xtask provider-calls`        | The same DB (opened read-only)                                     | As above                                                   |
| `xtask corpus-build`          | Three upstream checkouts (~833 MB)                                 | Shallow-clone the SHAs pinned in `corpus/sources.toml`     |
| `cargo test -p yunjian-voice` | `models/cache/<model>`                                             | `yunjian models fetch <model>`, or set `YUNJIAN_MODEL_DIR` |

A corpus build measures at about 9 minutes (release build, 32 cores), and two independent builds
produce a byte-identical `corpus.db` — the build is deterministic. Voice tests that need an absent
model report as `ignored` with an explicit reason rather than passing silently.

## Releasing

A production release also requires the Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. If an updater signature is missing or does not match the public
key embedded in the app, the workflow fails and leaves the GitHub Release as a draft; **do not bypass
this gate by disabling signing.**

## CI runner status

The workflows record migration labels for the Linux x86_64 AWS CodeBuild runner project
`yunjian-runner`, but the `yunjian-github` CodeConnections connection is still `PENDING`. Every job
therefore remains GitHub-hosted until the connection is authorised in the AWS console and the
`WORKFLOW_JOB_QUEUED` webhook is recreated.

Windows, macOS, Linux ARM, and mixed-platform matrices **must not** move to that Linux x86_64 project.
When CodeBuild is enabled, each job must also retain a unique second label so GitHub's superset label
matching cannot route it to the wrong runner.

## Conventions

- **Commit subjects are Chinese imperatives in Conventional Commits form**, e.g.
  `feat(core): 添加韵部检索`.
- **No `println!`.** Logs go through `tracing` to stderr and a rolling file; the same binary hosts an
  MCP stdio server, and one stray line on stdout corrupts the protocol stream.
- **No secrets in `config.toml`.** The top-level config sets `deny_unknown_fields`, so pasting an
  `api_key` in errors out instead of being silently discarded.
- **Never ingest third-party modern annotation, translation or appreciation** — see
  [Content provenance and licensing](PROVENANCE.md) and `corpus/DENYLIST.md`.
- **Both READMEs have a line ceiling** (230 lines each), asserted by
  `crates/yunjian-corpus/tests/docs_completeness.rs`. Longer material belongs in `docs/`; the READMEs
  stay navigational — and **never restate something unimplemented as done to save lines.**

Issues and pull requests are welcome. Read [Corpus and indexing](CORPUS.md) before touching a corpus
source.
