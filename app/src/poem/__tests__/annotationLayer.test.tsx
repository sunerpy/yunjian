/**
 * 拼音注音层：四档形态、两个开关的独立性、切换不查库、以及主动回忆下的默认隐藏。
 *
 * # 为什么这一组从详情页出发而不是只渲染 `OriginalText`
 *
 * 组件级用例证明组件对，不证明它可达。本仓库已经栽过一次：一组档位断言全绿，而它们硬写了
 * 中间参数，于是替被测代码回答了「它会用哪个值」这个问题，真实路径上的缺陷照旧漏过。
 * 所以下面凡是能从详情页走到的，一律从详情页走——注音数据经由端口取回、开关经由用户点击、
 * 渲染结果经由屏幕上的文字读出。
 *
 * # 期望值的来源
 *
 * 四档的判定属于 Rust 侧（`crates/yunjian-voice/src/annotate.rs`，那边有 15 条用例），
 * 这一组**不重算读音**，只喂给界面一份形状确定的注音，验证界面把四档说成四种不同的话。
 * 「不重算」是刻意的：在这里抄一遍判定逻辑，等于让两处实现互相背书而谁都没被验证。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AnnotatedLine,
  PoemAnnotation,
  PoemDetail,
  Reading,
  ToneAnnotation,
} from "../../contracts/core";
import type { AppreciationPort, PoemAnnotationRequest, PoemPort } from "../../data/ports";
import PoemDetailScreen from "../PoemDetailScreen";
import { ANNOTATION_PREFERENCE_KEYS } from "../annotationPreferences";

/** 两行，第二行带标点，用来验平仄格在标点位留空。 */
const BODY = "床前明月光\n举头望山，上";

const READINGS: Record<string, Reading> = {
  床: { kind: "generic", pinyin: "chuáng" },
  前: { kind: "generic", pinyin: "qián" },
  明: { kind: "generic", pinyin: "míng" },
  月: { kind: "generic", pinyin: "yuè" },
  光: { kind: "generic", pinyin: "guāng" },
  举: { kind: "generic", pinyin: "jǔ" },
  头: { kind: "uncertain", candidates: ["tóu", "tou"] },
  望: { kind: "absent" },
  山: {
    kind: "attested",
    pinyin: "shān",
    confidence: "rhyme_attested",
    evidence: "《平水韵》上平声部 十五删，据锁定版转录本。",
  },
  上: {
    kind: "attested",
    pinyin: "shàng",
    confidence: "tone_split",
    evidence: "《平水韵》去声部 二十三漾，据锁定版转录本。",
  },
};

function annotationFixture(poemId = "p-1"): PoemAnnotation {
  const coverage = { attested: 0, generic: 0, uncertain: 0, absent: 0 };
  const lines: AnnotatedLine[] = BODY.split("\n").map((text, line_index) => ({
    line_index,
    text,
    cells: Array.from(text).map((character) => {
      const reading = READINGS[character];
      if (reading === undefined) {
        return { character, reading: null };
      }
      coverage[reading.kind] += 1;
      return { character, reading };
    }),
  }));
  return { poem_id: poemId, lines, coverage };
}

function tonesFixture(): ToneAnnotation {
  const lines = BODY.split("\n").map((text, line_index) => ({
    line_index,
    text,
    // 与 core 同一判据：平仄格只覆盖内容字，标点不建格。
    cells: Array.from(text)
      .filter((character) => READINGS[character] !== undefined)
      .map((character) => ({ character, tone: "level" as const, readings: [] })),
  }));
  return { book: "pingshui", lines, unknown_count: 0, either_count: 0 };
}

function detailFixture(): PoemDetail {
  return {
    poem: {
      stable_id: "p-1",
      content_hash: "h",
      title: "样例",
      title_raw: "样例",
      ci_tune: null,
      author: "样例作者",
      dynasty: { canonical: "唐", raw: "唐" },
      genre: "shi",
      body: BODY,
      body_original: BODY,
      first_line: "床前明月光",
      last_char: "上",
      line_count: 2,
      char_count: 11,
      work_group: "g-1",
      source_locator: "样例卷一",
    },
    tones: tonesFixture(),
    rhyme_groups: [],
    provenance: {
      dataset: "样例",
      dataset_version: "1",
      license: "MIT",
      license_class: "public_domain",
    },
    tags: [],
    commentaries: [],
    work_group_siblings: [],
    attribution_conflict: null,
  } as unknown as PoemDetail;
}

type Ports = {
  poemPort: PoemPort;
  appreciationPort: AppreciationPort;
  detailCalls: () => number;
  annotationCalls: () => number;
};

/**
 * 端口替身。
 *
 * 两个方法各自带计数器，这是「切换开关不产生新查询」那条断言的唯一抓手：注音是不是被
 * 重新取了一次，只能从调用次数上看出来——界面上看起来是一样的。
 */
function ports(annotation: PoemAnnotation | Error = annotationFixture()): Ports {
  let detailCalls = 0;
  let annotationCalls = 0;
  const poemPort: PoemPort = {
    poemDetail: () => {
      detailCalls += 1;
      return Promise.resolve(detailFixture());
    },
    poemAnnotations: (request: PoemAnnotationRequest) => {
      annotationCalls += 1;
      return annotation instanceof Error
        ? Promise.reject(annotation)
        : Promise.resolve({ ...annotation, poem_id: request.poem_id });
    },
  };
  return {
    poemPort,
    appreciationPort: { appreciate: () => Promise.resolve({ kind: "absent" }) },
    detailCalls: () => detailCalls,
    annotationCalls: () => annotationCalls,
  };
}

function mount(given: Ports, extra: { recall?: boolean; onReveal?: () => void } = {}) {
  return render(
    <PoemDetailScreen
      poemId="p-1"
      poemPort={given.poemPort}
      appreciationPort={given.appreciationPort}
      onBack={vi.fn()}
      {...extra}
    />,
  );
}

async function settled() {
  await waitFor(() => {
    expect(screen.getByTestId("poem-detail")).toBeDefined();
  });
}

async function annotated(given: Ports) {
  await settled();
  await waitFor(() => {
    expect(given.annotationCalls()).toBeGreaterThan(0);
  });
  fireEvent.click(screen.getByTestId("pinyin-toggle"));
  await waitFor(() => {
    expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
  });
}

/**
 * 本环境**没有** `localStorage`，所以要自己装一个。
 *
 * 实测：`sessionStorage` 是对象，而 `localStorage` 与 `globalThis.localStorage` 都是
 * `undefined`（Node 自带的实验性同名全局要求 `--localstorage-file`，把 jsdom 那份挡掉了）。
 * 不装的话偏好读写会走进模块里那条「存储不可用就当关」的降级分支，四条持久化断言全都变成
 * 在验降级，而不是在验持久化——**一组永远绿而什么都没验的断言**。
 *
 * 装的是一个真正的 `Storage` 形状，被测模块照旧走 `getItem` / `setItem` 那条真实路径；
 * 「浏览器里真的能记住」由亲手 QA 里的刷新那一步负责，不由本文件冒充。
 */
function memoryStorage(): Storage {
  const entries = new Map<string, string>();
  return {
    get length() {
      return entries.size;
    },
    clear: () => entries.clear(),
    getItem: (key: string) => entries.get(key) ?? null,
    key: (index: number) => [...entries.keys()][index] ?? null,
    removeItem: (key: string) => {
      entries.delete(key);
    },
    setItem: (key: string, value: string) => {
      entries.set(key, value);
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("localStorage", memoryStorage());
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("四档读音各自说成不同的话", () => {
  it("有据破读显示带调拼音与依据标记，依据原文进 title", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    const attested = screen.getByTitle(/有据破读：《平水韵》上平声部 十五删/);
    expect(attested.textContent).toContain("shān");
    // 「据」是非颜色的有据标记：无障碍那条要求颜色不能是唯一编码。
    expect(attested.textContent).toContain("据");
  });

  it("单候选标为通用拼音，且不声称是古典语境裁决", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    const generic = screen.getAllByTitle("通用拼音，不是古典语境裁决");
    expect(generic.length).toBeGreaterThan(0);
    expect(generic.map((node) => node.textContent)).toContain("chuáng");
    for (const node of generic) {
      expect(node.textContent).not.toContain("据");
    }
  });

  it("多候选先只给一个存疑标记，点开才并列候选并写「此处读音存疑」", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    // 紧凑模式：候选不能一上来就摊在正文里，否则一个五读的字能把整行挤爆。
    expect(screen.queryByTestId("uncertain-detail")).toBeNull();

    const mark = screen.getByTestId("uncertain-mark");
    expect(mark.textContent).toBe("存疑");
    expect(mark.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(mark);
    const detail = screen.getByTestId("uncertain-detail");
    expect(detail.textContent).toContain("tóu / tou");
    expect(detail.textContent).toContain("此处读音存疑");
    expect(screen.getByTestId("uncertain-mark").getAttribute("aria-expanded")).toBe("true");
  });

  it("没有读音数据时写「暂无注音」，不造占位读音", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    const absent = screen.getByTitle("暂无注音");
    // 格内是短横而不是四个字：短横不是读音，所以「不造占位读音」照旧成立，
    // 而完整措辞在 title 里，含义另由覆盖说明逐字讲明。
    expect(absent.textContent).toBe("—");
    const note = screen.getByTestId("pinyin-coverage").textContent ?? "";
    expect(note).toContain("格内标为「—」");
  });

  it("四档覆盖按绝对数量公布，并声明破读只覆盖名册内作品", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    const note = screen.getByTestId("pinyin-coverage").textContent ?? "";
    const fixture = annotationFixture();
    expect(note).toContain(`有据破读 ${fixture.coverage.attested} 字`);
    // 界面上出现的记号都要在这条说明里讲明含义，否则它只是一个看不懂的装饰。
    expect(note).toContain("拼音右上标「据」");
    expect(note).toContain("点开看并列候选");
    expect(note).toContain(`存疑 ${fixture.coverage.uncertain} 字`);
    expect(note).toContain(`暂无注音 ${fixture.coverage.absent} 字`);
    // 覆盖边界要如实：不得宣称破读覆盖完整。
    expect(note).toContain("名册之外");
  });

  it("标点没有读音层，也不顶一个「暂无注音」", async () => {
    const given = ports();
    mount(given);
    await annotated(given);

    const marks = [...screen.getByTestId("poem-body").querySelectorAll("[data-reading]")];
    const punctuation = marks.filter((node) => node.getAttribute("data-reading") === "none");
    expect(punctuation.length).toBe(1);
    expect(punctuation[0]?.textContent).toBe("");
  });
});

describe("两层分居正文两侧且不争同一侧", () => {
  it("拼音在字上方的 ruby 里，平仄在正文之后的独立一行", async () => {
    const given = ports();
    mount(given);
    await annotated(given);
    fireEvent.click(screen.getByTestId("tone-toggle"));

    const line = screen.getByTestId("poem-body").querySelector(".poem-body__line");
    const text = line?.querySelector(".poem-body__text");
    const toneRow = line?.querySelector(".poem-body__tones");
    if (text === null || text === undefined || toneRow === null || toneRow === undefined) {
      throw new Error("正文格或平仄行没有渲染出来，本条断言的前提不成立");
    }

    // 拼音在正文格内部（ruby 的注位），平仄在正文格之后（同级的下一个节点）。
    // 两者不是同一种定位机制，因此争不到同一侧。
    expect(text.querySelectorAll("rt").length).toBeGreaterThan(0);
    expect(toneRow.querySelectorAll("rt").length).toBe(0);
    expect(text.compareDocumentPosition(toneRow) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
  });

  it("两层同开时格数逐行相等，标点位在平仄那一行留空", async () => {
    const given = ports();
    mount(given);
    await annotated(given);
    fireEvent.click(screen.getByTestId("tone-toggle"));

    const lines = [...screen.getByTestId("poem-body").querySelectorAll(".poem-body__line")];
    expect(lines.length).toBe(2);
    for (const line of lines) {
      const chars = line.querySelectorAll(".poem-body__char").length;
      const tones = line.querySelectorAll(".poem-body__tone").length;
      // 格数相等是对齐的构造性保证：不相等就一定有一层从某处开始整体错位。
      expect(tones).toBe(chars);
    }

    // 第二行有一个标点，它那一格的平仄是空的（而不是被上一个字的平仄顶上）。
    const second = lines[1];
    const blanks = [...(second?.querySelectorAll(".poem-body__tone") ?? [])].filter(
      (node) => node.textContent === "",
    );
    expect(blanks.length).toBe(1);
  });
});

describe("两个开关独立持久化，互不暗开", () => {
  it("开拼音不会把平仄一起带上", async () => {
    const given = ports();
    mount(given);
    await settled();

    fireEvent.click(screen.getByTestId("pinyin-toggle"));

    expect((screen.getByTestId("pinyin-toggle") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByTestId("tone-toggle") as HTMLInputElement).checked).toBe(false);
    expect(localStorage.getItem(ANNOTATION_PREFERENCE_KEYS.pinyin)).toBe("true");
    expect(localStorage.getItem(ANNOTATION_PREFERENCE_KEYS.tones)).not.toBe("true");
  });

  it("开平仄不会把拼音一起带上", async () => {
    const given = ports();
    mount(given);
    await settled();

    fireEvent.click(screen.getByTestId("tone-toggle"));

    expect((screen.getByTestId("tone-toggle") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByTestId("pinyin-toggle") as HTMLInputElement).checked).toBe(false);
    expect(localStorage.getItem(ANNOTATION_PREFERENCE_KEYS.tones)).toBe("true");
    expect(localStorage.getItem(ANNOTATION_PREFERENCE_KEYS.pinyin)).not.toBe("true");
  });

  it("两个键各自决定重挂载后的初值", async () => {
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.pinyin, "true");
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.tones, "false");

    const given = ports();
    mount(given);
    await settled();

    expect((screen.getByTestId("pinyin-toggle") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByTestId("tone-toggle") as HTMLInputElement).checked).toBe(false);
  });

  it("两个键写的不是同一个键", () => {
    expect(ANNOTATION_PREFERENCE_KEYS.pinyin).not.toBe(ANNOTATION_PREFERENCE_KEYS.tones);
  });
});

describe("切换开关不产生新的后端调用", () => {
  it("反复切两个开关，注音与详情的调用次数都不增加", async () => {
    const given = ports();
    mount(given);
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    const detailBefore = given.detailCalls();
    const annotationBefore = given.annotationCalls();

    for (const id of ["pinyin-toggle", "tone-toggle", "pinyin-toggle", "tone-toggle"]) {
      fireEvent.click(screen.getByTestId(id));
    }
    // 点开存疑详情也是纯本地状态，不该借机去取一次。
    fireEvent.click(screen.getByTestId("pinyin-toggle"));
    await waitFor(() => {
      expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
    });
    fireEvent.click(screen.getByTestId("uncertain-mark"));

    expect(given.annotationCalls()).toBe(annotationBefore);
    expect(given.detailCalls()).toBe(detailBefore);
  });

  it("整首注音是一次批量，不按字发查询", async () => {
    const given = ports();
    mount(given);
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    fireEvent.click(screen.getByTestId("pinyin-toggle"));
    await waitFor(() => {
      expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
    });

    // 正文有 11 个内容字。逐字查询会让计数变成两位数，一次批量则恒为 1。
    expect(given.annotationCalls()).toBe(1);
    expect(screen.getByTestId("poem-body").querySelectorAll("rt").length).toBeGreaterThan(10);
  });

  it("注音取不到时不显示注音层，也不编造读音", async () => {
    const given = ports(new Error("注音服务不可用"));
    mount(given);
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    fireEvent.click(screen.getByTestId("pinyin-toggle"));

    expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("off");
    expect(screen.queryByTestId("pinyin-coverage")).toBeNull();
    // 原文照旧完整可读：注音失败不能把阅读也一起废掉。
    expect(screen.getByTestId("poem-original").textContent).toContain("床前明月光");
  });
});

describe("无提示主动回忆", () => {
  it("两层默认隐藏，即使偏好里两个都开着", async () => {
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.pinyin, "true");
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.tones, "true");

    const given = ports();
    mount(given, { recall: true });
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    // 开关本身仍是「开」——偏好没有被改写；隐藏的是这一步的呈现。
    expect((screen.getByTestId("pinyin-toggle") as HTMLInputElement).checked).toBe(true);
    expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("off");
    expect(screen.queryByTestId("tone-row")).toBeNull();
  });

  it("主动揭示只记为支架使用，回调里没有任何判错的位置", async () => {
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.pinyin, "true");
    const revealed: unknown[] = [];
    const given = ports();
    mount(given, {
      recall: true,
      onReveal: (...args: unknown[]) => {
        revealed.push(...args);
      },
    } as { recall: boolean; onReveal: () => void });
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    fireEvent.click(screen.getByTestId("reveal-pinyin"));

    await waitFor(() => {
      expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
    });
    // 回调只收到层名。**没有第二个参数**，所以「揭示顺手判个错」在这里无处落笔。
    expect(revealed).toEqual(["pinyin"]);
  });

  it("揭示拼音不会把平仄一起揭示", async () => {
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.pinyin, "true");
    localStorage.setItem(ANNOTATION_PREFERENCE_KEYS.tones, "true");

    const given = ports();
    mount(given, { recall: true });
    await settled();
    await waitFor(() => {
      expect(given.annotationCalls()).toBe(1);
    });

    fireEvent.click(screen.getByTestId("reveal-pinyin"));

    await waitFor(() => {
      expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
    });
    expect(screen.queryByTestId("tone-row")).toBeNull();
  });
});
