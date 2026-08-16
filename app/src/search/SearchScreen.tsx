/**
 * 检索页。
 *
 * # 分页是游标式的，所以「上一页」要靠自己记
 *
 * 后端只给 `next_cursor`（`crates/yunjian-core/src/search/text.rs:73-74`），游标是不透明串，
 * 不能构造也不能反向推。所以往回翻只能靠客户端把用过的游标压成一个栈。
 * 这不是取巧——游标式分页本来就单向，硬造一个「上一页游标」等于伪造后端契约。
 */

import { useCallback, useEffect, useState } from "react";
import type { TagSummary } from "../contracts/core";
import type { SearchPort } from "../data/ports";
import { errorReason } from "../data/errorReason";
import type { RowFilters, SearchRow } from "../data/rows";
import { EMPTY_FILTERS, applyFilters, rowFromMetaHit, rowFromTextHit } from "../data/rows";
import { collapseByWorkGroup } from "../data/collapse";
import FilterBar from "./FilterBar";
import ResultList from "./ResultList";
import "./search.css";

/** 单页请求量。低于服务端硬上限 100（`TEXT_SEARCH_HARD_CAP`）。 */
const PAGE_LIMIT = 20;

export interface SearchScreenProps {
  port: SearchPort;
  onOpen: (poemId: string) => void;
}

interface PageState {
  rows: SearchRow[];
  totalEstimate: number | null;
  nextCursor: string | null;
}

const EMPTY_PAGE: PageState = { rows: [], totalEstimate: null, nextCursor: null };

export default function SearchScreen({ port, onOpen }: SearchScreenProps) {
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [theme, setTheme] = useState("");
  const [filters, setFilters] = useState<RowFilters>(EMPTY_FILTERS);
  const [dedup, setDedup] = useState(true);
  const [tags, setTags] = useState<TagSummary[]>([]);
  const [page, setPage] = useState<PageState>(EMPTY_PAGE);
  // 栈里存的是「取出当前这一页时用的游标」，首页是 null。
  const [cursorStack, setCursorStack] = useState<(string | null)[]>([null]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    port
      .listTags()
      .then((list) => {
        if (!disposed) {
          setTags(list);
        }
      })
      .catch(() => {
        // 标签取不到只让主题筛选不可用，不该让整页失败。
        if (!disposed) {
          setTags([]);
        }
      });
    return () => {
      disposed = true;
    };
  }, [port]);

  const fetchPage = useCallback(
    async (activeQuery: string, activeTheme: string, cursor: string | null) => {
      setLoading(true);
      setError(null);
      try {
        if (activeTheme !== "") {
          const result = await port.browseByTag({ tag: activeTheme, cursor });
          return {
            rows: result.hits.map(rowFromMetaHit),
            totalEstimate: null,
            nextCursor: result.next_cursor,
          } satisfies PageState;
        }
        const result = await port.searchText({
          query: activeQuery,
          limit: PAGE_LIMIT,
          cursor,
        });
        return {
          rows: result.hits.map(rowFromTextHit),
          totalEstimate: result.total_estimate,
          nextCursor: result.next_cursor,
        } satisfies PageState;
      } finally {
        setLoading(false);
      }
    },
    [port],
  );

  const load = useCallback(
    (activeQuery: string, activeTheme: string, cursor: string | null, stack: (string | null)[]) => {
      void fetchPage(activeQuery, activeTheme, cursor)
        .then((next) => {
          setPage(next);
          setCursorStack(stack);
        })
        .catch((cause: unknown) => {
          setPage(EMPTY_PAGE);
          setError(errorReason(cause, "检索失败"));
        });
    },
    [fetchPage],
  );

  const submit = useCallback(() => {
    const trimmed = draft.trim();
    if (trimmed === "" && theme === "") {
      return;
    }
    setQuery(trimmed);
    load(trimmed, theme, null, [null]);
  }, [draft, theme, load]);

  const changeTheme = useCallback(
    (next: string) => {
      setTheme(next);
      if (next !== "") {
        load(query, next, null, [null]);
      } else if (query !== "") {
        load(query, "", null, [null]);
      } else {
        setPage(EMPTY_PAGE);
        setCursorStack([null]);
      }
    },
    [query, load],
  );

  const goNext = useCallback(() => {
    if (page.nextCursor === null) {
      return;
    }
    load(query, theme, page.nextCursor, [...cursorStack, page.nextCursor]);
  }, [page.nextCursor, query, theme, cursorStack, load]);

  const goPrevious = useCallback(() => {
    if (cursorStack.length <= 1) {
      return;
    }
    const trimmed = cursorStack.slice(0, -1);
    const target = trimmed[trimmed.length - 1] ?? null;
    load(query, theme, target, trimmed);
  }, [cursorStack, query, theme, load]);

  const filtered = applyFilters(page.rows, filters);
  const groups = collapseByWorkGroup(filtered, dedup);
  const searched = query !== "" || theme !== "";

  return (
    <div className="search-screen">
      <form
        className="search-screen__form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}>
        <label className="search-screen__field">
          <span className="search-screen__label">检索正文或残句</span>
          <input
            type="search"
            className="search-screen__input"
            value={draft}
            placeholder="明月"
            onChange={(event) => {
              setDraft(event.target.value);
            }}
            data-testid="search-input"
          />
        </label>
        <button type="submit" className="search-screen__submit" data-testid="search-submit">
          检索
        </button>
      </form>

      <FilterBar
        filters={filters}
        onFiltersChange={setFilters}
        tags={tags}
        theme={theme}
        onThemeChange={changeTheme}
        dedup={dedup}
        onDedupChange={setDedup}
        showPageScopeNote={searched}
      />

      {error !== null && (
        <p className="search-screen__error" role="alert" data-testid="search-error">
          {error}
        </p>
      )}

      {loading && (
        <p className="search-screen__loading" data-testid="search-loading">
          正在检索…
        </p>
      )}

      {searched && !loading && error === null && (
        <>
          <p className="search-screen__summary" data-testid="search-summary">
            {page.totalEstimate === null
              ? `本页 ${groups.length} 条`
              : `估计命中 ${page.totalEstimate} 条，本页显示 ${groups.length} 条`}
          </p>
          <ResultList groups={groups} onOpen={onOpen} />
          <nav className="search-screen__paging" aria-label="分页">
            <button
              type="button"
              onClick={goPrevious}
              disabled={cursorStack.length <= 1}
              data-testid="page-previous">
              上一页
            </button>
            <button
              type="button"
              onClick={goNext}
              disabled={page.nextCursor === null}
              data-testid="page-next">
              下一页
            </button>
          </nav>
        </>
      )}
    </div>
  );
}
