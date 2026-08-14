/**
 * 非 Tauri 宿主下的样例端口。
 *
 * # 每一条样例都自报「我不是语料」
 *
 * 这份数据只在 `vite dev`、Vitest 与 Playwright 里出现——那三处都拿不到语料库文件。
 * 但「拿不到数据就编一点」在本项目里是有代价的：一张 dev 截图会被当成产品行为，
 * 而里面的归属与出处若看起来像真的，就等于用界面做了一次没有依据的考据主张。
 *
 * 所以三条硬约束：
 *
 * 1. **`provenance.source` 一律是 `样例数据（非语料）`**，且它在异说列表里逐行显示——
 *    用户看向出处的那一眼就会看到这句话。
 * 2. **集评的 `work` 一律带「样例」字样**，不冒用任何真实诗话的名字。
 *    编一个《沧浪诗话》卷几页几出来，正是出处标准要禁止的那种不可复核引用。
 * 3. **正文只用毫无争议的公有领域名篇**，不自己写诗充数。
 *
 * 界面上另有一条常驻横幅（`App.tsx`）说明当前是样例模式。
 */

import type { AppreciationState } from "../contracts/ai";
import type {
  Attribution,
  CommentaryEntry,
  DictionaryCharacter,
  DictionaryLookup,
  DynastyLabel,
  MetaHit,
  MetaPage,
  PoemDetail,
  PoemRecord,
  Provenance,
  RhymeGroupMembership,
  SearchPage,
  TagSummary,
  TextSearchHit,
  ToneAnnotation,
  ToneCell,
} from "../contracts/core";
import type {
  AppreciationPort,
  DictionaryLookupRequest,
  DictionaryPort,
  PoemDetailRequest,
  PoemPort,
  SearchPort,
  TagBrowseRequest,
  TextSearchRequest,
} from "./ports";

/** 样例模式的横幅文案，由 `App.tsx` 显示。 */
export const SAMPLE_MODE_NOTICE =
  "当前为样例模式：没有连接到语料库，下面的作品、归属与集评都是用于界面调试的样例，不是语料内容。";

const SAMPLE_SOURCE = "样例数据（非语料）";

function dynasty(raw: string): DynastyLabel {
  return { canonical: raw, raw };
}

function provenance(locator: string): Provenance {
  return {
    source_locator: locator,
    source_locator_kind: "sample",
    source: SAMPLE_SOURCE,
    revision: "sample",
    kind: "sample",
    license: "public_domain",
    license_class: "public_domain",
  };
}

interface SamplePoem {
  record: PoemRecord;
  rhymeGroups: RhymeGroupMembership[];
  commentaries: CommentaryEntry[];
  appreciation: AppreciationState;
}

function record(
  stableId: string,
  title: string,
  author: string,
  dynastyRaw: string,
  genre: string,
  lines: string[],
  workGroup: string,
): PoemRecord {
  const body = lines.join("\n");
  return {
    stable_id: stableId,
    content_hash: `sample-${stableId}`,
    title,
    title_raw: title,
    ci_tune: null,
    author,
    dynasty: dynasty(dynastyRaw),
    genre,
    body,
    body_original: body,
    script: "simplified",
    first_line: lines[0] ?? "",
    last_chars: lines.map((line) => Array.from(line).at(-2) ?? ""),
    line_count: lines.length,
    char_count: Array.from(body.replace(/\n/g, "")).length,
    work_group: workGroup,
    edition_group: `${workGroup}-${author}`,
  };
}

/**
 * 平仄标注的样例。
 *
 * 刻意让一个字落在 `unknown`：详情页对未知位置显示「？」而不是留空这条行为，
 * 只有存在未知位置时才看得见。
 */
function tones(lines: string[]): ToneAnnotation {
  const pattern: ToneCell["tone"][] = ["oblique", "oblique", "level", "level", "unknown"];
  let unknown = 0;
  const annotated = lines.map((text, lineIndex) => {
    const cells: ToneCell[] = Array.from(text).map((character, index) => {
      const tone = pattern[(index + lineIndex) % pattern.length] ?? "unknown";
      if (tone === "unknown") {
        unknown += 1;
      }
      return { character, tone, readings: [] };
    });
    return { line_index: lineIndex, text, cells };
  });
  return { book: "pingshui", lines: annotated, unknown_count: unknown, either_count: 0 };
}

const JING_YE_SI_LINES = ["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"];
const DENG_GUAN_QUE_LOU_LINES = ["白日依山尽", "黄河入海流", "欲穷千里目", "更上一层楼"];
const CHUN_XIAO_LINES = ["春眠不觉晓", "处处闻啼鸟", "夜来风雨声", "花落知多少"];

/** 有出处的样例集评。`work` 带「样例」字样，不冒用真实诗话名。 */
function sampleCommentary(id: string, text: string): CommentaryEntry {
  return {
    id,
    text,
    citation: {
      work: "样例诗话",
      author: "样例评者",
      dynasty: dynasty("宋"),
      work_completed_by: 1200,
      source_note: "卷一，据样例排印本",
    },
  };
}

/**
 * 缺出处的样例集评。
 *
 * `source_note` 是空白串而不是缺字段：**空而存在的引用与缺失的引用同罪**，
 * 而前者才是真正会溜过去的那一种。详情页必须把它渲染成错误态而不是空文本。
 */
const BROKEN_COMMENTARY: CommentaryEntry = {
  id: "sample-commentary-broken",
  text: "这条评语在样例里刻意缺出处，用来验证界面会拒绝展示它。",
  citation: {
    work: "样例诗话",
    author: "样例评者",
    dynasty: dynasty("宋"),
    work_completed_by: 1200,
    source_note: "   ",
  },
};

const SAMPLE_APPRECIATION_TEXT = [
  "这是一段样例赏析文本，用于验证 AI 赏析面板的排版与标注。它不是任何模型的真实输出，也不是考据结论。",
  "面板的边框、底色、字体与左侧标尺都与上方考据材料刻意不同，正是为了让这段文字不可能被误认为有出处的评语。",
].join("\n");

const POEMS: SamplePoem[] = [
  {
    record: record(
      "sample-jingyesi",
      "静夜思",
      "李白",
      "唐",
      "shi",
      JING_YE_SI_LINES,
      "sample-wg-jingyesi",
    ),
    rhymeGroups: [
      { book: "pingshui", group: "阳", tone: "level", confidence: "unambiguous" },
      { book: "cilin", group: "第二部", tone: "level", confidence: "resolved_by_vote" },
    ],
    commentaries: [
      sampleCommentary("sample-commentary-1", "五言写乡思，语极浅而意极远，样例条目。"),
      BROKEN_COMMENTARY,
    ],
    appreciation: {
      kind: "ready",
      view: {
        text: SAMPLE_APPRECIATION_TEXT,
        model: "sample-open-weight-7b",
        template_version: "1.0.0",
        source: "shipped",
      },
    },
  },
  {
    record: record(
      "sample-dengguanquelou-a",
      "登鹳雀楼",
      "王之涣",
      "唐",
      "shi",
      DENG_GUAN_QUE_LOU_LINES,
      "sample-wg-dengguanquelou",
    ),
    rhymeGroups: [{ book: "pingshui", group: "尤", tone: "level", confidence: "unambiguous" }],
    commentaries: [sampleCommentary("sample-commentary-2", "四句两联皆对，气象阔大，样例条目。")],
    appreciation: {
      kind: "configuration_required",
      settings_path: "云笺 → 设置 → AI 服务商与密钥",
    },
  },
  {
    // 与上一条**同一正文、不同作者**：这就是异说。折叠与「另见」全靠这一对样例才看得见。
    record: record(
      "sample-dengguanquelou-b",
      "登鹳雀楼",
      "朱斌",
      "唐",
      "shi",
      DENG_GUAN_QUE_LOU_LINES,
      "sample-wg-dengguanquelou",
    ),
    rhymeGroups: [{ book: "pingshui", group: "尤", tone: "level", confidence: "unambiguous" }],
    commentaries: [],
    appreciation: { kind: "absent" },
  },
  {
    record: record(
      "sample-chunxiao",
      "春晓",
      "孟浩然",
      "唐",
      "shi",
      CHUN_XIAO_LINES,
      "sample-wg-chunxiao",
    ),
    rhymeGroups: [{ book: "pingshui", group: "萧", tone: "level", confidence: "unresolved" }],
    commentaries: [sampleCommentary("sample-commentary-3", "不着一「愁」字而惜春意足，样例条目。")],
    appreciation: {
      kind: "failed",
      message: "样例模式下不连接任何 AI 服务商，因此这一条显示为失败态。",
    },
  },
];

const TAGS: TagSummary[] = [
  { name: "思乡", poem_count: 1 },
  { name: "登临", poem_count: 2 },
  { name: "春景", poem_count: 1 },
];

const TAG_MEMBERS: Record<string, string[]> = {
  思乡: ["sample-jingyesi"],
  登临: ["sample-dengguanquelou-a", "sample-dengguanquelou-b"],
  春景: ["sample-chunxiao"],
};

function find(poemId: string): SamplePoem | undefined {
  return POEMS.find((poem) => poem.record.stable_id === poemId);
}

function textHit(poem: SamplePoem, query: string): TextSearchHit | null {
  const lines = poem.record.body.split("\n");
  for (const [index, line] of lines.entries()) {
    const at = Array.from(line).join("").indexOf(query);
    if (at < 0) {
      continue;
    }
    // 高亮下标按码点算，与 core 的约定一致。
    const start = Array.from(line.slice(0, at)).length;
    return {
      poem_id: poem.record.stable_id,
      title: poem.record.title,
      author: poem.record.author,
      dynasty: poem.record.dynasty.canonical,
      matched_line_index: index,
      snippet: {
        text: line,
        highlights: [{ start, end: start + Array.from(query).length }],
      },
    };
  }
  return null;
}

function metaHit(poem: SamplePoem): MetaHit {
  return {
    stable_id: poem.record.stable_id,
    title: poem.record.title,
    title_raw: poem.record.title_raw,
    ci_tune: poem.record.ci_tune,
    author: poem.record.author,
    dynasty: poem.record.dynasty,
    first_line: poem.record.first_line,
    work_group: poem.record.work_group,
    genre: poem.record.genre,
    line_count: poem.record.line_count,
    char_count: poem.record.char_count,
    matched_line_index: null,
  };
}

function siblings(poem: SamplePoem): Attribution[] {
  return POEMS.filter(
    (other) =>
      other.record.work_group === poem.record.work_group &&
      other.record.stable_id !== poem.record.stable_id,
  ).map((other) => ({
    stable_id: other.record.stable_id,
    author: other.record.author,
    dynasty: other.record.dynasty,
    title: other.record.title,
    source_locator: `sample:${other.record.stable_id}`,
    provenance_source: SAMPLE_SOURCE,
    provenance_revision: "sample",
  }));
}

function detail(poem: SamplePoem): PoemDetail {
  const rest = siblings(poem);
  const authors = new Set([poem.record.author, ...rest.map((entry) => entry.author)]);
  return {
    poem: poem.record,
    author: {
      name: poem.record.author,
      dynasties: [poem.record.dynasty],
      poem_count: 1,
    },
    tones: tones(poem.record.body.split("\n")),
    rhyme_groups: poem.rhymeGroups,
    work_group_siblings: rest,
    attribution_conflict:
      authors.size > 1
        ? {
            work_group: poem.record.work_group,
            attributions: [
              {
                stable_id: poem.record.stable_id,
                author: poem.record.author,
                dynasty: poem.record.dynasty,
                title: poem.record.title,
                source_locator: `sample:${poem.record.stable_id}`,
                provenance_source: SAMPLE_SOURCE,
                provenance_revision: "sample",
              },
              ...rest,
            ],
          }
        : null,
    provenance: provenance(`sample:${poem.record.stable_id}`),
    tags: Object.entries(TAG_MEMBERS)
      .filter(([, members]) => members.includes(poem.record.stable_id))
      .map(([name]) => name),
    commentaries: poem.commentaries,
  };
}

/** 样例端口。所有方法都是 `async`，与 Tauri 那侧的形状一致。 */
export function createSamplePorts(): {
  search: SearchPort;
  poem: PoemPort;
  dictionary: DictionaryPort;
  appreciation: AppreciationPort;
} {
  const search: SearchPort = {
    searchText: (request: TextSearchRequest): Promise<SearchPage> => {
      const query = request.query.trim();
      const hits =
        query === ""
          ? []
          : POEMS.map((poem) => textHit(poem, query)).filter(
              (hit): hit is TextSearchHit => hit !== null,
            );
      return Promise.resolve({
        hits: hits.slice(0, request.limit),
        total_estimate: hits.length,
        next_cursor: null,
      });
    },
    browseByTag: (request: TagBrowseRequest): Promise<MetaPage> => {
      const members = TAG_MEMBERS[request.tag] ?? [];
      const hits = POEMS.filter((poem) => members.includes(poem.record.stable_id)).map(metaHit);
      return Promise.resolve({ hits, next_cursor: null, normalized: request.tag });
    },
    listTags: () => Promise.resolve(TAGS),
  };

  const poem: PoemPort = {
    poemDetail: (request: PoemDetailRequest): Promise<PoemDetail> => {
      const found = find(request.poem_id);
      if (found === undefined) {
        return Promise.reject(new Error(`样例数据里没有 ${request.poem_id}`));
      }
      return Promise.resolve(detail(found));
    },
  };

  const appreciation: AppreciationPort = {
    appreciate: (request: PoemDetailRequest): Promise<AppreciationState> => {
      const found = find(request.poem_id);
      return Promise.resolve(found?.appreciation ?? { kind: "absent" });
    },
  };

  const dictionary: DictionaryPort = {
    lookupDictionary: (request: DictionaryLookupRequest): Promise<DictionaryLookup> => {
      const query = Array.from(request.query)
        .filter((character) => !/[\s，。、；：？！,.!?]/u.test(character))
        .join("");
      if (Array.from(query).length < 1 || Array.from(query).length > 2) {
        return Promise.reject(new Error("内置字典只接受一字或双字查询"));
      }
      const entries: Record<string, DictionaryCharacter> = {
        斜: {
          character: "斜",
          normalized: "斜",
          variants: [],
          pronunciation: { kind: "attested", reading: "xiá" },
          poyin: {
            reading: "xiá",
            confidence: "rhyme_attested",
            evidence: "《平水韵》下平声部 六麻；样例数据，仅用于界面调试",
            source_locator: "sample:data/poyin.tsv:斜",
          },
          rhymes: [
            {
              book: "pingshui",
              rhyme_group: "六麻",
              tone: "level",
              tone_raw: "下平声部",
              source_locator: "sample:corpus.db:rhyme:pingshui:六麻:斜",
            },
            {
              book: "cilin",
              rhyme_group: "第三部",
              tone: "level",
              tone_raw: "平声",
              source_locator: "sample:corpus.db:rhyme:cilin:第三部:斜",
            },
          ],
        },
        阳: {
          character: "阳",
          normalized: "阳",
          variants: [],
          pronunciation: { kind: "general", reading: "yáng" },
          poyin: null,
          rhymes: [
            {
              book: "pingshui",
              rhyme_group: "七阳",
              tone: "level",
              tone_raw: "下平声部",
              source_locator: "sample:corpus.db:rhyme:pingshui:七阳:阳",
            },
          ],
        },
      };
      const characters = Array.from(query).map(
        (character): DictionaryCharacter =>
          entries[character] ?? {
            character,
            normalized: character,
            variants: [],
            pronunciation: { kind: "unavailable" },
            poyin: null,
            rhymes: [],
          },
      );
      return Promise.resolve({
        query,
        kind: characters.length === 1 ? "character" : "character_sequence",
        characters,
      });
    },
  };

  return { search, poem, dictionary, appreciation };
}
