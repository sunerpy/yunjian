/**
 * `work_group` 折叠与「另见 N 处异说」标注。
 */

import { describe, expect, it } from "vitest";
import type { SearchRow } from "../rows";
import { collapseByWorkGroup, variantAnnotation } from "../collapse";

function row(id: string, author: string, workGroup: string | null): SearchRow {
  return {
    poem_id: id,
    title: "登鹳雀楼",
    author,
    dynasty: "唐",
    work_group: workGroup,
    genre: "shi",
    snippet: { text: "白日依山尽", highlights: [] },
  };
}

describe("按 work_group 折叠", () => {
  it("两种归属折成一行，代表是原顺序里的第一条", () => {
    const groups = collapseByWorkGroup(
      [row("a", "王之涣", "wg-1"), row("b", "朱斌", "wg-1")],
      true,
    );

    expect(groups).toHaveLength(1);
    expect(groups[0]?.primary.author).toBe("王之涣");
    expect(groups[0]?.variants.map((variant) => variant.author)).toEqual(["朱斌"]);
  });

  it("标注文本是「另见 1 处异说」", () => {
    const groups = collapseByWorkGroup(
      [row("a", "王之涣", "wg-1"), row("b", "朱斌", "wg-1")],
      true,
    );
    expect(variantAnnotation(groups[0]!)).toBe("另见 1 处异说");
  });

  it("三条同组折成一行且标注 2 处", () => {
    const groups = collapseByWorkGroup(
      [row("a", "甲", "wg-1"), row("b", "乙", "wg-1"), row("c", "丙", "wg-1")],
      true,
    );
    expect(groups).toHaveLength(1);
    expect(variantAnnotation(groups[0]!)).toBe("另见 2 处异说");
  });

  it("组的先后由该组第一条命中的位置决定，不重排", () => {
    // 入参顺序即后端给的相关性顺序。按组名重排会悄悄改变结果排序。
    const groups = collapseByWorkGroup(
      [row("a", "甲", "wg-2"), row("b", "乙", "wg-1"), row("c", "丙", "wg-2")],
      true,
    );
    expect(groups.map((group) => group.primary.poem_id)).toEqual(["a", "b"]);
    expect(groups[0]?.variants.map((variant) => variant.poem_id)).toEqual(["c"]);
  });

  it("不同组不合并", () => {
    const groups = collapseByWorkGroup([row("a", "甲", "wg-1"), row("b", "乙", "wg-2")], true);
    expect(groups).toHaveLength(2);
    expect(groups.every((group) => group.variants.length === 0)).toBe(true);
  });
});

describe("单条不标注", () => {
  it("独一份的组没有「另见」文本，而不是「另见 0 处异说」", () => {
    const groups = collapseByWorkGroup([row("a", "甲", "wg-1")], true);
    expect(variantAnnotation(groups[0]!)).toBeNull();
  });
});

describe("work_group 为 null 的行", () => {
  it("各自成组，一条都不合并", () => {
    // `TextSearchHit` 没有 `work_group`（已核对 `crates/yunjian-core/src/search/text.rs:50-65`）。
    // 用空串代替 null 会把全部正文命中当成同一组折成一条——那是把「缺数据」伪装成「已判定」。
    const groups = collapseByWorkGroup([row("a", "甲", null), row("b", "乙", null)], true);
    expect(groups).toHaveLength(2);
  });

  it("不带「另见」标注，因为我们没有做过那个判定", () => {
    const groups = collapseByWorkGroup([row("a", "甲", null), row("b", "乙", null)], true);
    expect(groups.every((group) => group.collapsible === false)).toBe(true);
    expect(groups.map(variantAnnotation)).toEqual([null, null]);
  });
});

describe("不折叠开关（--no-dedup 的等价物）", () => {
  it("关掉折叠后同组两条各占一行", () => {
    const rows = [row("a", "王之涣", "wg-1"), row("b", "朱斌", "wg-1")];
    const groups = collapseByWorkGroup(rows, false);

    expect(groups).toHaveLength(2);
    expect(groups.map((group) => group.primary.author)).toEqual(["王之涣", "朱斌"]);
  });

  it("关掉折叠后不出现任何「另见」标注", () => {
    const rows = [row("a", "王之涣", "wg-1"), row("b", "朱斌", "wg-1")];
    expect(collapseByWorkGroup(rows, false).map(variantAnnotation)).toEqual([null, null]);
  });

  it("开关只影响分组，不丢也不增记录", () => {
    const rows = [row("a", "甲", "wg-1"), row("b", "乙", "wg-1"), row("c", "丙", null)];
    const collapsed = collapseByWorkGroup(rows, true);
    const flattened = collapsed.flatMap((group) => [group.primary, ...group.variants]);

    expect(flattened).toHaveLength(rows.length);
    expect(new Set(flattened.map((entry) => entry.poem_id))).toEqual(
      new Set(rows.map((entry) => entry.poem_id)),
    );
  });
});
