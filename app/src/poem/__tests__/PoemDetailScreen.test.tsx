/**
 * 详情页：集评错误态、异说逐条列出、AI 面板排在考据材料之后。
 *
 * 这一组与 `provenanceSeparation.test.tsx` 互补：那一组单独渲染三块看它们的关系，
 * 这一组走真实的详情页组装，验证在**完整页面**里那些关系依然成立。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppreciationState } from "../../contracts/ai";
import type { CommentaryEntry, PoemDetail } from "../../contracts/core";
import type { AppreciationPort, PoemPort } from "../../data/ports";
import PoemDetailScreen from "../PoemDetailScreen";

function commentary(overrides: Partial<CommentaryEntry["citation"]> = {}): CommentaryEntry {
  return {
    id: "c-1",
    text: "语极浅而意极远。",
    citation: {
      work: "沧浪诗话",
      author: "严羽",
      dynasty: { canonical: "宋", raw: "宋" },
      work_completed_by: 1250,
      source_note: "卷一，据历代诗话本",
      ...overrides,
    },
  };
}

/** 空注音：既有用例一律不开拼音层，形状齐全而内容为空即可。 */
const EMPTY_COVERAGE = { attested: 0, generic: 0, uncertain: 0, absent: 0 };

function detailFixture(overrides: Partial<PoemDetail> = {}): PoemDetail {
  return {
    poem: {
      stable_id: "p-1",
      content_hash: "h",
      title: "登鹳雀楼",
      title_raw: "登鹳雀楼",
      ci_tune: null,
      author: "王之涣",
      dynasty: { canonical: "唐", raw: "唐" },
      genre: "shi",
      body: "白日依山尽\n黄河入海流",
      body_original: "白日依山尽\n黄河入海流",
      script: "simplified",
      first_line: "白日依山尽",
      last_chars: ["山", "海"],
      line_count: 2,
      char_count: 10,
      work_group: "wg-1",
      edition_group: "wg-1-王之涣",
    },
    author: { name: "王之涣", dynasties: [{ canonical: "唐", raw: "唐" }], poem_count: 6 },
    tones: {
      book: "pingshui",
      lines: [
        {
          line_index: 0,
          text: "白日依山尽",
          cells: [
            { character: "白", tone: "oblique", readings: [] },
            { character: "日", tone: "oblique", readings: [] },
            { character: "依", tone: "level", readings: [] },
            { character: "山", tone: "level", readings: [] },
            { character: "尽", tone: "unknown", readings: [] },
          ],
        },
      ],
      unknown_count: 1,
      either_count: 0,
    },
    rhyme_groups: [{ book: "pingshui", group: "尤", tone: "level", confidence: "unambiguous" }],
    work_group_siblings: [],
    attribution_conflict: null,
    provenance: {
      source_locator: "chinese-poetry:全唐诗:1234",
      source_locator_kind: "upstream_id",
      source: "chinese-poetry",
      revision: "abc1234",
      kind: "json",
      license: "MIT",
      license_class: "public_domain",
    },
    tags: ["登临"],
    commentaries: [commentary()],
    ...overrides,
  };
}

function mount(detail: PoemDetail, appreciation: AppreciationState = { kind: "absent" }) {
  const poemPort: PoemPort = {
    poemDetail: () => Promise.resolve(detail),
    poemAnnotations: (request) =>
      Promise.resolve({ poem_id: request.poem_id, lines: [], coverage: EMPTY_COVERAGE }),
  };
  const appreciationPort: AppreciationPort = { appreciate: () => Promise.resolve(appreciation) };
  const onBack = vi.fn();
  render(
    <PoemDetailScreen
      poemId={detail.poem.stable_id}
      poemPort={poemPort}
      appreciationPort={appreciationPort}
      onBack={onBack}
    />,
  );
  return { onBack };
}

async function settled() {
  await waitFor(() => {
    expect(screen.getByTestId("poem-detail")).toBeDefined();
  });
}

describe("原文与平仄", () => {
  it("原文按行渲染", async () => {
    mount(detailFixture());
    await settled();
    expect(screen.getByText("白日依山尽")).toBeDefined();
    expect(screen.getByText("黄河入海流")).toBeDefined();
  });

  it("平仄默认不标注", async () => {
    mount(detailFixture());
    await settled();
    expect(screen.queryByTestId("tone-row")).toBeNull();
  });

  it("打开后逐字标注，未知位置标「？」而不是留空", async () => {
    // 留空看起来像「这个字没有平仄」，而真相是「我们的韵书里查不到它」。
    mount(detailFixture());
    await settled();
    fireEvent.click(screen.getByTestId("tone-toggle"));

    const row = screen.getByTestId("tone-row");
    expect(row.textContent).toBe("仄仄平平？");
    expect(screen.getByTestId("tone-unknown-note").textContent).toContain("平水韵");
  });
});

describe("韵部", () => {
  it("按韵书列出韵部、声调与可信度", async () => {
    mount(detailFixture());
    await settled();
    const entry = screen.getByTestId("rhyme-entry");
    expect(entry.textContent).toContain("平水韵");
    expect(entry.textContent).toContain("尤");
    expect(entry.textContent).toContain("平声");
    expect(entry.textContent).toContain("唯一读音");
  });

  it("按投票推定的可信度照实说出来，不显示成事实", async () => {
    mount(
      detailFixture({
        rhyme_groups: [
          { book: "cilin", group: "第十二部", tone: "departing", confidence: "resolved_by_vote" },
        ],
      }),
    );
    await settled();
    expect(screen.getByTestId("rhyme-entry").textContent).toContain("按同篇韵脚推定");
  });
});

describe("集评的出处", () => {
  it("每条集评下方内联显示出处", async () => {
    mount(detailFixture());
    await settled();
    expect(screen.getByTestId("commentary-citation").textContent).toBe(
      "—— 宋 严羽《沧浪诗话》（1250）卷一，据历代诗话本",
    );
  });

  it("缺出处的条目渲染成错误态，而不是空文本", async () => {
    mount(
      detailFixture({
        commentaries: [
          commentary(),
          { ...commentary(), id: "c-2", citation: { ...commentary().citation, source_note: "" } },
        ],
      }),
    );
    await settled();

    expect(screen.getAllByTestId("commentary-entry")).toHaveLength(1);
    const error = screen.getByTestId("commentary-error");
    expect(error.textContent).toContain("c-2");
    expect(error.textContent).toContain("缺出处");
    expect(error.textContent).toContain("拒绝展示");
  });

  it("缺出处的条目其正文一个字都不出现在页面上", async () => {
    // 这是最核心的一条：无法复核的评语不得作为考据材料呈现。
    const broken: CommentaryEntry = {
      id: "c-broken",
      text: "这条不该出现在页面上。",
      citation: { ...commentary().citation, source_note: "  " },
    };
    mount(detailFixture({ commentaries: [broken] }));
    await settled();

    expect(screen.queryByText("这条不该出现在页面上。")).toBeNull();
    expect(screen.getByTestId("poem-detail").textContent).not.toContain("这条不该出现在页面上");
  });

  it("空而存在的 source_note 与整个 citation 缺失都判错误态", async () => {
    const noCitation = { id: "c-x", text: "无出处对象。" } as unknown as CommentaryEntry;
    mount(detailFixture({ commentaries: [noCitation] }));
    await settled();

    expect(screen.getByTestId("commentary-error").textContent).toContain("整个出处对象");
    expect(screen.queryByTestId("commentary-entry")).toBeNull();
  });

  it("没有集评时说「暂无」，不留一片空白", async () => {
    mount(detailFixture({ commentaries: [] }));
    await settled();
    expect(screen.getByTestId("commentary-empty")).toBeDefined();
  });
});

describe("异说：每一种归属及其来源", () => {
  const dual = detailFixture({
    work_group_siblings: [
      {
        stable_id: "p-2",
        author: "朱斌",
        dynasty: { canonical: "唐", raw: "唐" },
        title: "登鹳雀楼",
        source_locator: "chinese-poetry:文苑英华:99",
        provenance_source: "chinese-poetry",
        provenance_revision: "def5678",
      },
    ],
    attribution_conflict: {
      work_group: "wg-1",
      attributions: [],
    },
  });

  it("两种归属都列出来，含本篇自己", async () => {
    // 只列 siblings 会让用户看到 N-1 种说法，却看不到「当前显示的是哪一种」。
    mount(dual);
    await settled();

    const entries = screen.getAllByTestId("attribution-entry");
    expect(entries).toHaveLength(2);
    expect(entries[0]?.textContent).toContain("王之涣");
    expect(entries[1]?.textContent).toContain("朱斌");
  });

  it("每一种归属都带自己的来源与 revision", async () => {
    mount(dual);
    await settled();
    const entries = screen.getAllByTestId("attribution-entry");
    expect(entries[0]?.textContent).toContain("abc1234");
    expect(entries[0]?.textContent).toContain("chinese-poetry:全唐诗:1234");
    expect(entries[1]?.textContent).toContain("def5678");
    expect(entries[1]?.textContent).toContain("chinese-poetry:文苑英华:99");
  });

  it("有冲突时说明不替用户择一", async () => {
    mount(dual);
    await settled();
    expect(screen.getByTestId("poem-attributions").textContent).toContain("不替你择一");
  });

  it("没有同组记录时整块不出现", async () => {
    mount(detailFixture());
    await settled();
    expect(screen.queryByTestId("poem-attributions")).toBeNull();
  });
});

describe("整页里 AI 与考据的关系", () => {
  it("AI 面板在原文、韵部、异说、集评全部之后", async () => {
    mount(detailFixture(), {
      kind: "ready",
      view: { text: "赏析。", model: "m-1", template_version: "1.0.0" },
    });
    await settled();

    const ai = screen.getByTestId("ai-panel");
    for (const id of ["poem-original", "poem-rhyme", "poem-commentary"]) {
      const relation = screen.getByTestId(id).compareDocumentPosition(ai);
      expect(relation & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });

  it("整页里没有任何 sourced 容器包住 AI 容器", async () => {
    mount(detailFixture(), {
      kind: "ready",
      view: { text: "赏析。", model: "m-1", template_version: "1.0.0" },
    });
    await settled();

    for (const node of document.querySelectorAll('[data-provenance="sourced"]')) {
      expect(node.querySelectorAll('[data-provenance="ai-generated"]')).toHaveLength(0);
    }
    for (const node of document.querySelectorAll('[data-provenance="ai-generated"]')) {
      expect(node.querySelectorAll('[data-provenance="sourced"]')).toHaveLength(0);
    }
  });

  it("AI 面板即使没有内容也带标签与披露", async () => {
    mount(detailFixture(), { kind: "absent" });
    await settled();
    expect(screen.getByTestId("ai-panel-label").textContent).toBe("AI 赏析");
    expect(screen.getByTestId("ai-disclosure")).toBeDefined();
  });
});

describe("失败与返回", () => {
  it("读取失败显示原因而不是空页", async () => {
    const poemPort: PoemPort = {
      poemDetail: () => Promise.reject(new Error("集评 c-9 缺出处")),
      poemAnnotations: (request) =>
        Promise.resolve({ poem_id: request.poem_id, lines: [], coverage: EMPTY_COVERAGE }),
    };
    render(
      <PoemDetailScreen
        poemId="p-1"
        poemPort={poemPort}
        appreciationPort={{ appreciate: () => Promise.resolve({ kind: "absent" }) }}
        onBack={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("detail-error").textContent).toContain("缺出处");
    });
  });

  it("AI 端口抛异常时面板落到失败态，不影响考据材料", async () => {
    const poemPort: PoemPort = {
      poemDetail: () => Promise.resolve(detailFixture()),
      poemAnnotations: (request) =>
        Promise.resolve({ poem_id: request.poem_id, lines: [], coverage: EMPTY_COVERAGE }),
    };
    render(
      <PoemDetailScreen
        poemId="p-1"
        poemPort={poemPort}
        appreciationPort={{ appreciate: () => Promise.reject(new Error("密钥无效")) }}
        onBack={vi.fn()}
      />,
    );
    await settled();
    await waitFor(() => {
      expect(screen.getByTestId("ai-failed").textContent).toContain("密钥无效");
    });
    expect(screen.getByTestId("commentary-citation")).toBeDefined();
  });

  it("返回按钮回调", async () => {
    const { onBack } = mount(detailFixture());
    await settled();
    fireEvent.click(screen.getByTestId("detail-back"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
