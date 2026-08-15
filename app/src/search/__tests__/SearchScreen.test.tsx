/**
 * 检索页的端到端行为：输入、高亮、筛选、分页、异说折叠开关。
 *
 * 用替身端口而不是样例端口：样例数据是给人看的，测试要的是可控输入。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { MetaPage, SearchPage, TagSummary } from "../../contracts/core";
import type { SearchPort, TextSearchRequest } from "../../data/ports";
import SearchScreen from "../SearchScreen";

function page(hits: SearchPage["hits"], nextCursor: string | null = null): SearchPage {
  return { hits, total_estimate: hits.length, next_cursor: nextCursor };
}

function hit(id: string, title: string, author: string, line: string, at: number) {
  return {
    poem_id: id,
    title,
    author,
    dynasty: "唐",
    matched_line_index: 0,
    snippet: { text: line, highlights: [{ start: at, end: at + 2 }] },
  };
}

interface PortOverrides {
  searchText?: SearchPort["searchText"];
  browseByTag?: SearchPort["browseByTag"];
  listTags?: SearchPort["listTags"];
}

function portFixture(overrides: PortOverrides = {}): SearchPort {
  return {
    searchText: overrides.searchText ?? (() => Promise.resolve(page([]))),
    browseByTag:
      overrides.browseByTag ??
      (() => Promise.resolve({ hits: [], next_cursor: null, normalized: "" } satisfies MetaPage)),
    listTags: overrides.listTags ?? (() => Promise.resolve([] satisfies TagSummary[])),
  };
}

function search(port: SearchPort) {
  const onOpen = vi.fn();
  render(<SearchScreen port={port} onOpen={onOpen} />);
  return { onOpen };
}

async function submit(query: string) {
  fireEvent.change(screen.getByTestId("search-input"), { target: { value: query } });
  fireEvent.click(screen.getByTestId("search-submit"));
  await waitFor(() => {
    expect(screen.queryByTestId("search-loading")).toBeNull();
  });
}

describe("检索", () => {
  it("已有文本再次聚焦后仍可继续输入且不会提前发请求", () => {
    const searchText = vi.fn(() => Promise.resolve(page([])));
    search(portFixture({ searchText }));
    const input = screen.getByTestId("search-input") as HTMLInputElement;

    fireEvent.change(input, { target: { value: "明月" } });
    fireEvent.blur(input);
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "明月千里" } });

    expect(input.value).toBe("明月千里");
    expect(searchText).not.toHaveBeenCalled();
  });

  it("提交检索词后列出命中并高亮", async () => {
    const port = portFixture({
      searchText: () => Promise.resolve(page([hit("p-1", "静夜思", "李白", "床前明月光", 2)])),
    });
    search(port);
    await submit("明月");

    expect(screen.getByText("静夜思")).toBeDefined();
    const mark = screen.getByText("明月");
    expect(mark.tagName).toBe("MARK");
  });

  it("把检索词与硬上限内的 limit 传给端口", async () => {
    const searchText = vi.fn((_request: TextSearchRequest) => Promise.resolve(page([])));
    search(portFixture({ searchText }));
    await submit("明月");

    expect(searchText).toHaveBeenCalledWith({ query: "明月", limit: 20, cursor: null });
    // 服务端硬上限是 100（TEXT_SEARCH_HARD_CAP），请求量必须在它之内。
    expect(searchText.mock.calls[0]?.[0].limit).toBeLessThanOrEqual(100);
  });

  it("空检索词不发请求", async () => {
    const searchText = vi.fn(() => Promise.resolve(page([])));
    search(portFixture({ searchText }));
    fireEvent.click(screen.getByTestId("search-submit"));
    expect(searchText).not.toHaveBeenCalled();
  });

  it("零命中显示「没有命中」而不是一片空白", async () => {
    search(portFixture());
    await submit("不存在的句子");
    expect(screen.getByTestId("result-empty")).toBeDefined();
  });

  it("检索失败显示原因", async () => {
    search(
      portFixture({
        searchText: () => Promise.reject(new Error("语料库还没就位")),
      }),
    );
    await submit("明月");
    expect(screen.getByTestId("search-error").textContent).toContain("语料库还没就位");
  });

  // Tauri 的 `invoke` 失败时 reject 的是**字符串**，不是 `Error`。只判 `instanceof Error`
  // 会让真实原因整条丢掉，界面上只剩一句「检索失败」——真机验收里正是这句话把一次
  // IPC 失败伪装成了「检索功能坏了」，而它本可以自己说出坏在哪。
  it("检索被字符串 reject 时也显示原因", async () => {
    search(
      portFixture({
        searchText: () => Promise.reject("数据库错误：attempt to write a readonly database"),
      }),
    );
    await submit("明月");
    expect(screen.getByTestId("search-error").textContent).toContain("readonly database");
  });

  it("点结果行回调 poem_id", async () => {
    const port = portFixture({
      searchText: () => Promise.resolve(page([hit("p-1", "静夜思", "李白", "床前明月光", 2)])),
    });
    const { onOpen } = search(port);
    await submit("明月");

    fireEvent.click(screen.getByText("静夜思"));
    expect(onOpen).toHaveBeenCalledWith("p-1");
  });
});

describe("页内过滤与它必须说出口的限制", () => {
  const twoAuthors = () =>
    Promise.resolve(
      page([
        hit("p-1", "静夜思", "李白", "床前明月光", 2),
        hit("p-2", "月夜", "杜甫", "今夜月明人尽望", 2),
      ]),
    );

  it("作者过滤只留匹配的行", async () => {
    search(portFixture({ searchText: twoAuthors }));
    await submit("月");

    expect(screen.getAllByTestId("result-row")).toHaveLength(2);
    fireEvent.change(screen.getByLabelText("作者"), { target: { value: "李白" } });
    expect(screen.getAllByTestId("result-row")).toHaveLength(1);
  });

  it("检索后就显示「只在本页过滤」的说明", async () => {
    // `TextSearchRequest` 没有服务端过滤入口，所以「本页没有」不等于「语料里没有」。
    // 不说这件事，用户会把前者读成后者。
    search(portFixture({ searchText: twoAuthors }));
    await submit("月");
    expect(screen.getByTestId("page-scope-note").textContent).toContain("当前这一页");
  });
});

describe("主题筛选走另一条查询", () => {
  it("选定主题后调 browseByTag 而不是 searchText", async () => {
    const searchText = vi.fn(() => Promise.resolve(page([])));
    const browseByTag = vi.fn(() =>
      Promise.resolve({
        hits: [
          {
            stable_id: "p-9",
            title: "春晓",
            title_raw: "春晓",
            ci_tune: null,
            author: "孟浩然",
            dynasty: { canonical: "唐", raw: "唐" },
            first_line: "春眠不觉晓",
            work_group: "wg-9",
            genre: "shi",
            line_count: 4,
            char_count: 20,
            matched_line_index: null,
          },
        ],
        next_cursor: null,
        normalized: "春景",
      } satisfies MetaPage),
    );
    search(
      portFixture({
        searchText,
        browseByTag,
        listTags: () => Promise.resolve([{ name: "春景", poem_count: 1 }]),
      }),
    );

    await waitFor(() => {
      expect(screen.getByLabelText("主题")).toBeDefined();
    });
    fireEvent.change(screen.getByLabelText("主题"), { target: { value: "春景" } });

    await waitFor(() => {
      expect(browseByTag).toHaveBeenCalledWith({ tag: "春景", cursor: null });
    });
    expect(searchText).not.toHaveBeenCalled();
    expect(screen.getByTestId("theme-scope-note")).toBeDefined();
  });
});

describe("游标分页", () => {
  const first = () => page([hit("p-1", "一", "甲", "床前明月光", 2)], "cursor-2");

  it("有 next_cursor 时「下一页」可用，末页时禁用", async () => {
    const searchText = vi.fn((request: TextSearchRequest) =>
      Promise.resolve(
        request.cursor === null ? first() : page([hit("p-2", "二", "乙", "举头望明月", 3)]),
      ),
    );
    search(portFixture({ searchText }));
    await submit("明月");

    const next = screen.getByTestId("page-next");
    expect(next.hasAttribute("disabled")).toBe(false);

    fireEvent.click(next);
    await waitFor(() => {
      expect(screen.getByText("二")).toBeDefined();
    });
    expect(searchText).toHaveBeenLastCalledWith({
      query: "明月",
      limit: 20,
      cursor: "cursor-2",
    });
    expect(screen.getByTestId("page-next").hasAttribute("disabled")).toBe(true);
  });

  it("首页时「上一页」禁用，翻过一页后可用并回到首页游标", async () => {
    // 游标是单向的、不透明的，所以「上一页」只能靠客户端记住用过的游标。
    const searchText = vi.fn((request: TextSearchRequest) =>
      Promise.resolve(
        request.cursor === null ? first() : page([hit("p-2", "二", "乙", "举头望明月", 3)]),
      ),
    );
    search(portFixture({ searchText }));
    await submit("明月");

    expect(screen.getByTestId("page-previous").hasAttribute("disabled")).toBe(true);

    fireEvent.click(screen.getByTestId("page-next"));
    await waitFor(() => {
      expect(screen.getByText("二")).toBeDefined();
    });
    expect(screen.getByTestId("page-previous").hasAttribute("disabled")).toBe(false);

    fireEvent.click(screen.getByTestId("page-previous"));
    await waitFor(() => {
      expect(screen.getByText("一")).toBeDefined();
    });
    expect(searchText).toHaveBeenLastCalledWith({ query: "明月", limit: 20, cursor: null });
  });

  it("重新检索把游标栈清空", async () => {
    const searchText = vi.fn((request: TextSearchRequest) =>
      Promise.resolve(
        request.cursor === null ? first() : page([hit("p-2", "二", "乙", "举头望明月", 3)]),
      ),
    );
    search(portFixture({ searchText }));
    await submit("明月");
    fireEvent.click(screen.getByTestId("page-next"));
    await waitFor(() => {
      expect(screen.getByText("二")).toBeDefined();
    });

    await submit("依山");
    expect(screen.getByTestId("page-previous").hasAttribute("disabled")).toBe(true);
  });
});

describe("异说折叠开关", () => {
  // 正文检索的命中没有 work_group，所以折叠只在元数据（主题浏览）结果上看得见。
  const dual = () =>
    Promise.resolve({
      hits: ["a", "b"].map((suffix, index) => ({
        stable_id: `p-${suffix}`,
        title: "登鹳雀楼",
        title_raw: "登鹳雀楼",
        ci_tune: null,
        author: index === 0 ? "王之涣" : "朱斌",
        dynasty: { canonical: "唐", raw: "唐" },
        first_line: "白日依山尽",
        work_group: "wg-1",
        genre: "shi",
        line_count: 4,
        char_count: 20,
        matched_line_index: null,
      })),
      next_cursor: null,
      normalized: "登临",
    } satisfies MetaPage);

  async function openTheme() {
    search(
      portFixture({
        browseByTag: dual,
        listTags: () => Promise.resolve([{ name: "登临", poem_count: 2 }]),
      }),
    );
    await waitFor(() => {
      expect(screen.getByLabelText("主题")).toBeDefined();
    });
    fireEvent.change(screen.getByLabelText("主题"), { target: { value: "登临" } });
    await waitFor(() => {
      expect(screen.getAllByTestId("result-row").length).toBeGreaterThan(0);
    });
  }

  it("默认折叠成一行并标注「另见 1 处异说」", async () => {
    await openTheme();
    expect(screen.getAllByTestId("result-row")).toHaveLength(1);
    expect(screen.getByTestId("variant-annotation").textContent).toBe("另见 1 处异说");
  });

  it("打开「显示全部异说」后两条各占一行且无标注", async () => {
    await openTheme();
    fireEvent.click(screen.getByTestId("no-dedup-toggle"));

    expect(screen.getAllByTestId("result-row")).toHaveLength(2);
    expect(screen.queryByTestId("variant-annotation")).toBeNull();
    expect(screen.getByText("唐 · 朱斌 · shi")).toBeDefined();
  });
});
