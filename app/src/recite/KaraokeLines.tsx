/**
 * 逐字高亮：karaoke 式的音步推进，叠加偏置假设给出的已匹配前缀。
 *
 * # 两种高亮在视觉上必须可分
 *
 * 它们的证据强度不同（见 `voiceHighlight.ts` 的模块文档），所以不能画成同一种样式：
 * karaoke 那一段是确定的（时间戳来自拼接算术），已匹配前缀那一段只是「识别器认到这里」
 * 的提示，而识别器的字错率实测 77.01%。画成一样会让用户把后者读成前者。
 *
 * # 组件不判断任何事
 *
 * 当前音步下标与已匹配长度都由调用方算好传进来，本组件只负责摆 DOM 并把每一段的含义挂到
 * `aria-label` 上——读屏用户与色觉异常用户不能靠颜色分辨哪一段是什么。
 */

import type { FootMark } from "../contracts/voice";
import { contentChars } from "./voiceHighlight";

export interface KaraokeLinesProps {
  /** 逐行文本，未去标点。 */
  lines: readonly string[];
  /** 逐音步时间戳。空数组表示还没示范过，此时只画文本。 */
  marks: readonly FootMark[];
  /** 当前音步在 `marks` 里的下标；`-1` 表示此刻不在任何音步内。 */
  activeMark: number;
  /** 正在复诵第几行；`-1` 表示不在复诵。 */
  listeningLine: number;
  /** 当前行已匹配的字数，取自偏置假设。 */
  matchedChars: number;
}

/**
 * 一个音步在它所在行里的起始字下标。
 *
 * `FootMark` 只带文本而不带字下标，所以按**同一行内在它之前的音步字数之和**定位。这依赖
 * 一条已经被保证的性质：同一行的音步按 `index_in_line` 顺序首尾相接且不重叠
 * （`prosody::cut`）。按文本 `indexOf` 定位是另一条路，而那条路在一行里出现重复词时会
 * 定位到错误的位置。
 */
export function footStart(marks: readonly FootMark[], foot: FootMark): number {
  return marks
    .filter((mark) => mark.line === foot.line && mark.index_in_line < foot.index_in_line)
    .reduce((total, mark) => total + [...mark.text].length, 0);
}

export default function KaraokeLines({
  lines,
  marks,
  activeMark,
  listeningLine,
  matchedChars,
}: KaraokeLinesProps) {
  const active = marks[activeMark];
  const activeStart = active === undefined ? 0 : footStart(marks, active);
  const activeLength = active === undefined ? 0 : [...active.text].length;

  return (
    <ol className="voice-lines" data-testid="voice-lines">
      {lines.map((text, line) => {
        const chars = contentChars(text);
        const listening = line === listeningLine;
        return (
          <li
            key={line}
            className="voice-lines__line"
            data-line={line}
            data-listening={listening ? "true" : undefined}>
            {chars.map((character, index) => {
              const karaoke =
                active !== undefined &&
                active.line === line &&
                index >= activeStart &&
                index < activeStart + activeLength;
              const matched = listening && index < matchedChars;
              return (
                <span
                  key={index}
                  className="voice-char"
                  data-testid={`voice-char-${line}-${index}`}
                  data-karaoke={karaoke ? "true" : undefined}
                  data-matched={matched ? "true" : undefined}
                  aria-label={charLabel(character, karaoke, matched)}>
                  {character}
                </span>
              );
            })}
          </li>
        );
      })}
    </ol>
  );
}

function charLabel(character: string, karaoke: boolean, matched: boolean): string {
  if (karaoke) {
    return `${character}（示范正在读这一音步）`;
  }
  if (matched) {
    return `${character}（识别器认到这里，仅供参考）`;
  }
  return character;
}
