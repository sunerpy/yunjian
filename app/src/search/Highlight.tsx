/**
 * 命中高亮。
 *
 * # 一个必须按码点切的理由
 *
 * `HighlightRange` 的下标是 **Unicode 字符下标**，core 的注释明确写了这一点
 * （`crates/yunjian-core/src/search/text.rs:32-34`：「使用 Unicode 字符下标而不是 UTF-8
 * 字节下标」）。而 JS 的 `String.prototype.slice` 按 **UTF-16 码元**切。
 * 对基本多文种平面内的汉字两者数值相同，所以绝大多数诗都看不出差别——
 * 但对增补平面的字（部分生僻字与异体字，如 U+20000 以上的扩展 B 区汉字，一个字符占两个码元）
 * 会整段错位。`Array.from` 按码点迭代，因此下面先拆成码点数组再取区间。
 *
 * 这不是假想问题：语料里有繁体与异体字，而扩展区汉字正是异体字最集中的地方。
 */

import type { HighlightedSnippet, HighlightRange } from "../contracts/core";

interface Piece {
  text: string;
  highlighted: boolean;
}

/**
 * 把一段文本按高亮区间切成交替的片段。
 *
 * 区间先排序再合并重叠部分：后端目前只给不重叠的区间，但一个重叠的区间会让朴素实现产出
 * 重复字符——那是「文字凭空多出来」，比高亮位置不对严重得多。
 */
export function splitByHighlights(snippet: HighlightedSnippet): Piece[] {
  const codePoints = Array.from(snippet.text);
  const total = codePoints.length;

  const ranges: HighlightRange[] = [...snippet.highlights]
    .map((range) => ({
      start: Math.max(0, Math.min(range.start, total)),
      end: Math.max(0, Math.min(range.end, total)),
    }))
    .filter((range) => range.end > range.start)
    .sort((left, right) => left.start - right.start);

  const merged: HighlightRange[] = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last !== undefined && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }

  const pieces: Piece[] = [];
  let cursor = 0;
  for (const range of merged) {
    if (range.start > cursor) {
      pieces.push({ text: codePoints.slice(cursor, range.start).join(""), highlighted: false });
    }
    pieces.push({ text: codePoints.slice(range.start, range.end).join(""), highlighted: true });
    cursor = range.end;
  }
  if (cursor < total) {
    pieces.push({ text: codePoints.slice(cursor).join(""), highlighted: false });
  }
  return pieces;
}

export interface HighlightProps {
  snippet: HighlightedSnippet;
}

export default function Highlight({ snippet }: HighlightProps) {
  const pieces = splitByHighlights(snippet);
  return (
    <span className="highlight">
      {pieces.map((piece, index) =>
        piece.highlighted ? (
          // `<mark>` 而不是带背景色的 `<span>`：屏幕阅读器会读出「标记」，
          // 于是「哪几个字命中了」对不看屏幕的用户同样成立。
          <mark className="highlight__hit" key={index}>
            {piece.text}
          </mark>
        ) : (
          <span key={index}>{piece.text}</span>
        ),
      )}
    </span>
  );
}
