/**
 * 历代集评。
 *
 * # 这个组件的主要职责是**拒绝渲染**
 *
 * 每条集评的出处内联显示在它自己下方，格式与 CLI 已有的人类可读输出一致
 * （`crates/yunjian-cli/src/output.rs:322-330`）。缺出处的条目**不显示正文**，
 * 而是显示一条点名编号与缺失字段的错误。
 *
 * 为什么是错误而不是空态：无法复核的文言评语一旦排成正常样式，就与带卷次页码的引文混为一谈，
 * 而「逐条注明出处」正是这部分内容能合法随包分发的全部理由。空文本更糟——它看起来像
 * 「这条没什么可说的」，实际是「我们不知道这话是谁说的」。
 *
 * 校验逻辑与缺失字段命名在 `data/commentary.ts`，那里对齐了 core 的 `missing_field` 取值。
 */

import type { CommentaryEntry } from "../contracts/core";
import { checkCommentaries, citationLine, missingCitationMessage } from "../data/commentary";

export interface CommentaryListProps {
  commentaries: CommentaryEntry[];
}

export default function CommentaryList({ commentaries }: CommentaryListProps) {
  const checks = checkCommentaries(commentaries);

  return (
    <section
      className="sourced-block sourced-block--commentary"
      data-provenance="sourced"
      data-testid="poem-commentary"
      aria-label="历代集评">
      <header className="sourced-block__head">
        <h2 className="sourced-block__title">历代集评</h2>
        <p className="sourced-block__byline">前现代评语，公有领域，逐条注明出处</p>
      </header>

      {checks.length === 0 ? (
        <p className="sourced-block__caveat" data-testid="commentary-empty">
          本篇暂无随包集评。
        </p>
      ) : (
        <ol className="commentary-list">
          {checks.map((check, index) =>
            check.kind === "valid" ? (
              <li
                className="commentary"
                data-testid="commentary-entry"
                data-commentary-id={check.entry.id}
                key={check.entry.id}>
                <p className="commentary__text">{check.entry.text}</p>
                <p className="commentary__citation" data-testid="commentary-citation">
                  {citationLine(check.entry.citation)}
                </p>
              </li>
            ) : (
              <li
                className="commentary commentary--error"
                data-testid="commentary-error"
                data-commentary-id={check.id}
                key={`${check.id}:${index}`}>
                <p className="commentary__error" role="alert">
                  {missingCitationMessage(check)}
                </p>
              </li>
            ),
          )}
        </ol>
      )}
    </section>
  );
}
