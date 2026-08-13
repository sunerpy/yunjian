/**
 * 逐字反馈：按内核给的对齐操作列表逐格渲染五类差异标记。
 *
 * # 这个组件不判断任何事
 *
 * 每一格显示什么字、算哪一类、说明写什么，全部由 `opMark.ts` 的 `opCells` 从
 * 内核载荷里读出来。这里只负责把它们摆成 DOM，并把每格的说明挂到 `title` 与
 * `aria-label` 上——读屏用户与色觉异常用户不能靠颜色分辨哪一格是什么。
 *
 * # 顺序是作答顺序，不是参考文本顺序
 *
 * 内核给的 `ops` 按作答过程排列（`align.rs` 的 `Alignment.ops`）。这一点在有回读时
 * 尤其重要：回读那一段在参考文本里是往回跳的，按参考顺序重排会让「他读到这里又回头
 * 重读了一遍」变成「他把这一段读了两次」，而那是两种不同的错误。
 */

import type { ReciteOp } from "../contracts/recite";
import { OP_MARKS, OP_MARK_KINDS, opCells } from "./opMark";

export interface OpFeedbackProps {
  ops: ReciteOp[];
}

export default function OpFeedback({ ops }: OpFeedbackProps) {
  const cells = opCells(ops);
  const differences = cells.filter((cell) => cell.mark.kind !== "normal");

  return (
    <div>
      <p className="recite-ops" data-testid="op-feedback">
        {cells.map((cell, index) => (
          <span
            // 下标做 key：同一个字可能在一首诗里出现多次，字本身不唯一，
            // 而 `ops` 在一次渲染内是不变的，所以下标是稳定的。
            key={index}
            className={`recite-op ${cell.mark.className}`}
            data-op={cell.mark.kind}
            title={cell.description}
            aria-label={cell.description}>
            <span>{cell.text}</span>
            {cell.expected !== undefined && (
              <sup className="recite-op__expected">{cell.expected}</sup>
            )}
            <sup className="recite-op__mark" aria-hidden="true">
              {cell.mark.mark}
            </sup>
          </span>
        ))}
      </p>

      <p className="recite-legend" data-testid="op-legend">
        {OP_MARK_KINDS.map((kind) => (
          <span
            key={kind}
            className={
              `recite-legend__item ${OP_MARKS[kind].className}` +
              // 相符那一项在图例里补一圈中性边框，理由见 `recite.css`。
              (kind === "normal" ? " recite-legend__item--normal" : "")
            }
            data-legend={kind}>
            <span aria-hidden="true">{OP_MARKS[kind].mark}</span>
            <span>{OP_MARKS[kind].label}</span>
          </span>
        ))}
      </p>

      {differences.length === 0 ? (
        <p className="recite-section__note recite-diff--empty" data-testid="no-differences">
          全篇相符，没有差异。
        </p>
      ) : (
        <ul className="recite-diff" data-testid="op-differences">
          {differences.map((cell, index) => (
            <li key={index} className="recite-diff__item">
              <span className={`recite-diff__mark ${cell.mark.className}`} aria-hidden="true">
                {cell.mark.mark}
              </span>
              <span>{cell.description}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
