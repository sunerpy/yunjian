[简体中文](../DESIGN-LEARNING.zh.md) · English

# Learning, Dictionary and Pinyin Design

This document defines Yunjian's memorisation and review method, built-in dictionary, and pinyin
controls. It is a pre-implementation design: it does not claim that these interfaces and data
structures already exist. The design preserves the existing FSRS scheduler and the honesty boundary
around voice. It focuses on three problems: how to divide long poems into learnable units, how to keep
daily work from being overwhelmed by backlog, and how to help with rare characters, contextual
readings and polyphones without making claims beyond the evidence.

## Contents

- [Decisions at a glance](#decisions-at-a-glance)
- [Current evidence and boundaries](#current-evidence-and-boundaries)
- [Learning objects and mastery](#learning-objects-and-mastery)
- [Default learning path](#default-learning-path)
- [FSRS and same-day relearning](#fsrs-and-same-day-relearning)
- [Daily budget, backlog and visible pressure](#daily-budget-backlog-and-visible-pressure)
- [Cold-start difficulty](#cold-start-difficulty)
- [Typed scoring and order errors](#typed-scoring-and-order-errors)
- [Chanted recitation and the voice boundary](#chanted-recitation-and-the-voice-boundary)
- [Built-in dictionary](#built-in-dictionary)
- [Pinyin and tone-pattern controls](#pinyin-and-tone-pattern-controls)
- [Data contracts and caching](#data-contracts-and-caching)
- [Metrics, experiments and acceptance](#metrics-experiments-and-acceptance)
- [Delivery phases](#delivery-phases)

## Decisions at a glance

1. **Keep FSRS.** Yunjian continues to use the existing FSRS-6 parameters, four-grade assessment and
   history keyed by stable IDs. The new design addresses card division, same-day relearning, daily
   budgets and backlog management outside FSRS.
2. **Do not schedule a long poem as one card.** The whole poem is an aggregate, a paired-line chunk is
   the FSRS card, and a prosodic foot is the demonstration and practice unit. A short poem of no more
   than four lines and 32 content characters may collapse to one whole-poem chunk.
3. **Chanted recitation is the default path.** The sequence is foot-level demonstration, karaoke-style
   repetition, then active recall. Voice reports only speaking activity, pauses and relative rhythm; it
   never judges character accuracy or selects an FSRS grade.
4. **Clear due work before introducing new cards.** The daily plan packs work into a time budget. When
   due work exceeds the budget, new cards pause, while backlog size, expected duration and seven-day
   pressure remain directly visible rather than being hidden behind “today complete”.
5. **Dictionary v1 is a rhyme-book facts panel, not a modern dictionary.** It exposes only the shipped
   `rhyme` and `variant_map` data plus contextual readings with located evidence. Modern-dictionary
   definitions are permanently excluded, with no future ingestion opening left in the design.
6. **Pinyin and tone pattern are independent controls.** Evidence-backed contextual readings take
   priority. When the reading cannot be resolved, candidates are shown side by side with “reading
   uncertain here”; a modern pinyin reading is never guessed from a rhyme group.

## Current evidence and boundaries

### Why this is not another memory algorithm

FSRS and the DHP family used by MoMo share the same lineage. Existing comparison material does not
show MMX-6 to be more efficient online; it instead observes worse active and completion retention. The
actual observed retention is also about 75%, below the operational target of 85%. The immediate
problems are therefore not that the formula is insufficiently new, but how much work users receive,
how much they owe, whether long poems become prohibitively large cards, and whether they can recover
on the same day after forgetting. This design does not recommend replacing FSRS.

The current implementation also constrains compatibility. On a first review, `Scheduler::review_at`
passes `None` to FSRS and then stores `stability`, `difficulty`, `due_day` and the four-grade result.
There is no interface for writing objective features directly into initial `difficulty`. Cold-start
features may therefore rank new cards, estimate duration and warn about risk; they must not masquerade
as FSRS memory difficulty.

### Existing capabilities

- The scheduler uses `Again` / `Hard` / `Good` / `Easy`, with history keyed by a work's stable ID;
  changing a content digest does not erase that history.
- The typed path has deterministic normalisation, five `AlignOp` classes, and strict and lenient
  accuracy. Phonetic similarity relaxes substitutions only, never deletions or insertions.
- The voice path already has `FootMark` timestamps, per-line sessions and three rhythm feedback fields,
  and waits for the user to select a grade at the end.
- The `pinyin` dependency can return every candidate reading for a character. `Poyin` can find a
  specific contextual override by character and line context.
- The corpus has `rhyme`, `variant_map`, `tag` and `poem_tag` tables, but `tag` and `poem_tag` currently
  contain zero rows in the distributed artifact.

### Non-goals

- Do not design automatic voice scoring, and do not derive character accuracy, completeness or an FSRS
  grade from an ASR transcript.
- Do not replace FSRS, and do not write cold-start heuristics into FSRS `difficulty`.
- Do not include or transcribe definitions from any modern dictionary. Do not design a provider slot
  through which one could be connected later.
- Do not use AI-generated images. If AI-generated textual explanations appear elsewhere in the product,
  they must not enter dictionary primary data and must live in a separate area labelled with the model
  and “not human-reviewed”.
- Do not claim tags work when there is no data, and do not invent how many of the 1,815 rhyme-book
  candidates can be uniquely resolved.

## Learning objects and mastery

### Three object levels

| Level         | Purpose                                                           | Scheduled by FSRS | Stable identity                               |
| ------------- | ----------------------------------------------------------------- | ----------------- | --------------------------------------------- |
| Whole poem    | Overall progress, complete-recall check and statistical aggregate | No                | `poem.stable_id`                              |
| Chunk         | Independent recall, grading and due scheduling                    | Yes               | `poem_id + segmentation_version + line_range` |
| Prosodic foot | Demonstration playback, karaoke highlighting and local repetition | No                | `chunk_id + foot_index`                       |

A chunk is based on the corpus's metrical lines: two adjacent lines form one chunk; if one line remains
at the end, it joins the preceding chunk. A work of no more than four lines and 32 content characters
forms one chunk, avoiding a mechanical split of a quatrain. Ci, qu and irregular forms still use the
existing metrical-line segmentation rather than visual wrapping. The segmentation algorithm must be
versioned; rebuilding the same version must produce byte-stable chunk identities and boundaries.

The whole poem is not scheduled directly: otherwise one failure on a long poem creates both an
expensive due item and an ambiguous grade. Prosodic feet are not scheduled either, because they are
action scaffolds and should not generate dozens of tiny cards. Only a chunk carries an independently
answerable memory judgement.

### Mastery must not be flattered by an average

The work page shows all of the following:

- `Established`: chunks that have completed a first independent recall / total chunks;
- `Due`: chunks currently due or overdue;
- `Weak points`: chunks most recently graded `Again` or `Hard`;
- `Complete recitation`: the latest whole-poem check without textual prompts, available after every
  chunk has been established.

A whole poem is “currently secure” only when **every chunk is established and none is overdue**. An
average across chunks cannot cancel out one entirely unknown chunk. A whole-poem check is an aggregate
event: it creates no second set of FSRS cards and changes no chunk due date. The user still confirms a
grade per chunk, so a single whole-poem impression cannot overwrite granular history.

## Default learning path

Every new chunk follows the same fading scaffold. A user may jump ahead to active recall, but the
default does not skip understanding and rhythm:

1. **Read and inspect.** Show the source text; pinyin, tone pattern and dictionary are available on
   demand. Uncertain readings are exposed before playback rather than silently guessed during it.
2. **Foot-level demonstration.** Play and highlight by `FootMark`. Without voice capability, retain
   visual grouping and manual progression.
3. **Karaoke-style repetition.** Highlighting follows the demonstration. Playback and recording do not
   overlap. After the user repeats, feedback is limited to activity, pauses and relative rhythm.
4. **First-character prompts.** Retain only the first character of each line and ask for the chunk.
5. **Masked recall.** Use the existing `Cloze` / `FirstChar` / `Masked` exercises to remove prompts
   progressively.
6. **Active recall.** Complete a typed or self-reported recitation with no source-text prompt. Only this
   attempt produces a formal FSRS grade.

Opening the dictionary, showing pinyin, and replaying a demonstration are learning scaffolds. They are
not failures and do not directly lower the grade. The system may record them for feature validation,
but the formal grade comes only from typed mapping or user choice.

## FSRS and same-day relearning

### Submit one formal review only

The first independent answer to a due chunk is the formal review and immediately submits one of
`Again` / `Hard` / `Good` / `Easy` to the existing scheduler. The voice path always uses the user's
choice. The typed path may suggest the existing deterministic mapping, which the user may change.

An `Again` or `Hard` result also enters a **same-day relearning queue**:

- the first repetition appears 10 minutes after the formal review;
- after passing, one more repetition appears one hour later;
- another failure schedules 10 minutes from that failure, until the user passes or the day ends;
- an item still incomplete across the local day boundary remains visibly unfinished relearning and does
  not silently create another FSRS review record.

These 10-minute and one-hour attempts write practice events only. They **do not call
`Scheduler::review` again and do not advance `due_day`**. This enables prompt reconstruction after
forgetting without presenting dense same-day clicks as evidence of memory across multiple days. A new
chunk likewise submits only once after its first active recall; demonstration, repetition and masked
practice before it never feed FSRS.

### The shortest recovery path

Relearning does not return the user to the beginning of the poem. It first replays the failed chunk's
feet, then uses first-character prompts, then performs one unprompted recall. If only one foot was
wrong, practice may begin there, but passing still requires the entire chunk in the correct order.

## Daily budget, backlog and visible pressure

### Use a time budget, not a misleading fixed card count

First run suggests a 15-minute daily budget, which the user can change. Estimated duration per chunk
uses an exponential moving average of that user's actual duration for similar chunks. Before enough
samples exist, content-character count and whether the chunk is new provide the fallback. Estimation is
for packing and display only; it does not affect FSRS.

The daily queue packs the budget in this order:

1. same-day 10-minute and one-hour relearning;
2. overdue chunks, oldest overdue first;
3. chunks due today;
4. only then, if budget remains, new chunks up to the user's new-card limit.

If the first three categories fill the budget, the new-card count is zero. Due cards that did not fit
are not hidden from the plan: they appear as backlog. A user may explicitly add time, but the default
completion action promises only the planned work.

### Pressure information required on the home page

- Today's plan: card count and estimated minutes;
- Total due: planned / not packed into the plan;
- Overdue backlog: chunk count, oldest overdue age and estimated clearance time;
- Next seven days: estimated minutes aggregated from current FSRS due dates;
- Observed retention: the percentage of due reviews not graded `Again` over the last 30 days, with
  sample size;
- Target retention: the 85% operational target, shown alongside rather than in place of observation.

The current scheduler code uses `DESIRED_RETENTION = 0.9`, the model target used when FSRS calculates
intervals. The home page's 85% is an operational target for actual review outcomes. They require
different labels; a model parameter must not be presented as achieved retention. With insufficient
samples, show only “insufficient data (n=…)” rather than extrapolating a percentage.

## Cold-start difficulty

### Features estimate load; they do not seed FSRS

Every candidate feature is locally, offline and deterministically computable:

| Feature                         | Initial computation                                              | Validation before activation                                                             |
| ------------------------------- | ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Character count                 | Number of `content_chars`                                        | Bin against first-learning duration and first-week `Again` rate; test monotonicity       |
| Rare-character ratio            | Proportion below a whole-corpus frequency threshold              | Test whether it independently explains dictionary opens, duration and first-week failure |
| Contextual/polyphone candidates | Text intersection with `poyin.tsv` and `polyphone_index.tsv`     | Test relationship with uncertainty, character lookup and failure                         |
| Variant count                   | Number of text characters found in `variant_map`                 | Test whether it increases input correction, lookup and duration                          |
| Pattern regularity              | Dispersion of metrical-line lengths and number of derivable feet | At equal character count, test effect on duration and order errors                       |
| Allusion/topic tags             | Tags supplied by `poem_tag`                                      | The table is empty: **disabled** until corpus rebuild and coverage validation            |

The first release uses only character count as the duration fallback because its definition and
observation are the most direct. Other features are recorded but do not rank cards. Each candidate must
have a stable direction over a pre-frozen window and metric, and improve duration estimates on held-out
works, before being enabled independently. Do not invent weights merely to produce a “difficulty
score”.

`polyphone_index.tsv` physically has 1,843 lines, whose comments declare 1,815 Basic Multilingual Plane
data candidates. The number that can be uniquely resolved with context has not been computed. Until a
reproducible measurement exists, the interface, metrics and documentation must not report a unique
resolution rate.

## Typed scoring and order errors

The current `AlignOp::ReRecitation` means returning to a previously correct span and continuously
reciting it again. It does not fully represent swapping two lines, moving a later line earlier, or
reversing a block. “Re-recitation count” therefore cannot stand in for every order error.

Add a separate, deterministic order layer to the typed path **only; it never enters the voice path**:

1. Use the same metrical-line/foot segmentation as chunks to encode the reference as a stable unit
   sequence.
2. Match complete units in the answer and compute the longest common subsequence (LCS) of the reference
   sequence and matched sequence.
3. Define `ordered_coverage = LCS length / reference unit count`. A complete unit outside the LCS is a
   move, not a deletion.
4. Feed `min(existing completeness, ordered_coverage)` into the existing completeness threshold for a
   suggested grade. A detected move forbids suggesting `Easy`; strict character accuracy remains the
   original character-level value.

Before activation, golden cases must prove that correct order is not penalised, an adjacent-line swap
is detected, block reversal is more severe than one local re-recitation, and repeating a correct line
remains `ReRecitation` rather than a move. Then compare how often users override suggested grades in
anonymous local replay. If validation fails, show an order hint only and do not change the grade.

## Chanted recitation and the voice boundary

The basis for making chanted recitation the default path is the large difference in existing material:
100% long-term retention for sung verse versus 32% for spoken recitation. This supports putting rhythm
and melodic scaffolding at the learning entrance; it does not support machine judgement of a person's
pronunciation.

Implementation reuses the existing `FootMark { start_sample, end_sample }`: highlighting advances by
sample range during demonstration, and the user may repeat a selected foot. Recording must begin after
the demonstration stops; the microphone must not hear the loudspeaker and produce false feedback.

The voice contract continues to contain only:

- `guided_practice`: demonstration, repetition and feedback on activity, pauses and relative rhythm;
- `coverage_advisory`: coarse coverage that may become available only after an independent KWS
  experiment passes frozen gates.

Neither mode provides per-character pronunciation grades, missed-character lists or automatic FSRS
grades. The measured CER of classical-Chinese ASR is currently 77.01%, so it cannot serve as alignment
input for scoring. If voice, a model or permission is unavailable, the path degrades to visual feet,
manual rhythm and typed practice. Learning and review cannot be blocked by voice capability.

## Built-in dictionary

### v1 entry

Selecting a source-text character opens an in-place panel and shows available data in this order:

1. the glyph and any verifiable variant/normalised relationship in `variant_map`;
2. the current line's pinyin state: evidence-backed contextual reading, general candidates, or “reading
   uncertain here”;
3. `rhyme_book`, `tone_raw` and `rhyme_group` entries from 《平水韵》 and 《词林正韵》; show every
   entry for a character rather than selecting one on the user's behalf;
4. when `poyin.tsv` matches, its `confidence` and complete `evidence`;
5. the boundary statement: “Rhyme books record tone and rhyme groups. They are not modern definitions
   and cannot by themselves determine pinyin in this context.”

The `rhyme` table has only `rhyme_book`, `rhyme_group`, `tone`, `tone_raw` and `character`, with no
modern pinyin or definition field. v1 consumes only these existing facts, adding no new licensing risk
and making no claim to be a comprehensive Chinese dictionary.

### Permanent source rules

Modern-dictionary definitions are permanently excluded: they are neither bundled nor queried online,
and there is no provider extension point for adding them later. A future public-domain character book
may enter only as a new named asset in `corpus/sources.toml`, pass `verify-sources`, and give every
record a non-empty `source_locator` containing a volume, section or page locator. Entries with unknown
or unlocatable sources are rejected.

If AI text is used for a separate learning explanation, it must be partitioned from dictionary facts,
must not participate in search or reading decisions, and must not be written back into the corpus. Each
passage identifies the model and says “not human-reviewed”. This design neither needs nor permits
AI-generated images.

## Pinyin and tone-pattern controls

### Resolution precedence

When a whole poem loads, resolve every content character in one batch rather than making per-character
queries:

1. If the current line matches `Poyin::reading` with `rhyme_attested` or `tone_split` confidence, show
   that tone-marked pinyin and an evidence marker. A concrete context row takes precedence over `*`.
2. Without an override, obtain every candidate from the existing `ToPinyinMulti`. If there is exactly
   one candidate, show it as “general pinyin”, without claiming a classical-context decision.
3. With multiple candidates and no contextual evidence, show candidates side by side. Compact source
   text shows an uncertainty marker; selecting it reveals `xíng / háng / …` and “reading uncertain
   here”.
4. With no pinyin data, do not fabricate a placeholder reading; show “no annotation available”.

An `engine_default` row explicitly means no override and must not be promoted to an evidence-backed
reading. Multiple rhyme-book entries can explain why a reading is uncertain but cannot independently
produce modern pinyin. `data/poyin.tsv` physically has 117 lines and covers the 22 works in
`reading_roster.tsv`; no complete contextual-reading coverage may be claimed outside that roster. The
existing evidence-backed golden examples are 斜 `xiá`, 衰 `cuī` and 骑 `jì`.

### Layout rules

Pinyin and “tone/rhyme pattern” retain independent persisted controls; neither silently enables the
other:

- in horizontal layout, pinyin uses a ruby layer above the character, while tone pattern uses a small
  baseline-under marker or a line-end rhyme marker;
- in vertical layout, pinyin sits outside the character column, while tone pattern sits inside the cell
  or at line end; they must not compete for the same side;
- with both controls enabled, source line spacing remains readable, uncertainty follows the pinyin
  layer, and rhyme-foot marking follows the tone-pattern layer;
- unprompted active recall hides both layers by default. Explicitly revealing one records scaffold use
  but does not automatically mark an error.

Long poems are prefetched and resolved as a whole. Opening the panel reads the in-memory result.
Toggling either control must not trigger per-character database queries, network requests or audio-model
loading.

## Data contracts and caching

The design needs the following logical records. The names are contract drafts, not a claim that a
database migration already exists:

| Record              | Required fields                                                            | Purpose                                                |
| ------------------- | -------------------------------------------------------------------------- | ------------------------------------------------------ |
| `learning_chunk`    | `chunk_id`, `poem_id`, `segmentation_version`, `line_start`, `line_end`    | Stable chunk boundary                                  |
| `practice_event`    | `chunk_id`, `kind`, `started_at`, `duration_ms`, `scaffold_used`, `result` | Same-day practice, not an FSRS log                     |
| `whole_poem_check`  | `poem_id`, `checked_at`, `result`, `weak_chunk_ids`                        | Aggregate event, not scheduled                         |
| `annotation_bundle` | `poem_id`, `content_hash`, `lexicon_hash`, `items`                         | Batched whole-poem pinyin, uncertainty and rhyme facts |

Existing `review_state` / `review_log` continue to own FSRS state. Chunk IDs must be stable and have an
explicit parent relationship to the whole-poem ID. A segmentation-version change first produces a
migration report and must never silently discard history. Annotation cache keys include at least the
work content digest and contextual-reading lexicon digest; changing either invalidates and rebuilds the
whole work.

Local events serve local statistics by default. Any future sync or telemetry design must separately
specify data minimisation and user consent. This design sends no poem input, recording or learning
history to a third party.

## Metrics, experiments and acceptance

### Outcome metrics

- 30-day observed retention with sample size, target 85%, current baseline about 75%;
- daily plan completion and the number of misleading cases where backlog remains while the plan claims
  completion (target zero);
- overdue chunk count and estimated days to clear;
- time to establish a new chunk and first-week `Again` rate;
- whole-poem check pass rate and weakest-chunk distribution;
- user override rate for suggested grades, stratified by ordinary and order errors;
- pinyin-state coverage: evidence-backed contextual / single general candidate / multiple uncertain /
  absent, always with absolute counts and denominator.

### Required acceptance gates

1. **Scheduling conservation:** same-day relearning neither adds a `review_log` row nor changes FSRS
   `due_day`.
2. **Stable division:** the same corpus and segmentation version produce identical chunk IDs; short
   poems and an unmatched final line have golden tests.
3. **No hidden backlog:** under every budget, `planned + not packed = total due`.
4. **Order detection:** correct order, adjacent-line swap, block reversal, repetition and omission each
   hit the expected class; voice types cannot call this scoring path.
5. **Reading honesty:** the three golden contextual readings match; `engine_default` does not override;
   multiple candidates show uncertainty; `rhyme` alone cannot generate pinyin.
6. **Source gate:** a new character-book asset missing a licence, digest or locator fails the build; a
   modern-dictionary fixture is rejected.
7. **Performance:** a long-work detail performs one batched annotation/rhyme lookup; toggling controls
   adds no query.
8. **Accessibility:** colour is not the sole encoding for tone pattern, uncertainty or due state; ruby
   and vertical side annotations are read by assistive technology after their source character.

Every retention or efficiency experiment freezes its window, inclusion rules and primary metrics in
advance. “Fewer cards per day” is insufficient, because lower workload may trade away memory. Retention,
completion and backlog must be reported together.

## Delivery phases

### Phase 1: make workload controllable

- Implement deterministic chunk identity, whole-poem aggregates, the daily minute budget and backlog
  panel.
- Preserve the existing FSRS parameters and grading interface.
- Add 10-minute and one-hour practice queues without writing FSRS review logs.
- Use only character count as the cold-start duration fallback; merely record other features.

### Phase 2: complete the default learning path

- Connect `FootMark` to foot-level demonstration and karaoke highlighting.
- Complete first-character, masking and active-recall scaffold fading.
- Fully degrade voice failure to visual rhythm and typing, preserving the user-selected-grade boundary.
- Launch typed order hints; let them affect suggested grades only after golden tests and override-rate
  validation pass.

### Phase 3: ship the dictionary and two independent controls

- Batch-combine `Poyin`, `ToPinyinMulti`, `rhyme` and `variant_map`.
- Ship horizontal ruby, vertical side annotation and the independent tone-pattern layer.
- Show source, confidence and uncertainty; do not ship modern definitions.
- Measure and publish absolute coverage across the four pinyin states, still without presenting
  candidate count as unique resolution count.

### Phase 4: calibrate instead of replacing the algorithm

- Validate cold-start features individually against real local events.
- Compare budget and backlog strategies on 30-day retention, completion and activity together.
- Enable a feature only when evidence shows that it improves load estimation; do not use this as a
  reason to replace FSRS.
- Keep `tag` / `poem_tag` disabled until the corpus artifact has real coverage.
