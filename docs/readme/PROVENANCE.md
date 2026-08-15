# Content provenance and licensing

Two things live on this page: **why 云笺 has the architecture it has**, and **what licence each class
of shipped content falls under**. They are two sides of the same question — the architecture is a
consequence of copyright constraints, not an aesthetic preference.

## Why it is built this way

In one line: **the copyright wall is the architecture.**

Across the open poetry datasets that exist, every one carrying modern annotation, translation or
appreciation has a licence chain that does not hold up: scraped from commercial sites, marked
academic-use-only, or covered by a repository-level LICENSE that cannot possibly grant rights over
transcribed content. Verifying this in practice flagged 10 files carrying modern vernacular annotation
_inside_ a single MIT repository.

So 云笺 combines exactly three things:

1. **Public-domain source text** — pre-modern works, out of copyright;
2. **Pre-modern commentary with a per-entry citation** — a Song critic writing about a Tang poem is
   itself public domain;
3. **Clearly labelled AI appreciation** — generated with your own API key, always tagged as AI text.

The AI feature is not a nice-to-have. It is the only lawful way to fill the hole the copyright wall
leaves behind.

## The code

Licensed under [MIT](../../LICENSE).

## The shipped corpus

**Only public-domain source text plus MIT-licensed upstream compilation output.** Per-asset licence
verdicts live in [`corpus/sources.toml`](../../corpus/sources.toml) and are enforced by
`xtask verify-sources`; rejected datasets and the reason for each rejection are recorded in
[`corpus/DENYLIST.md`](../../corpus/DENYLIST.md).

Verification is per **file**, not per repository — a repository's MIT LICENSE cannot license content
it scraped.

## Appreciation text

**AI-generated, not scholarship.** The UI renders it in a different visual register from cited
commentary and discloses that it is unreviewed. AI-generated poems are labelled as such and never
enter the corpus or the appreciation tables.

**The pre-generated set ships from open-weight models only, never a closed API.** This is a licence
chain constraint rather than a performance preference: two of three closed-API output-redistribution
clauses could not be verified, while downloaded weights carry no such clause. `xtask pregenerate`
enforces it before writing a single record (MIT and Apache-2.0 weights only, local runtimes only);
per-record provenance and the full disclosure are in [`dataset/README.md`](../../dataset/README.md).

## The voice feature: two licence tiers

- The `voice` cargo feature is **off by default**. A default build is pure MIT and, as measured,
  links no onnxruntime at all.
- With `voice` enabled, the prebuilt sherpa-onnx artifact **statically contains GPL-3.0 espeak-ng**
  (50 exported `espeak_*` symbols, measured). MIT is one-way compatible with GPL-3.0, so this is not a
  conflict — but **a distributed voice-enabled build must be offered under GPL-3.0 as a whole.**
- Releases therefore come in two flavours: default builds labelled MIT, voice builds labelled
  GPL-3.0. Details in [Voice build](../VOICE-BUILD.zh.md).

## Model weights

No model weights are bundled. Voice models are downloaded on demand and only under a verified MIT or
Apache-2.0 licence — per-model verdicts, evidence and digests are in
[`models.toml`](../../models.toml), enforced by `xtask verify-models`; rejected models and the reason
for each are in [`models/DENYLIST.md`](../../models/DENYLIST.md).

Verification overturned an earlier judgement: **the FunASR family (SenseVoice / Paraformer) ships
under Alibaba's own licence agreement**, so Whisper is the only offline recognition family left.

## One conclusion that does not soften

**Character accuracy on the voice path is always advisory, never a score.** That is not cautious
phrasing but a measured result: see the [CER report](../reports/asr-cer.md).

Every bundled or downloaded third-party asset, its licence and its attribution are listed in
[Third-party licences](LICENSES.md).
