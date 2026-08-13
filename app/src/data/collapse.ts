/**
 * 按 `work_group` 把重出的检索结果折成一行，并保留「不折叠」这条退路。
 *
 * # 为什么折叠而不是静默择一
 *
 * `work_group` 是**不含作者**的分组键（`crates/yunjian-corpus/src/model.rs:401-405`：
 * 去空白标点后正文的 BLAKE3 前缀），刻意如此设计正是为了让**归属冲突可被检测**
 * （`.omo/notepads/yunjian/decisions.md` 的「两个分组键」一节）。
 * 一首诗在语料里可能既挂张若虚又挂贺知章；后端不替用户选一个，前端也不能。
 * 折叠 + 标注「另见 N 处异说」的意思是「我们知道有分歧，点进去看」，
 * 而静默取第一条等于替用户做了一个考据判断。
 *
 * # 折叠是**展示层**的事，不是后端行为
 *
 * 已核实：`search_text` 不按 `work_group` 折叠，CLI 也**没有** `--no-dedup` 参数
 * （`crates/yunjian-cli/src/cli.rs:55-74` 的 `Search` 只有 query/limit/author/dynasty/
 * rhyme-book/cursor）。唯一的既有折叠实现是 MCP 的相似诗
 * （`crates/yunjian-mcp/src/lib.rs:872-883` 的 `dedup_by_work_group`），
 * 它同样「先去重再截断」。本模块沿用它的两条语义：
 *
 * 1. **保序**：入参顺序即后端给的相关性顺序，组的位置由该组第一条命中决定。
 * 2. **首条为代表**：不重新排序、不挑「最好的版本」——那又是一个考据判断。
 */

import type { SearchRow } from "./rows";

/** 一个折叠后的组。 */
export interface CollapsedGroup {
  /** 代表行，取该组在原顺序里的第一条。 */
  primary: SearchRow;
  /**
   * 同组的其余行。
   *
   * 命名刻意是 `variants`（异说）而不是 `duplicates`（重复）：它们**不是**冗余数据，
   * 而是同一正文的不同归属或不同版本，每一条都有自己的出处。
   */
  variants: SearchRow[];
  /**
   * 这一组是不是真的按 `work_group` 判定出来的。
   *
   * `work_group` 为 `null` 的行（正文检索命中）会各自成为一个 `collapsible: false` 的组。
   * 界面据此**不显示**「另见」标注——那会声称一个我们没有做过的判定。
   */
  collapsible: boolean;
}

/**
 * 折叠。
 *
 * `dedup` 为 `false` 时等价于 CLI 语义上的 `--no-dedup`：每行各自成组，一条都不合并，
 * 于是用户能看到语料里真实的重出情况。
 */
export function collapseByWorkGroup(rows: readonly SearchRow[], dedup: boolean): CollapsedGroup[] {
  if (!dedup) {
    return rows.map((row) => ({ primary: row, variants: [], collapsible: false }));
  }

  const groups: CollapsedGroup[] = [];
  const indexByWorkGroup = new Map<string, number>();

  for (const row of rows) {
    const key = row.work_group;
    if (key === null) {
      groups.push({ primary: row, variants: [], collapsible: false });
      continue;
    }
    const existing = indexByWorkGroup.get(key);
    if (existing === undefined) {
      indexByWorkGroup.set(key, groups.length);
      groups.push({ primary: row, variants: [], collapsible: true });
      continue;
    }
    const group = groups[existing];
    if (group !== undefined) {
      group.variants.push(row);
    }
  }

  return groups;
}

/**
 * 「另见 N 处异说」的标注文本，`N` 为同组其余归属的条数。
 *
 * 只有该组确实按 `work_group` 判定过且真有其余条目时才有文本；否则为 `null`，
 * 让调用方少渲染一个节点而不是渲染一句「另见 0 处异说」。
 */
export function variantAnnotation(group: CollapsedGroup): string | null {
  if (!group.collapsible || group.variants.length === 0) {
    return null;
  }
  return `另见 ${group.variants.length} 处异说`;
}
