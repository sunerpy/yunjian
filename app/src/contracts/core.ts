/**
 * `yunjian-core` 序列化形状的逐字镜像。
 *
 * # 为什么用 snake_case 而不是前端惯用的 camelCase
 *
 * 这些类型描述的是 **serde 输出的 JSON**，不是前端自己发明的视图模型。改名就需要一层映射，
 * 而映射层是最容易与后端悄悄漂移的地方——后端加一个字段，映射层不加，编译照样通过。
 * 保持 snake_case 的代价是与前端习惯不一致，收益是「字段名对不上」会在类型检查时就报出来。
 *
 * # 每条都核对过源码，没有一条是凭记忆写的
 *
 * 项目已在「凭记忆写标识符」上栽过六次（见 notepads/issues.md）。因此下面每个结构都标注了
 * 它在 Rust 侧的出处，并记下已实测的三处**反直觉之处**：
 *
 * 1. **`TextSearchHit` 没有 `work_group`。** 只有 `MetaHit`（题目/作者/朝代/标签检索）与
 *    `PoemRecord` 有。正文检索的命中因此**无法参与异说折叠**——这不是我们偷懒，是数据里没有。
 *    折叠层用 `null` 表示这件事，见 `data/rows.ts`。
 * 2. **`CommentaryEntry.citation` 不是 `Option`。** core 在 `poem_detail` 里遇到空引用会返回
 *    `Error::CommentaryCitationMissing` 而不是构造一条无出处的记录
 *    （`crates/yunjian-core/src/search/topic.rs:760-781`）。前端仍然要校验：类型上不可能
 *    不等于运行时不可能——IPC 那一侧是 JSON，任何形状都能过来。
 * 3. **`snippet.highlights` 的下标是 Unicode 字符下标，不是 UTF-16 码元下标**
 *    （`crates/yunjian-core/src/search/text.rs:32-40` 明确写了这一点）。
 *    JS 的 `slice` 按码元切，对 BMP 内的汉字两者相同，对增补平面（部分生僻字、异体字）会错位。
 *    因此高亮必须先 `Array.from` 拆成码点，见 `search/Highlight.tsx`。
 */

/** 朝代的规范键与上游原串。`crates/yunjian-core/src/search/meta.rs:62-71`。 */
export interface DynastyLabel {
  canonical: string;
  raw: string;
}

/**
 * 高亮范围，**Unicode 字符下标**，`start` 含、`end` 不含。
 *
 * `crates/yunjian-core/src/search/text.rs:32-40`。
 */
export interface HighlightRange {
  start: number;
  end: number;
}

/** 一段文本及其字符级高亮范围。`crates/yunjian-core/src/search/text.rs:42-48`。 */
export interface HighlightedSnippet {
  text: string;
  highlights: HighlightRange[];
}

/**
 * 一条正文检索命中。`crates/yunjian-core/src/search/text.rs:50-65`。
 *
 * **注意没有 `work_group`、没有 `genre`。** 正文检索走 FTS5 与 n-gram 候选表，
 * 返回的是命中句而不是作品元数据。
 */
export interface TextSearchHit {
  poem_id: string;
  title: string;
  author: string;
  /** 朝代规范键，是裸 `String` 而不是 `DynastyLabel`——与 `MetaHit` 不同。 */
  dynasty: string;
  matched_line_index: number;
  snippet: HighlightedSnippet;
}

/** 一页正文检索结果。`crates/yunjian-core/src/search/text.rs:67-78`。 */
export interface SearchPage {
  hits: TextSearchHit[];
  total_estimate: number;
  /** `null` 表示已到末页。入参那一侧叫 `cursor`，出参这一侧叫 `next_cursor`。 */
  next_cursor: string | null;
}

/**
 * 元数据检索命中。`crates/yunjian-core/src/search/meta.rs:103-131`。
 *
 * 与 `TextSearchHit` 的三处结构差异都已核实：`stable_id` 而非 `poem_id`、
 * `dynasty` 是 `DynastyLabel` 而非 `String`、**有 `work_group` 与 `genre`**。
 */
export interface MetaHit {
  stable_id: string;
  title: string;
  title_raw: string;
  ci_tune: string | null;
  author: string;
  dynasty: DynastyLabel;
  first_line: string;
  work_group: string;
  /** 运行期是裸 `String`，不是构建期的 `Genre` 枚举。 */
  genre: string;
  line_count: number;
  char_count: number;
  matched_line_index: number | null;
}

/** 一页元数据检索结果。`crates/yunjian-core/src/search/meta.rs:133-143`。 */
export interface MetaPage {
  hits: MetaHit[];
  next_cursor: string | null;
  normalized: string;
}

/** 一条归属。`crates/yunjian-core/src/search/meta.rs:156-171`。 */
export interface Attribution {
  stable_id: string;
  author: string;
  dynasty: DynastyLabel;
  title: string;
  source_locator: string;
  provenance_source: string;
  provenance_revision: string;
}

/** 归属冲突：同一正文挂了多个作者。`crates/yunjian-core/src/search/meta.rs:173-185`。 */
export interface AttributionConflict {
  work_group: string;
  attributions: Attribution[];
}

/** 平仄。`crates/yunjian-core/src/search/topic.rs:68-79`，`rename_all = "snake_case"`。 */
export type Tone = "level" | "oblique" | "either" | "unknown";

/** 韵书。`crates/yunjian-core/src/rhyme.rs:29-38`，`rename_all = "snake_case"`。 */
export type RhymeBook = "pingshui" | "cilin" | "xinyun";

/** 韵部声调。`crates/yunjian-core/src/rhyme.rs:180-193`。 */
export type RhymeTone = "level" | "rising" | "departing" | "entering" | "oblique";

/** 韵部判定可信度。`crates/yunjian-core/src/rhyme.rs:124-133`。 */
export type RhymeConfidence = "resolved_by_vote" | "unambiguous" | "unresolved";

/** 单字的平仄格。`crates/yunjian-core/src/search/topic.rs:146-153`。 */
export interface ToneCell {
  character: string;
  tone: Tone;
  readings: string[];
}

/** 一行的平仄标注。`crates/yunjian-core/src/search/topic.rs:155-164`。 */
export interface ToneLine {
  line_index: number;
  text: string;
  cells: ToneCell[];
}

/**
 * 整首的平仄标注。`crates/yunjian-core/src/search/topic.rs:166-182`。
 *
 * **没有 `has_unknown` 字段**——那是 Rust 侧的方法，不参与序列化。想要就用 `unknown_count > 0`。
 */
export interface ToneAnnotation {
  book: RhymeBook;
  lines: ToneLine[];
  unknown_count: number;
  either_count: number;
}

/**
 * 有据破读的依据强度。`crates/yunjian-voice/src/annotate.rs` 的 `AttestedConfidence`，
 * `rename_all = "snake_case"`。
 *
 * **只有两个取值。** 破读词表里还有第三档 `engine_default`，它表示的恰恰是「不覆写，
 * 只登记处置」，因此在类型上到不了这里——那一档的字会落到 `generic` 或 `uncertain`。
 */
export type AttestedConfidence = "rhyme_attested" | "tone_split";

/**
 * 一个字的读音处境，四档互斥。`crates/yunjian-voice/src/annotate.rs` 的 `Reading`，
 * 内部标签是 `kind`。
 *
 * 四档必须在界面上说成四种不同的话：
 *
 * | `kind` | 含义 | 界面不能说成 |
 * | --- | --- | --- |
 * | `attested` | 破读词表按当前句给出的有据读音 | —— |
 * | `generic` | 通用候选只有一个 | 「古典语境裁决」 |
 * | `uncertain` | 多候选且无破读证据 | 任何一个唯一读音 |
 * | `absent` | 没有读音数据 | 一个占位读音 |
 */
export type Reading =
  | {
      kind: "attested";
      pinyin: string;
      confidence: AttestedConfidence;
      evidence: string;
    }
  | { kind: "generic"; pinyin: string }
  | { kind: "uncertain"; candidates: string[] }
  | { kind: "absent" };

/**
 * 正文里的一格。
 *
 * `reading` 为 `null` 表示这一格**不是内容字**（标点、空白），因此没有读音层。
 * 它与 `{ kind: "absent" }` 不是一回事：后者是「有读音位但查不到数据」。
 * 混同会让「，」也顶着一个「暂无注音」。
 */
export interface AnnotationCell {
  character: string;
  reading: Reading | null;
}

/** 一行的注音。`line_index` 与 `ToneLine.line_index` 同一口径：都按过滤空行后的下标。 */
export interface AnnotatedLine {
  line_index: number;
  text: string;
  cells: AnnotationCell[];
}

/**
 * 四档的绝对数量。
 *
 * 存计数而不是比例，因为比例会把「3 / 4」和「3000 / 4000」说成同一件事。
 */
export interface AnnotationCoverage {
  attested: number;
  generic: number;
  uncertain: number;
  absent: number;
}

/** 整首的注音结果。`poem_id` 原样回带，供界面确认这份注音属于它当前展示的那一首。 */
export interface PoemAnnotation {
  poem_id: string;
  lines: AnnotatedLine[];
  coverage: AnnotationCoverage;
}

/** 逐韵书的韵部归属。`crates/yunjian-core/src/search/topic.rs:207-218`。 */
export interface RhymeGroupMembership {
  book: RhymeBook;
  group: string;
  tone: RhymeTone;
  confidence: RhymeConfidence;
}

/** 溯源。`crates/yunjian-core/src/search/topic.rs:258-275`。全部字段都是裸 `String`。 */
export interface Provenance {
  source_locator: string;
  source_locator_kind: string;
  source: string;
  revision: string;
  kind: string;
  license: string;
  license_class: string;
}

/**
 * 一条集评的出处。`crates/yunjian-core/src/search/topic.rs:231-245`。
 *
 * 卷次、页码、所据版本**都在 `source_note` 这一个串里**，没有独立的 `volume` / `page` /
 * `locator` 字段。构建期校验要求它同时含卷次定位符与版本说明
 * （`crates/yunjian-corpus/src/commentary.rs:349-366`）。
 */
export interface CommentaryCitation {
  /** 所引著作，不带书名号——由展示层加。 */
  work: string;
  author: string;
  dynasty: DynastyLabel;
  /** 成书年的保守上界，必然早于 1912。 */
  work_completed_by: number;
  source_note: string;
}

/** 一条历代集评。`crates/yunjian-core/src/search/topic.rs:247-256`。 */
export interface CommentaryEntry {
  id: string;
  text: string;
  citation: CommentaryCitation;
}

/** 作品本体。`crates/yunjian-core/src/search/topic.rs:277-314`。 */
export interface PoemRecord {
  stable_id: string;
  content_hash: string;
  title: string;
  title_raw: string;
  ci_tune: string | null;
  author: string;
  dynasty: DynastyLabel;
  genre: string;
  /** 规范简体正文，行以换行分隔。 */
  body: string;
  /** 上游原字形正文。繁体来源在这里保留繁体。 */
  body_original: string;
  script: string;
  first_line: string;
  last_chars: string[];
  line_count: number;
  char_count: number;
  work_group: string;
  edition_group: string;
}

/** 作者记录。`crates/yunjian-core/src/search/topic.rs:321-329`。 */
export interface AuthorRecord {
  name: string;
  dynasties: DynastyLabel[];
  poem_count: number;
}

/**
 * 作品详情。`crates/yunjian-core/src/search/topic.rs:351-372`。
 *
 * 字段名是 `work_group_siblings`（core 侧）。**MCP 的对外契约把同一概念叫
 * `work_group_alternatives`**（`crates/yunjian-mcp/src/schema.rs:280`）——两个名字都是真的，
 * 取决于走哪条传输层。本前端走 core 契约。
 */
export interface PoemDetail {
  poem: PoemRecord;
  author: AuthorRecord;
  tones: ToneAnnotation;
  rhyme_groups: RhymeGroupMembership[];
  /** 同一 `work_group` 下的其它记录，**不含本篇**。 */
  work_group_siblings: Attribution[];
  /** 同一正文挂了多个不同作者时非 `null`。 */
  attribution_conflict: AttributionConflict | null;
  provenance: Provenance;
  tags: string[];
  commentaries: CommentaryEntry[];
}

/** 标签（主题）汇总。`crates/yunjian-core/src/search/topic.rs:331-338`。 */
export interface TagSummary {
  name: string;
  poem_count: number;
}

export type DictionaryQueryKind = "character" | "character_sequence";

export interface VariantRelation {
  variant: string;
  normalized: string;
}

export interface DictionaryRhymeFact {
  book: RhymeBook;
  rhyme_group: string;
  tone: RhymeTone;
  tone_raw: string;
  source_locator: string;
}

export type PoyinConfidence = "rhyme_attested" | "tone_split" | "engine_default";

export interface PoyinEvidence {
  reading: string | null;
  confidence: PoyinConfidence;
  evidence: string;
  source_locator: string;
}

export type DictionaryPronunciation =
  | { kind: "attested"; reading: string }
  | { kind: "general"; reading: string }
  | { kind: "uncertain"; candidates: string[] }
  | { kind: "unavailable" };

export interface DictionaryCharacter {
  character: string;
  normalized: string;
  variants: VariantRelation[];
  pronunciation: DictionaryPronunciation;
  poyin: PoyinEvidence | null;
  rhymes: DictionaryRhymeFact[];
}

export interface DictionaryLookup {
  query: string;
  kind: DictionaryQueryKind;
  characters: DictionaryCharacter[];
}
