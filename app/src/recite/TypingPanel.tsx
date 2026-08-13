/**
 * 打字面板：提示文本、作答输入、提交。
 *
 * # 三处刻意的约束
 *
 * 1. **空作答提交按钮禁用。** 按零分记账会往复习历史写一条用户没做过的记录，
 *    事后无法撤回——命令行那边同样在空作答时报用法错误而不是记零分
 *    （`learnings.md` 的 todo 58 第 3 条）。
 * 2. **提示文本原样显示内核给的串**，被遮处是内核填的全角下划线 `＿`。
 *    不用 CSS 的 `text-decoration` 伪造下划线：伪造出来的空在复制粘贴时会消失，
 *    而用户复制提示去别处对照是一件真实会发生的事。
 * 3. **这里拿不到参考诗文**（`ReciteSession` 类型上就没有那个字段），
 *    所以「把答案显示在提示旁边」不是被规范禁止，而是写不出来。
 */

import type { ReciteSession } from "../contracts/recite";
import { RECITE_MODE_LABEL } from "../contracts/recite";

export interface TypingPanelProps {
  session: ReciteSession;
  answer: string;
  busy: boolean;
  /** 已经提交过作答时为真：此时输入与提交都锁住，一局只评一次。 */
  submitted: boolean;
  onAnswerChange(answer: string): void;
  onSubmit(): void;
}

export default function TypingPanel({
  session,
  answer,
  busy,
  submitted,
  onAnswerChange,
  onSubmit,
}: TypingPanelProps) {
  const empty = answer.trim() === "";

  return (
    <section className="recite-section" aria-label="作答">
      <h2 className="recite-section__title">
        {session.title} — {session.author}（{session.dynasty}）
      </h2>

      <p className="recite-section__note" data-testid="session-mode">
        形态：{RECITE_MODE_LABEL[session.mode]}
        {session.ratio !== undefined && `（比例 ${session.ratio.toFixed(2)}`}
        {session.ratio !== undefined && session.seed !== undefined && `，种子 ${session.seed}`}
        {session.ratio !== undefined && "）"}
        {session.masked_lines !== undefined && `（遮 ${session.masked_lines} 句）`}
        {" · "}
        挖空 {session.hidden_indices.length} 处 · 共 {session.line_count} 句
      </p>

      {session.fallback_reason !== undefined && (
        <p className="recite-fallback" role="alert" data-testid="voice-fallback">
          {session.requested_mode !== undefined &&
            `请求的形态是「${RECITE_MODE_LABEL[session.requested_mode]}」：`}
          {session.fallback_reason}
        </p>
      )}

      <p className="recite-prompt" data-testid="session-prompt">
        {session.prompt}
      </p>

      <div className="recite-field">
        <label className="recite-field__label" htmlFor="recite-answer">
          作答
        </label>
        <textarea
          id="recite-answer"
          className="recite-field__control recite-field__control--answer"
          data-testid="recite-answer"
          value={answer}
          disabled={busy || submitted}
          onChange={(event) => {
            onAnswerChange(event.target.value);
          }}
        />
        <p className="recite-field__hint">
          标点与空白无需照抄：内核在评分前会去掉它们并按语料随包的异体字表归一化，
          与命令行走的是同一条归一化路径。
        </p>
      </div>

      <div className="recite-actions">
        <button
          type="button"
          className="recite-button"
          data-testid="submit-answer"
          disabled={empty || busy || submitted}
          onClick={onSubmit}>
          {submitted ? "已评分" : "提交作答"}
        </button>
        {empty && !submitted && (
          <span className="recite-field__hint" data-testid="empty-answer-hint">
            作答为空时不提交：记一次零分会往复习历史写一条你没做过的记录。
          </span>
        )}
      </div>
    </section>
  );
}
