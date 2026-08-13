/**
 * 考据材料与 AI 生成内容的**视觉与结构区分**。
 *
 * # 为什么这一组要单独存在
 *
 * 「AI 面板的容器类与原文、集评的容器类不同」这条要求，如果只写在 AI 面板自己的测试里，
 * 就只能证明 AI 面板用了某个类；证明不了那个类**与另外两块不一样**——
 * 而后者才是要求的原文。所以这一组同时渲染三块，对着它们之间的关系下断言。
 *
 * 三个维度，缺一个都能让「区分」名存实亡：
 *
 * 1. **类名不相交**：AI 面板不得复用任何 `sourced-block*` 类。
 * 2. **容器不嵌套**：`data-provenance` 的两种值不得互相包含。嵌套正是「交错」在 DOM 里的形态。
 * 3. **顺序在后**：AI 面板排在原文与集评之后。
 *
 * 另有一条对着样式表本身：两个语域的令牌组不得混用。类名再不同，
 * 只要 `.ai-panel` 用了 `--color-sourced-surface`，视觉上仍然是一样的。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AppreciationState } from "../../contracts/ai";
import type { CommentaryEntry, PoemDetail } from "../../contracts/core";
import AiAppreciationPanel from "../AiAppreciationPanel";
import CommentaryList from "../CommentaryList";
import OriginalText from "../OriginalText";

const COMMENTARY: CommentaryEntry = {
  id: "c-1",
  text: "语极浅而意极远。",
  citation: {
    work: "沧浪诗话",
    author: "严羽",
    dynasty: { canonical: "宋", raw: "宋" },
    work_completed_by: 1250,
    source_note: "卷一，据历代诗话本",
  },
};

const POEM: PoemDetail["poem"] = {
  stable_id: "p-1",
  content_hash: "h",
  title: "静夜思",
  title_raw: "静夜思",
  ci_tune: null,
  author: "李白",
  dynasty: { canonical: "唐", raw: "唐" },
  genre: "shi",
  body: "床前明月光\n疑是地上霜",
  body_original: "床前明月光\n疑是地上霜",
  script: "simplified",
  first_line: "床前明月光",
  last_chars: ["月", "上"],
  line_count: 2,
  char_count: 10,
  work_group: "wg-1",
  edition_group: "wg-1-李白",
};

const TONES: PoemDetail["tones"] = {
  book: "pingshui",
  lines: [],
  unknown_count: 0,
  either_count: 0,
};

const AI_STATE: AppreciationState = {
  kind: "ready",
  view: { text: "赏析文本。", model: "m-1", template_version: "1.0.0" },
};

function classesOf(element: Element): string[] {
  return [...element.classList];
}

describe("容器类不相交", () => {
  it("AI 面板不带任何 sourced-block 类", () => {
    const { getByTestId } = render(<AiAppreciationPanel state={AI_STATE} />);
    const classes = classesOf(getByTestId("ai-panel"));
    expect(classes).toContain("ai-panel");
    expect(classes.filter((name) => name.startsWith("sourced-block"))).toEqual([]);
  });

  it("原文块不带任何 ai- 类", () => {
    const { getByTestId } = render(<OriginalText poem={POEM} tones={TONES} showTones={false} />);
    const classes = classesOf(getByTestId("poem-original"));
    expect(classes).toContain("sourced-block");
    expect(classes.filter((name) => name.startsWith("ai-"))).toEqual([]);
  });

  it("集评块不带任何 ai- 类", () => {
    const { getByTestId } = render(<CommentaryList commentaries={[COMMENTARY]} />);
    const classes = classesOf(getByTestId("poem-commentary"));
    expect(classes).toContain("sourced-block");
    expect(classes.filter((name) => name.startsWith("ai-"))).toEqual([]);
  });

  it("三个容器两两之间的类名集合互不相等", () => {
    const ai = render(<AiAppreciationPanel state={AI_STATE} />);
    const aiClasses = new Set(classesOf(ai.getByTestId("ai-panel")));
    ai.unmount();

    const original = render(<OriginalText poem={POEM} tones={TONES} showTones={false} />);
    const originalClasses = new Set(classesOf(original.getByTestId("poem-original")));
    original.unmount();

    const commentary = render(<CommentaryList commentaries={[COMMENTARY]} />);
    const commentaryClasses = new Set(classesOf(commentary.getByTestId("poem-commentary")));
    commentary.unmount();

    // AI 与两块考据材料完全不共享类；原文与集评共享 `sourced-block` 但各有自己的修饰类。
    for (const sourced of [originalClasses, commentaryClasses]) {
      const shared = [...aiClasses].filter((name) => sourced.has(name));
      expect(shared).toEqual([]);
    }
    expect(originalClasses).not.toEqual(commentaryClasses);
  });
});

describe("溯源标记", () => {
  it("AI 面板标 ai-generated，两块考据材料标 sourced", () => {
    const ai = render(<AiAppreciationPanel state={AI_STATE} />);
    expect(ai.getByTestId("ai-panel").getAttribute("data-provenance")).toBe("ai-generated");
    ai.unmount();

    const original = render(<OriginalText poem={POEM} tones={TONES} showTones={false} />);
    expect(original.getByTestId("poem-original").getAttribute("data-provenance")).toBe("sourced");
    original.unmount();

    const commentary = render(<CommentaryList commentaries={[COMMENTARY]} />);
    expect(commentary.getByTestId("poem-commentary").getAttribute("data-provenance")).toBe(
      "sourced",
    );
  });

  it("AI 容器里不含任何 sourced 容器，反之亦然——嵌套就是交错", () => {
    const { getByTestId } = render(
      <>
        <OriginalText poem={POEM} tones={TONES} showTones={false} />
        <CommentaryList commentaries={[COMMENTARY]} />
        <AiAppreciationPanel state={AI_STATE} />
      </>,
    );

    const aiPanel = getByTestId("ai-panel");
    expect(aiPanel.querySelectorAll('[data-provenance="sourced"]')).toHaveLength(0);

    for (const id of ["poem-original", "poem-commentary"]) {
      expect(getByTestId(id).querySelectorAll('[data-provenance="ai-generated"]')).toHaveLength(0);
    }
  });

  it("AI 面板在文档顺序上排在原文与集评之后", () => {
    const { getByTestId } = render(
      <>
        <OriginalText poem={POEM} tones={TONES} showTones={false} />
        <CommentaryList commentaries={[COMMENTARY]} />
        <AiAppreciationPanel state={AI_STATE} />
      </>,
    );

    const ai = getByTestId("ai-panel");
    for (const id of ["poem-original", "poem-commentary"]) {
      // DOCUMENT_POSITION_FOLLOWING = 4：AI 面板在该块之后。
      const relation = getByTestId(id).compareDocumentPosition(ai);
      expect(relation & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });
});

describe("样式令牌不混用", () => {
  const css = readFileSync(resolve(process.cwd(), "src/poem/poem.css"), "utf8");

  /** 取一条选择器的规则体。样式表是手写的，因此按 `选择器 { … }` 粗切即可。 */
  function ruleBody(selector: string): string {
    const bodies: string[] = [];
    const pattern = new RegExp(`(^|\\n)${selector.replace(".", "\\.")}[^{]*\\{([^}]*)\\}`, "g");
    let match = pattern.exec(css);
    while (match !== null) {
      bodies.push(match[2] ?? "");
      match = pattern.exec(css);
    }
    return bodies.join("\n");
  }

  it(".ai-panel 不引用任何 --color-sourced-* 或 --color-citation-* 令牌", () => {
    // 类名不同但令牌相同，视觉上仍然一样——那条要求就名存实亡了。
    const body = ruleBody(".ai-panel");
    expect(body).not.toContain("--color-sourced");
    expect(body).not.toContain("--color-citation");
  });

  it(".sourced-block 不引用任何 --color-ai-* 令牌", () => {
    const body = ruleBody(".sourced-block");
    expect(body).not.toContain("--color-ai-");
  });

  it("两者的字体族刻意相反：考据用衬线，AI 用无衬线", () => {
    // 在一个通篇衬线排古典诗词的界面里，无衬线本身就是最强的「这不是原始材料」信号。
    expect(ruleBody(".sourced-block")).toContain("--font-serif");
    expect(ruleBody(".ai-panel")).toContain("--font-sans");
  });

  it("AI 面板用虚线边框，考据材料用实线", () => {
    expect(ruleBody(".ai-panel")).toMatch(/border:\s*2px dashed/);
    expect(ruleBody(".sourced-block")).toMatch(/border:\s*1px solid/);
  });

  it("令牌层为两个语域各声明了自己的一组颜色", () => {
    const tokens = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");
    for (const token of [
      "--color-sourced-surface",
      "--color-sourced-border",
      "--color-ai-surface",
      "--color-ai-border",
    ]) {
      expect(tokens).toContain(token);
    }
  });
});
