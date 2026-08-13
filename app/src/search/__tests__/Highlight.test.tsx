/**
 * 命中高亮的切分。
 *
 * 重点在**按码点切而不是按码元切**。JS 的 `slice` 按 UTF-16 码元切，而
 * `HighlightRange` 的下标是 Unicode 字符下标（`crates/yunjian-core/src/search/text.rs:32-34`
 * 明确写了这一点）。对 BMP 内的汉字两者相同，对增补平面的字会整段错位——
 * 而增补平面正是异体字最集中的地方，语料里有的。
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import Highlight, { splitByHighlights } from "../Highlight";

describe("按码点切", () => {
  it("常见汉字：区间对上", () => {
    const pieces = splitByHighlights({
      text: "床前明月光",
      highlights: [{ start: 2, end: 4 }],
    });
    expect(pieces).toEqual([
      { text: "床前", highlighted: false },
      { text: "明月", highlighted: true },
      { text: "光", highlighted: false },
    ]);
  });

  it("增补平面的字算一个字符而不是两个码元", () => {
    // U+20000（𠀀）在 JS 里 `length === 2`。按 slice 切会把高亮整段往后错一位。
    const text = "𠀀明月𠀀";
    expect(text.length).toBe(6);
    expect(Array.from(text)).toHaveLength(4);

    const pieces = splitByHighlights({ text, highlights: [{ start: 1, end: 3 }] });
    expect(pieces).toEqual([
      { text: "𠀀", highlighted: false },
      { text: "明月", highlighted: true },
      { text: "𠀀", highlighted: false },
    ]);
  });
});

describe("边界", () => {
  it("没有高亮时整段一片", () => {
    expect(splitByHighlights({ text: "白日依山尽", highlights: [] })).toEqual([
      { text: "白日依山尽", highlighted: false },
    ]);
  });

  it("从头开始的高亮不产生空的前导片段", () => {
    expect(splitByHighlights({ text: "明月光", highlights: [{ start: 0, end: 2 }] })).toEqual([
      { text: "明月", highlighted: true },
      { text: "光", highlighted: false },
    ]);
  });

  it("到尾结束的高亮不产生空的尾随片段", () => {
    expect(splitByHighlights({ text: "望明月", highlights: [{ start: 1, end: 3 }] })).toEqual([
      { text: "望", highlighted: false },
      { text: "明月", highlighted: true },
    ]);
  });

  it("多段高亮各自成片", () => {
    const pieces = splitByHighlights({
      text: "明月照明月",
      highlights: [
        { start: 0, end: 2 },
        { start: 3, end: 5 },
      ],
    });
    expect(pieces.filter((piece) => piece.highlighted).map((piece) => piece.text)).toEqual([
      "明月",
      "明月",
    ]);
  });

  it("重叠区间合并，绝不让字符重复出现", () => {
    // 一个重叠区间会让朴素实现产出重复字符——那是「文字凭空多出来」，
    // 比高亮位置不对严重得多。
    const pieces = splitByHighlights({
      text: "床前明月光",
      highlights: [
        { start: 1, end: 3 },
        { start: 2, end: 4 },
      ],
    });
    expect(pieces.map((piece) => piece.text).join("")).toBe("床前明月光");
    expect(pieces.filter((piece) => piece.highlighted).map((piece) => piece.text)).toEqual([
      "前明月",
    ]);
  });

  it("越界区间被夹到文本长度内，不抛异常", () => {
    const pieces = splitByHighlights({ text: "明月", highlights: [{ start: 1, end: 99 }] });
    expect(pieces.map((piece) => piece.text).join("")).toBe("明月");
  });

  it("空区间与倒置区间被丢掉", () => {
    const pieces = splitByHighlights({
      text: "明月",
      highlights: [
        { start: 1, end: 1 },
        { start: 2, end: 0 },
      ],
    });
    expect(pieces).toEqual([{ text: "明月", highlighted: false }]);
  });

  it("任何输入下拼回去都等于原文", () => {
    for (const highlights of [
      [],
      [{ start: 0, end: 1 }],
      [{ start: 3, end: 5 }],
      [
        { start: 0, end: 2 },
        { start: 1, end: 4 },
      ],
      [{ start: -5, end: 2 }],
    ]) {
      const pieces = splitByHighlights({ text: "床前明月光", highlights });
      expect(pieces.map((piece) => piece.text).join("")).toBe("床前明月光");
    }
  });
});

describe("渲染", () => {
  it("命中部分用 mark，屏幕阅读器会读出「标记」", () => {
    render(<Highlight snippet={{ text: "床前明月光", highlights: [{ start: 2, end: 4 }] }} />);
    const marks = screen.getAllByText("明月");
    expect(marks).toHaveLength(1);
    expect(marks[0]?.tagName).toBe("MARK");
  });
});
