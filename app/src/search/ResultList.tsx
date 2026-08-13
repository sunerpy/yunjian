/**
 * 检索结果列表。
 *
 * 每一组渲染成一行。异说标注只在该组确实按 `work_group` 判定过时才出现，
 * 判据在 `data/collapse.ts` 的 `variantAnnotation`。
 */

import type { CollapsedGroup } from "../data/collapse";
import { variantAnnotation } from "../data/collapse";
import Highlight from "./Highlight";

export interface ResultListProps {
  groups: CollapsedGroup[];
  onOpen: (poemId: string) => void;
}

export default function ResultList({ groups, onOpen }: ResultListProps) {
  if (groups.length === 0) {
    return (
      <p className="result-list__empty" data-testid="result-empty">
        没有命中。
      </p>
    );
  }

  return (
    <ul className="result-list" data-testid="result-list">
      {groups.map((group) => {
        const annotation = variantAnnotation(group);
        const row = group.primary;
        return (
          <li
            className="result-row"
            key={`${row.poem_id}:${group.variants.length}`}
            data-testid="result-row"
            data-work-group={row.work_group ?? ""}
            data-variant-count={group.variants.length}>
            <button
              type="button"
              className="result-row__open"
              onClick={() => {
                onOpen(row.poem_id);
              }}>
              <span className="result-row__title">{row.title}</span>
              <span className="result-row__meta">
                {row.dynasty} · {row.author}
                {row.genre === null ? "" : ` · ${row.genre}`}
              </span>
              <span className="result-row__snippet">
                <Highlight snippet={row.snippet} />
              </span>
            </button>
            {annotation !== null && (
              // 异说标注刻意是列表行的一部分而不是一个可点开的抽屉：
              // 用户要看到「这里有分歧」而不必先发现一个折叠箭头。
              // 具体每一种归属及其出处在详情页列出——那里才有 source_locator。
              <p className="result-row__variants" data-testid="variant-annotation">
                {annotation}
              </p>
            )}
          </li>
        );
      })}
    </ul>
  );
}
