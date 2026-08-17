[简体中文](../AI.zh.md) · English

# AI appreciation

AI appreciation is not a nice-to-have in Yunjian — **it fills a hole**. No dataset carrying modern
annotation, translation or appreciation has a defensible chain of rights (see
[Corpus and indexing](CORPUS.md)), so the shipped content can only ever be the combination
"public-domain source text + pre-modern commentary with per-entry citations + clearly-labelled AI
appreciation". This document covers how the key is stored, how the cache saves money, how
pre-generation works, **and how pre-generation's current state differs from its target state**.

## Contents

- [BYOK: the key comes from the OS keychain, never from an environment variable](#byok-the-key-comes-from-the-os-keychain-never-from-an-environment-variable)
- [Linux keyutils is memory-only and dies on reboot (must be stated plainly)](#linux-keyutils-is-memory-only-and-dies-on-reboot-must-be-stated-plainly)
- [The key never appears in Debug, logs or a URL](#the-key-never-appears-in-debug-logs-or-a-url)
- [The two-tier cache: why the shipped tier must be provider-independent](#the-two-tier-cache-why-the-shipped-tier-must-be-provider-independent)
- [Provider boundary and no-key operation](#provider-boundary-and-no-key-operation)
- [Streaming and real cancellation](#streaming-and-real-cancellation)
- [Pre-generation policy](#pre-generation-policy)
- [Pre-generation's current state: generated (recorded honestly)](#pre-generations-current-state-generated-recorded-honestly)
- [Labelling duty and the accuracy disclaimer](#labelling-duty-and-the-accuracy-disclaimer)
- [What does not exist yet (recorded honestly)](#what-does-not-exist-yet-recorded-honestly)

## BYOK: the key comes from the OS keychain, never from an environment variable

Yunjian is BYOK (bring your own key): the user supplies their own API key, and the project proxies no
requests and holds no credentials. The multi-provider client is `genai` (`0.6`, with
`default-features = false` plus an explicit `rustls-tls`).

Key storage lives in `yunjian-ai::keystore`, over the `keyring_core::CredentialStore` abstraction.
**Only `keyring-core` plus per-platform stores are used; the `keyring` facade crate is deliberately
avoided** — its own documentation says applications "should not be linking to this library at all",
and it would pull all five platforms' stores into the dependency graph at once, putting the Linux
D-Bus stack into a Windows build.

`Backend` has eight variants: `SecretService`, `Keyutils`, `WindowsCredential`, `AppleKeychain`,
`AndroidKeystore`, `SessionMemory`, `PlaintextFile`, `Absent`. Actual per-platform probing:

| Platform | Preferred                                         | Fallback                                     |
| -------- | ------------------------------------------------- | -------------------------------------------- |
| Linux    | `zbus_secret_service_keyring_store::Store::new()` | `linux_keyutils_keyring_store::Store::new()` |
| macOS    | `apple_native_keyring_store::keychain::Store`     | —                                            |
| iOS      | `apple_native_keyring_store::protected::Store`    | —                                            |
| Windows  | `windows_native_keyring_store::Store`             | —                                            |
| Android  | `android_native_keyring_store::Store`             | —                                            |

**The overall chain** is: the OS keychain → if none is available and `allow_plaintext_file == true`,
`PlaintextFile` → otherwise `SessionMemory`. **A plaintext file is not an automatic last resort but an
explicit opt-in** — "the keychain failed, so quietly write plaintext" is the most easily accepted and
least defensible default there is.

On Linux and headless hosts the real chain is `SecretService → Keyutils → SessionMemory`. Secret
Service needs a D-Bus session **and** a real secrets daemon, neither of which exists in CI, containers
or a bare SSH session, so a failing `Store::new()` is **an expected path, not an exception**. The
integration test `linux_no_dbus.rs` confirms by measurement that without D-Bus the selection is
`Backend::Keyutils` with a reported `Persistence::LoginSession`.

**Source reporting is not a single `KeySource` enum but a four-field `StorageReport`:**
`backend: Backend`, `persistence: Persistence`, `protection: Protection`, `location: String`. The UI
label is **derived from `persistence`, not from the backend name** — see the next section for why.

## Linux keyutils is memory-only and dies on reboot (must be stated plainly)

**This has to be written out explicitly, because calling it "the system keychain" would be a false
statement.**

The documentation on `Backend::Keyutils` reads: the Linux kernel keyutils, **memory-only, lost on
reboot**. The corresponding `Persistence::LoginSession` is documented as living until logout or
reboot, with keyutils in that tier. Upstream calls it "strongly recommended" on headless Linux
(it is part of the kernel and always available) while explicitly requiring callers to "prepare for
`Entry::get_password` to fail".

So:

- `settings_summary()` renders this tier as the kernel session keyring that **is lost after a reboot or
  logout, at which point the key must be entered again** — **the product promises no persistence**.
- **The key-read path must handle absence and ask again**, which is not defensive programming but the
  normal behaviour of this tier.
- The test `keyutils_persistence_maps_to_login_session_not_persistent` asserts explicitly that
  `UntilReboot → LoginSession` and that it **is not** `Persistent`.

Keyutils and Secret Service are both "keychains", but their **persistence is fundamentally different**,
which is the entire reason the `StorageReport.persistence` field must exist and the UI label must be
derived from it.

## The key never appears in Debug, logs or a URL

Four mechanisms, each with tests:

- **No environment variable.** `auth_resolver` returns only `Ok(Some(AuthData::Key(...)))` or `Err` and
  **never `Ok(None)`**, which would trigger `genai`'s `AuthData::FromEnv` fallback. The test
  `the_resolver_key_reaches_the_authorization_header_without_touching_the_environment` verifies that
  the resolver's key forms the `Authorization` header directly, and compares the set of environment
  variables whose names contain `KEY` / `TOKEN` / `SECRET` before and after the call for equality.
- **Not in `Debug`.** `OsKeychain::fmt` does not render the underlying `CredentialStore`;
  `KeyStore::fmt` renders only `service` and `tier_report()`, not the tier structure holding a
  `SecretString`; `GenAiProvider::fmt` does not render the `client` that captured the key, reporting
  only non-secret fields such as `key_configured: bool`. `keystore_errors_never_render_the_key`
  checks both `Display` and `Debug`, and
  `debug_output_of_keystore_and_report_carries_no_secret` checks `KeyStore` and `StorageReport`.
- **Not in logs.** Platform errors pass through `redact_credentials(&err.to_string())` before reaching
  `tracing::info!`.
- **Not in `config.toml`.** The top-level config sets `deny_unknown_fields`, so pasting an `api_key`
  into it errors rather than being silently discarded. The cost is forward compatibility — an older
  binary errors on a new key in a newer config; that is a deliberate trade-off, not an oversight.

The redactor itself had two real, measured defects whose fixes are worth recording because they show
that **positive and negative cases both need tests**:

1. `--api-key hunter2xyz` (the whitespace-separated command-line shape) slipped through entirely at
   first. The fix accepts whitespace as a separator **only when the key name is immediately preceded by
   `-`** — accepting whitespace unconditionally is wrong, because it would launder "missing token
   configuration" into "missing token <redacted>".
2. Treating `Token` / `Basic` as auth-scheme words recognizable standalone in free text produces
   widespread false positives in Chinese diagnostics. HTTP headers are case-insensitive, so
   case cannot disambiguate; the only fix is to shrink the vocabulary: in free text only `Bearer` is
   recognized, and the other four words apply only after a credential key name.

**Testing only "the key was scrubbed" yields an implementation that launders diagnostics into a field
of placeholders**, so both cases have regression guards
(`keeps_prose_that_merely_mentions_a_credential_name`,
`redacts_named_credentials_regardless_of_value_shape`).

## The two-tier cache: why the shipped tier must be provider-independent

Two tables, with the schema in `crates/yunjian-ai/schema-cache.sql`:

| Table                  | Tier         | Contents                                                         |
| ---------------------- | ------------ | ---------------------------------------------------------------- |
| `appreciation_shipped` | Shipped tier | Pre-generated appreciations distributed with the corpus artifact |
| `appreciation_cache`   | Local tier   | Results the user generated with their own key                    |

**Lookup checks the local tier first and the shipped tier second.** A request carrying a `style` (a
user-defined tone) **does not consult the shipped tier** — the shipped text was not generated in that
style, and passing it off as such would be wrong.

**The local key is a BLAKE3 over six items plus temperature**: `provider.as_str()`,
`request.model()`, `request.style().unwrap_or("")`, `request.template_version()`, `stable_id`,
`grounding_digest` (each followed by a `0` separator byte), finally mixing in
`temperature().to_bits().to_le_bytes()`.

**The shipped key is only `(stable_id, template_version)`** — no provider, no model, no style, no
temperature — after which `grounding_digest` is verified separately; on return the provider is
constructed as the fixed `ProviderId("shipped")`.

**That is the whole meaning of the shipped tier being "provider-independent", and the reason it can
actually save money.** If the shipped key contained the provider, a user on DeepSeek could never hit a
row pre-generated with a different model, and the shipped dataset would effectively not exist for them
— at which point it would be decoration valid only for users of one particular provider. The test
`shipped_hit_is_provider_independent_and_performs_zero_provider_calls` uses a `CountingProvider` with a
deliberately different provider id and asserts the text comes from the shipped row **and that
`provider.calls() == 0`**; an end-to-end test at the MCP entry point asserts
`provider.appreciate_calls() == 0`. **This assertion is falsifiable, not ornamental.**

**Eviction touches only `appreciation_cache`; shipped rows are never evicted** (held by
`lru_eviction_removes_only_user_paid_rows`).

**One honest note where the wording disagrees with the implementation:** the eviction SQL is
`ORDER BY created_at ASC, key ASC LIMIT ?1` and a lookup **does not refresh** `created_at`, so the
actual semantics are **FIFO by creation time, not LRU** — despite the index being named
`appreciation_cache_lru(created_at, key)`. The trade-off is itself reasonable: refreshing `created_at`
would overwrite generation time with access time and corrupt provenance. The proper fix is a separate
`last_accessed_at` column that eviction orders by, leaving `created_at` with its generation semantics.
**The difference currently affects hit rate only, not correctness or the cost conclusion**, but the
index name would mislead the next reader, so it is recorded here.

## Provider boundary and no-key operation

The appreciation boundary is the trait `AppreciationProvider` with three methods:

```rust
async fn appreciate(&self, AppreciationRequest) -> Result<Appreciation>;
async fn appreciate_stream(&self, AppreciationRequest)
    -> Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>>;
fn id(&self) -> ProviderId;
```

Poem generation is `PoemGenerationProvider::generate_poem`, and the combined trait is
`AiProvider: AppreciationProvider + PoemGenerationProvider`. **`PoemGenerationProvider` has only
`generate_poem` and offers no write entry point at all** — this is isolation at the interface level,
not a runtime check.

**Prompt templates are versioned and embedded at compile time:**
`APPRECIATION_TEMPLATE_FILE = "appreciation.1.0.0.md"`,
`APPRECIATION_TEMPLATE_VERSION = "1.0.0"`, with the body embedded via `include_str!`.
`PromptTemplate::register(name, file_name, version, source)` validates three things: the version is a
numeric semver, the file name is **strictly** `{name}.{version}.md`, and the body is non-empty.
`AppreciationRequest::render_prompt()` substitutes `{{grounding}}` with the corpus fact block and then
appends the optional style.

**The template version enters the cache key**, so changing the template cannot let old cache entries
pass as the new template's output — that is what "versioned" actually does here, not bookkeeping.

**No-key operation is a first-class requirement, not a degraded mode.** All three `NullProvider` entry
points (appreciation, streaming appreciation, poem generation) return the typed
`Error::AiKeyNotConfigured { provider }`. The MCP behaviour is:

1. **check both cache tiers first** — so shipped appreciations work with no key configured at all;
2. on a miss with no provider (or a provider returning `AiKeyNotConfigured`), return an **ordinary
   result** of `configuration_required` together with the settings path `AI_SETTINGS_PATH`, **not a tool
   error**.

The distinction is substantive: a tool error makes an MCP client treat the situation as a fault, whereas
"no key configured" is a normal state with an obvious next step.

## Streaming and real cancellation

The single outward handle is `yunjian_core::operation::OperationHandle` (see
[Architecture](ARCHITECTURE.md)). Internally, `tokio_util::sync::CancellationToken` propagates
cancellation into the HTTP stream:

- on the worker side, `reporter.wait_for_stop(Duration::from_millis(2))` plus a closed or refusing sink
  triggers `cancellation.cancel()`;
- on the HTTP side, both **establishing the stream** and **reading each event** use `tokio::select!`
  against `cancellation.cancelled()`, and the send channel races cancellation through
  `send_unless_cancelled`.

**Cancellation really cancels rather than discarding later results.** The test
`cancelling_mid_stream_stops_chunks_within_100_ms_and_never_caches_partial_text` asserts that after
cancelling, an `Event::Cancelled` arrives within 100 ms, with **zero additional chunks** and a **cache
length of 0**.

**Only a complete result is cached.** The cache-write boundary is the trait
`AppreciationCacheWriter`, documented verbatim as the cache-write boundary that accepts complete
appreciations and nothing else; the implementation calls `cache_writer.store_completed` only on
`ChatStreamEvent::End`, with a non-empty full body, and **only if not cancelled**. A truncated
appreciation in the cache would show the user a cut-off text that the system believes is complete —
considerably worse than not caching.

## Pre-generation policy

The shipped appreciation dataset is produced by `cargo run -p xtask -- pregenerate`.

**Open-weight models only, no closed API. This is a licensing constraint, not a performance
preference.** Two of three closed providers' output terms are unknown: Anthropic's commercial terms
assign Output rights to the customer while forbidding training a competitor; **OpenAI's corresponding
terms could not be verified (the site returned 403 during research); DeepSeek's terms are entirely
unverified**. Downloaded open weights carry no term restricting how their output may be redistributed,
which sidesteps the uncertainty.

The constraint is enforced in code: before generating a single record, pregenerate validates the weight
licence (**MIT and Apache-2.0 only**) and the runtime, and aborts naming the open-weight requirement if
configured against a closed API provider.

**Coverage selection has two paths, and the preferred one takes precedence:**

- `select_by_poem_tag()` — the SQL is
  `SELECT DISTINCT poem_id FROM poem_tag WHERE tag IN (...) ORDER BY poem_id`, selecting by anthology
  tag. **This is first in order.**
- `select_by_roster()` — on a miss, resolves `(author, title)` from the reviewed roster in
  `tags.toml`.

**Which path was used goes into the manifest**: `CoverageSelector` has two variants, `PoemTag` and
`ReviewedRoster`, with the stable strings `"poem_tag"` and `"reviewed_roster"`. **So the fallback is not
silent** — anyone reading the manifest can see the preferred path was not the one taken.

**Appreciations generated with a user's key never enter the shipped dataset.** This is enforced by
`ensure_readable_table()`: pregeneration **may read only** `appreciation_shipped` and **fails hard** on
`appreciation_cache`; the resume helper `existing_pregenerated_ids()` likewise queries only the shipped
table.

Key `DatasetManifest` fields: `template_version`, `coverage_selector`, `generation_executed`,
`not_executed_reason`, `corpus_version`.

## Pre-generation's current state: generated (recorded honestly)

**The current checkout contains a 16-record appreciation dataset whose every body is genuine
open-weight model output.** This section records the real state, not the target state.

**One: inference has been executed** (2026-08-17, local Ollama 0.32.14 loading `deepseek-r1:7b`,
roughly 40 seconds per record on CPU, about 11 minutes for 16). Measured facts:

| Fact                      | Value                                             |
| ------------------------- | ------------------------------------------------- |
| `generation_executed`     | `true` (`not_executed_reason` is `null`)          |
| Model / licence / runtime | `deepseek-r1:7b` / `MIT` / `ollama`               |
| Records                   | 16, with 0 not-generated markers                  |
| Body length               | 187 – 506 characters                              |
| `coverage_selector`       | `poem_tag` (the preferred path, not the fallback) |
| Template version          | `1.0.0`                                           |

The `--endpoint` URL **must carry a trailing slash** (`http://127.0.0.1:11434/`). Without it the
request fails at the network layer, and both `/v1` and `/v1/` return HTTP 404 — all three read like a
runtime that never started, when the actual cause is path joining.

**Two: the tamper-evidence gates remain, and they block both directions.** When inference does not
run, every body carries the explicit marker
`NOT_GENERATED_MARKER = "<<未生成：本条不是模型输出，需开放权重模型推理>>"`, the manifest records
`generation_executed = false` with a reason, and the final terminal line prints a NOT EXECUTED notice.
Conversely, `PregeneratedDataset::push` also rejects a record that declares execution while carrying
the marker. So "looking as though it really ran" cannot get through structurally — and this machinery
does not lapse now that a real run has happened; it guards the next one.

**Three: the thinking-block residue of reasoning weights is absorbed at the generation entry point.**
After the runtime strips the thinking block of a `deepseek-r1`-class model, a leading blank line is
left behind; measured, 16 of 16 bodies began with `\n\n`. `Generator::appreciate` therefore trims
before accepting: the criterion matches its own empty-body check — if "empty after trimming" counts as
having no content, then what trimming removes is not content. Without this, every shipped row would
carry leading whitespace, visible at the top of the appreciation panel on the detail page.

**Four: `dataset/appreciations.json` together with its digest, manifest and `.work/` are all in
`.gitignore`** (rules including `/dataset/appreciations.json`); only the hand-maintained
[`dataset/README.md`](../../dataset/README.md) is tracked. **This trade-off is unrelated to whether the
dataset has been generated**: it is the versioning policy for two independent release artifacts — the
dataset ships with a `corpus-v*` release, not with the source tree.

**Five: coverage is 16 poems.** The preferred path `select_by_poem_tag` matches on the current
`corpus/build/release/corpus.db`, so `coverage_selector` is recorded as `poem_tag`. The target is the
two standard 300-poem anthologies plus anthology tags — roughly a few thousand; the present artifact's
`poem_tag` only reaches these 16. Widening coverage requires re-running
`cargo run -p xtask -- corpus-build` (roughly 50 minutes at Tang–Song scale) so more tags land in the
artifact, **with no code change**.

## Labelling duty and the accuracy disclaimer

**AI appreciation text is AI-generated and is not scholarship.** Three duties:

1. **Visual separation.** The UI presents AI appreciation and cited commentary in **visually distinct
   registers**, never at the same visual level. Typesetting uncited generated prose to look like a
   quotation carrying a volume and page number is itself a form of misrepresentation.
2. **An "unreviewed" disclosure.** Every record in the shipped dataset has `reviewed == false`, and
   `dataset/README.md` discloses the fabrication and misattribution risks item by item.
3. **AI-generated poems are always labelled and never stored.** The user-facing label constant is
   `GENERATED_POEM_LABEL = "AI 生成，非古人作品"`; the MCP appreciation tool's annotation is
   `title = "AI 赏析"`.

**"Never stored" is enforced by a test rather than by convention.**
`generated_seven_character_quatrain_is_labelled_rhymed_and_never_persisted` saves `corpus_before` and
`cache_before` before generating, then asserts that **the corpus row count and both cache tier counts
are completely unchanged**, and asserts `payload["label"] == "AI 生成，非古人作品"`.

**The accuracy disclaimer, stated with its actual shape:** an AI appreciation may invent an allusion,
misattribute an author, misread a line, or transplant one poem's background onto another. It has no
citation to check against, so **it must not be quoted as fact**. This is not boilerplate — the entire
corpus position of this project is that analytical content which cannot be verified does not enter the
database, and the only reason AI appreciation can exist at all is that it is labelled as AI-generated,
presented separately from verifiable commentary, and never written into the corpus.

The crate-level documentation of `yunjian-ai` states this as a contract: all generated content is
labelled as AI output and never enters the corpus tables.

## What does not exist yet (recorded honestly)

- **Coverage is only 16 poems** against a target of a few thousand. The present corpus artifact's
  `poem_tag` only reaches these; the target coverage requires re-running `corpus-build` and then
  pregenerate (previous section, item five).
- **On a physical mobile device the shipped appreciation is still the placeholder marker.** This is not
  a code gap: mobile first run fetches the seed from
  `releases/latest/download/assets_manifest.json`, and the `appreciations.json` on the live
  `corpus-v0.1.0` release is still the 16-record placeholder build of 2026-08-15 (verified by
  downloading it: 16 of 16 are placeholders). With `YUNJIAN_ASSETS_MANIFEST` pointed at the new local
  seed, the desktop renders the real appreciation (the on-device assertion
  `shipped_appreciation_without_key` passes, with a 242-character body), so **what is missing is one
  release carrying the new seed, not a line of code**. The mobile criterion now also requires that the
  body exclude the not-generated marker, so this gap shows up in the device report as a visible FAIL
  rather than a silent PASS.
- **Eviction is FIFO rather than LRU**, and the index name `appreciation_cache_lru` disagrees with the
  behaviour (previous section).
- **Two closed providers' output terms are unverified**: OpenAI's partially (403), DeepSeek's entirely.
  **Nothing is blocked today** — the shipped dataset uses open weights only, and output from a user's
  key is never published — but somebody should actually read both before changing that posture.
- **There is no persistence promise on Linux.** Keyutils is lost on reboot, as above; this is not a
  pending defect but the nature of that tier, and the product's response is to state it plainly and ask
  again when the key is absent.

## Related documents

- [Corpus and indexing](CORPUS.md) — the shape of the copyright wall, and why AI appreciation is necessary rather than decorative
- [Architecture](ARCHITECTURE.md) — the long-operation event, progress and cancellation protocol
- [Third-party licences](../../LICENSES.md) — per-asset licence and attribution
