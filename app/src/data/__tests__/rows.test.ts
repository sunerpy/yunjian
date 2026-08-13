/**
 * 命中形状归一与页内过滤。
 */

import { describe, expect, it } from "vitest";
import type { MetaHit, TextSearchHit } from "../../contracts/core";
import {
  EMPTY_FILTERS,
  applyFilters,
  hasActiveFilters,
  rowFromMetaHit,
  rowFromTextHit,
} from "../rows";

const TEXT_HIT: TextSearchHit = {
  poem_id: "p-1",
  title: "静夜思",
  author: "李白",
  dynasty: "唐",
  matched_line_index: 0,
  snippet: { text: "床前明月光", highlights: [{ start: 2, end: 4 }] },
};

const META_HIT: MetaHit = {
  stable_id: "p-2",
  title: "登鹳雀楼",
  title_raw: "登鹳雀楼",
  ci_tune: null,
  author: "王之涣",
  dynasty: { canonical: "唐", raw: "唐" },
  first_line: "白日依山尽",
  work_group: "wg-1",
  genre: "shi",
  line_count: 4,
  char_count: 20,
  matched_line_index: null,
};

describe("正文命中归一", () => {
  it("work_group 与 genre 恒为 null，因为 TextSearchHit 没有这两个字段", () => {
    const row = rowFromTextHit(TEXT_HIT);
    expect(row.work_group).toBeNull();
    expect(row.genre).toBeNull();
  });

  it("poem_id 与高亮原样带过来", () => {
    const row = rowFromTextHit(TEXT_HIT);
    expect(row.poem_id).toBe("p-1");
    expect(row.snippet.highlights).toEqual([{ start: 2, end: 4 }]);
  });
});

describe("元数据命中归一", () => {
  it("stable_id 映射到 poem_id", () => {
    expect(rowFromMetaHit(META_HIT).poem_id).toBe("p-2");
  });

  it("dynasty 取 canonical 而不是整个 DynastyLabel 对象", () => {
    expect(rowFromMetaHit(META_HIT).dynasty).toBe("唐");
  });

  it("work_group 与 genre 带过来，所以元数据命中可以参与折叠", () => {
    const row = rowFromMetaHit(META_HIT);
    expect(row.work_group).toBe("wg-1");
    expect(row.genre).toBe("shi");
  });

  it("摘要退回首句且不带高亮，不给它编一段高亮出来", () => {
    // 元数据检索不返回命中句文本，只有 matched_line_index 与 first_line。
    const row = rowFromMetaHit(META_HIT);
    expect(row.snippet).toEqual({ text: "白日依山尽", highlights: [] });
  });
});

describe("页内过滤", () => {
  const rows = [rowFromTextHit(TEXT_HIT), rowFromMetaHit(META_HIT)];

  it("空过滤条件不筛掉任何东西", () => {
    expect(applyFilters(rows, EMPTY_FILTERS)).toHaveLength(2);
    expect(hasActiveFilters(EMPTY_FILTERS)).toBe(false);
  });

  it("作者用包含匹配", () => {
    const filtered = applyFilters(rows, { ...EMPTY_FILTERS, author: "李" });
    expect(filtered.map((row) => row.poem_id)).toEqual(["p-1"]);
  });

  it("朝代用全等匹配，避免「唐」命中「五代十国」之类的组合键", () => {
    expect(applyFilters(rows, { ...EMPTY_FILTERS, dynasty: "唐" })).toHaveLength(2);
    expect(applyFilters(rows, { ...EMPTY_FILTERS, dynasty: "宋" })).toHaveLength(0);
  });

  it("体裁过滤开启时排除 genre 为 null 的行，而不是放它们过去", () => {
    // 保留会让用户以为它们通过了检查，而事实是我们不知道它们的体裁。
    const filtered = applyFilters(rows, { ...EMPTY_FILTERS, genre: "shi" });
    expect(filtered.map((row) => row.poem_id)).toEqual(["p-2"]);
  });

  it("只有空白的过滤条件视为未启用", () => {
    expect(hasActiveFilters({ author: "  ", dynasty: "", genre: "" })).toBe(false);
    expect(applyFilters(rows, { author: "  ", dynasty: "", genre: "" })).toHaveLength(2);
  });
});
