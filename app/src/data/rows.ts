/**
 * 把两种后端命中形状归一成列表行，并施加**页内**过滤。
 *
 * # 为什么过滤是页内的
 *
 * 这不是权宜之计，是照抄已验证的既有语义。CLI 的 `--author` / `--dynasty` 在文档与代码里
 * 都明确写着「**在本页内过滤**」（`crates/yunjian-cli/src/cli.rs:63` 与 `:67`），
 * 因为 `TextSearchRequest` 只有 `query` / `limit` / `cursor` 三个字段
 * （`crates/yunjian-core/src/search/text.rs:9-18`），根本没有服务端过滤入口。
 * 界面必须把这件事说出来，否则用户会把「本页没有杜甫」读成「杜甫没写过」。
 *
 * `total_estimate` 同理是**过滤前**的数，CLI 的 `SearchOut` 注释也这么写
 * （`crates/yunjian-cli/src/output.rs:56-58`）。
 */

import type { HighlightedSnippet, MetaHit, TextSearchHit } from "../contracts/core";

/**
 * 检索结果行的归一化形状。
 *
 * 正文检索（`TextSearchHit`）与元数据检索（`MetaHit`）字段不同却要显示在同一个列表里，
 * 所以有这一层。它是**前端视图模型而不是 core 形状**，因此住在 `data/` 而不是 `contracts/`。
 */
export interface SearchRow {
  /** 作品稳定标识。`TextSearchHit.poem_id` 与 `MetaHit.stable_id` 是同一个东西。 */
  poem_id: string;
  title: string;
  author: string;
  /** 朝代规范键。`MetaHit` 那侧取 `dynasty.canonical`。 */
  dynasty: string;
  /**
   * 异说折叠键。
   *
   * **`null` 表示「这一行不具备折叠所需的信息」**，不是「不属于任何组」。
   * `TextSearchHit` 确实没有 `work_group`（已核对 `search/text.rs:50-65`），
   * 于是正文检索的结果一律不折叠。用空串代替 `null` 会让所有正文命中被当成同一组折成一条，
   * 那是把「缺数据」伪装成「已判定」。
   */
  work_group: string | null;
  /** 体裁；正文检索拿不到，为 `null`。 */
  genre: string | null;
  /** 命中句及其高亮；元数据检索没有命中句时用首句且无高亮。 */
  snippet: HighlightedSnippet;
}

/**
 * 正文检索命中 -> 列表行。
 *
 * `work_group` 与 `genre` 恒为 `null`：`TextSearchHit` 没有这两个字段。
 */
export function rowFromTextHit(hit: TextSearchHit): SearchRow {
  return {
    poem_id: hit.poem_id,
    title: hit.title,
    author: hit.author,
    dynasty: hit.dynasty,
    work_group: null,
    genre: null,
    snippet: hit.snippet,
  };
}

/**
 * 元数据命中 -> 列表行。
 *
 * 三处映射都核对过 `crates/yunjian-core/src/search/meta.rs:103-131`：
 * `stable_id` 而不是 `poem_id`、`dynasty.canonical` 而不是裸串、`work_group` 与 `genre` 都在。
 *
 * 元数据检索**不返回命中句文本**（只有 `matched_line_index` 与 `first_line`），
 * 所以摘要退回首句且不带高亮。给它编一段高亮就是编造后端没说的话。
 */
export function rowFromMetaHit(hit: MetaHit): SearchRow {
  const snippet: HighlightedSnippet = { text: hit.first_line, highlights: [] };
  return {
    poem_id: hit.stable_id,
    title: hit.title,
    author: hit.author,
    dynasty: hit.dynasty.canonical,
    work_group: hit.work_group,
    genre: hit.genre,
    snippet,
  };
}

/** 页内过滤条件。全部可留空，留空即不过滤。 */
export interface RowFilters {
  author: string;
  dynasty: string;
  genre: string;
}

/** 空过滤条件，供初始状态与测试共用，免得两处各写一份。 */
export const EMPTY_FILTERS: RowFilters = { author: "", dynasty: "", genre: "" };

/** 过滤条件是否为空——决定要不要显示「页内过滤」提示。 */
export function hasActiveFilters(filters: RowFilters): boolean {
  return (
    filters.author.trim() !== "" || filters.dynasty.trim() !== "" || filters.genre.trim() !== ""
  );
}

/**
 * 施加页内过滤。
 *
 * 作者用**包含**匹配（与 CLI 的前缀语义相比更宽，因为界面上用户是边打边看）；
 * 朝代与体裁用**全等**匹配，它们是规范键而不是自由文本，包含匹配会让「唐」命中「五代十国」
 * 之类的组合键。
 *
 * `genre` 为 `null` 的行（正文检索命中）在体裁过滤开启时**被排除而不是保留**：
 * 保留会让用户以为它们通过了检查，而事实是我们不知道它们的体裁。
 */
export function applyFilters(rows: readonly SearchRow[], filters: RowFilters): SearchRow[] {
  const author = filters.author.trim();
  const dynasty = filters.dynasty.trim();
  const genre = filters.genre.trim();

  return rows.filter((row) => {
    if (author !== "" && !row.author.includes(author)) {
      return false;
    }
    if (dynasty !== "" && row.dynasty !== dynasty) {
      return false;
    }
    if (genre !== "" && row.genre !== genre) {
      return false;
    }
    return true;
  });
}
