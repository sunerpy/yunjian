/**
 * 两种高亮各自的判据。
 *
 * # 为什么两向都要验
 *
 * 只断言「前缀长度算对了」不足以守住这条边界：一个**恒返回 0** 的实现同样能让「不宣称
 * 字级结论」看起来成立，而那时高亮整段消失、功能实际没了。所以既验它在该匹配时确实匹配，
 * 也验它在不该匹配时确实不匹配。
 */

import { describe, expect, it } from "vitest";
import { markAt } from "../../contracts/voice";
import type { FootMark } from "../../contracts/voice";
import { contentChars, matchedPrefixLength } from "../voiceHighlight";

const LINE_MARKS: FootMark[] = [
  { line: 0, index_in_line: 0, text: "床前", start_ms: 0, end_ms: 500 },
  { line: 0, index_in_line: 1, text: "明月光", start_ms: 620, end_ms: 1_370 },
];

describe("karaoke 高亮取自拼接时间戳", () => {
  it("时刻落在某个音步内时命中它", () => {
    expect(markAt(LINE_MARKS, 0)).toBe(0);
    expect(markAt(LINE_MARKS, 499)).toBe(0);
    expect(markAt(LINE_MARKS, 620)).toBe(1);
    expect(markAt(LINE_MARKS, 1_369)).toBe(1);
  });

  it("**音步之间的静音不命中任何音步**", () => {
    // 这一条不是补充：音步之间有 120 ms 的静音（`Prosody::CLASSICAL` 的 `foot_pause_ms`），
    // 而那正是节奏的实现方式。用闭区间会让 500 ms 那一帧同时命中两个音步，
    // 用「取最近一个」会让静音期间前一个音步一直亮着，两者都把停顿画没了。
    expect(markAt(LINE_MARKS, 500)).toBe(-1);
    expect(markAt(LINE_MARKS, 610)).toBe(-1);
  });

  it("超出末音步之后不再命中", () => {
    expect(markAt(LINE_MARKS, 1_370)).toBe(-1);
    expect(markAt(LINE_MARKS, 9_999)).toBe(-1);
  });

  it("没有示范过时（空标记）恒不命中", () => {
    expect(markAt([], 0)).toBe(-1);
  });
});

describe("已匹配前缀取自偏置假设", () => {
  it("公共前缀按字计，两侧都先剥掉标点", () => {
    // 参考文本带标点而转写不带；不剥的话第一个逗号就会把前缀截断在两个字上。
    expect(matchedPrefixLength("床前明月光，", "床前明月")).toBe(4);
    expect(matchedPrefixLength("床前明月光", "床前明月光")).toBe(5);
  });

  it("第一个不同的字处截断，**不做编辑距离对齐**", () => {
    // 编辑距离会给出「漏了哪几个字」，而那正是 CER 77% 下不可报告的东西。
    expect(matchedPrefixLength("床前明月光", "床钱明月光")).toBe(1);
  });

  it("没有偏置假设时为 0，而不是当成全匹配", () => {
    expect(matchedPrefixLength("床前明月光", null)).toBe(0);
    expect(matchedPrefixLength("床前明月光", "")).toBe(0);
  });

  it("转写比参考长时不越界", () => {
    expect(matchedPrefixLength("床前", "床前明月光")).toBe(2);
  });

  it("正文字符判据把标点与空白剥干净", () => {
    expect(contentChars("床前明月光，")).toEqual(["床", "前", "明", "月", "光"]);
    expect(contentChars(" 举头 望明月。").join("")).toBe("举头望明月");
  });
});
