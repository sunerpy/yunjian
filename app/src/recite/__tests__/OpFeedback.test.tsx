/**
 * 逐字反馈面板：五类差异在 DOM 上必须可分，且每一格都有可读的说明。
 *
 * # 为什么在这里再断言一次「可分」
 *
 * `opMark.test.ts` 断言的是**描述子表与样式表**可分；这一组断言的是**渲染结果**可分。
 * 两者能各自成立而合起来不成立：表里六个类名都不同，组件却把它们全渲染成同一个类。
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ReciteOp } from "../../contracts/recite";
import OpFeedback from "../OpFeedback";
import { OP_MARKS, OP_MARK_KINDS } from "../opMark";

/** 五类差异齐备的一组操作，外加两个相符项。 */
const OPS: ReciteOp[] = [
  { kind: "normal", reference_index: 0, attempt_index: 0, character: "床" },
  { kind: "deletion", reference_index: 1, reference: "前" },
  { kind: "insertion", reference_index: 2, attempt_index: 1, attempt: "啊" },
  {
    kind: "re_recitation",
    reference_start: 2,
    reference_end: 5,
    attempt_start: 2,
    attempt_end: 5,
    text: "明月光",
  },
  {
    kind: "substitution",
    reference_index: 12,
    attempt_index: 12,
    reference: "望",
    attempt: "看",
    near_homophone: false,
  },
  {
    kind: "substitution",
    reference_index: 17,
    attempt_index: 17,
    reference: "思",
    attempt: "丝",
    near_homophone: true,
  },
  { kind: "normal", reference_index: 19, attempt_index: 19, character: "乡" },
];

function opElements(): HTMLElement[] {
  return [...screen.getByTestId("op-feedback").querySelectorAll<HTMLElement>("[data-op]")];
}

describe("逐字反馈", () => {
  it("**五类差异各自渲染出不同的视觉标记**", () => {
    render(<OpFeedback ops={OPS} />);
    const cells = opElements();
    expect(cells).toHaveLength(OPS.length);

    // 1) `data-op` 把六个类别逐一标出来，五类差异一个不漏。
    const kinds = cells.map((cell) => cell.dataset.op);
    expect(kinds).toEqual([
      "normal",
      "deletion",
      "insertion",
      "re_recitation",
      "substitution",
      "near_homophone_substitution",
      "normal",
    ]);

    // 2) 五类差异的样式类两两不同。
    const differenceCells = cells.filter((cell) => cell.dataset.op !== "normal");
    const classNames = differenceCells.map((cell) => cell.className);
    expect(new Set(classNames).size).toBe(differenceCells.length);

    // 3) 记号字符两两不同，且是描述子表里的那一个。
    const marks = differenceCells.map(
      (cell) => cell.querySelector(".recite-op__mark")?.textContent ?? "",
    );
    expect(new Set(marks).size).toBe(differenceCells.length);
    expect(marks).toEqual(["✗", "＋", "↻", "≠", "≈"]);
  });

  it("每一格都有 aria-label，不靠颜色分辨", () => {
    render(<OpFeedback ops={OPS} />);
    for (const cell of opElements()) {
      expect(
        cell.getAttribute("aria-label") ?? "",
        `${cell.dataset.op ?? "?"} 缺 aria-label`,
      ).not.toBe("");
      // 悬停提示与读屏说明取同一句：两句不同的说法比一句更糟。
      expect(cell.getAttribute("title")).toBe(cell.getAttribute("aria-label"));
    }
  });

  it("只有替换类带应读字角标，这是结构上的区分而不只是样式", () => {
    render(<OpFeedback ops={OPS} />);
    const withExpected = opElements().filter(
      (cell) => cell.querySelector(".recite-op__expected") !== null,
    );
    expect(withExpected.map((cell) => cell.dataset.op)).toEqual([
      "substitution",
      "near_homophone_substitution",
    ]);
    expect(withExpected[0]?.querySelector(".recite-op__expected")?.textContent).toBe("望");
    expect(withExpected[1]?.querySelector(".recite-op__expected")?.textContent).toBe("思");
  });

  it("差异清单只列非相符项，措辞与格子上的说明一致", () => {
    render(<OpFeedback ops={OPS} />);
    const items = [...screen.getByTestId("op-differences").querySelectorAll("li")];
    expect(items).toHaveLength(5);
    expect(items[0]?.textContent).toContain("第 2 字 漏读：应读「前」");
    expect(items[4]?.textContent).toContain("第 18 字 近音替换：应读「思」，实读「丝」");
    expect(screen.queryByTestId("no-differences")).toBeNull();
  });

  it("全对时说「全篇相符」而不是留一个空列表", () => {
    render(
      <OpFeedback
        ops={[{ kind: "normal", reference_index: 0, attempt_index: 0, character: "床" }]}
      />,
    );
    expect(screen.getByTestId("no-differences").textContent).toContain("全篇相符");
    expect(screen.queryByTestId("op-differences")).toBeNull();
  });

  it("图例复用与格子相同的类，于是不可能与渲染不一致", () => {
    render(<OpFeedback ops={OPS} />);
    const legend = screen.getByTestId("op-legend");
    for (const kind of OP_MARK_KINDS) {
      const entry = legend.querySelector<HTMLElement>(`[data-legend="${kind}"]`);
      expect(entry, `图例缺 ${kind}`).not.toBeNull();
      expect(entry?.className).toContain(OP_MARKS[kind].className);
      expect(entry?.textContent).toContain(OP_MARKS[kind].label);
    }
  });
});
