/**
 * 逐韵书的韵部归属。
 *
 * 三处如实呈现：
 *
 * 1. **按韵书分行**，因为同一首诗在平水韵与词林正韵下可以归入不同韵部，合成一行会丢掉这件事。
 * 2. **可信度照实显示**。`resolved_by_vote`（多音字按同韵脚投票定的）与 `unambiguous`
 *    （本来就只有一读）不是一回事，把两者都显示成「平水韵 · 东」等于把推断说成事实。
 * 3. **`xinyun` 目前是空槽**。中华新韵因许可未核实被扣留，核心会返回 `RhymeBookUnavailable`
 *    （见 `.omo/notepads/yunjian/problems.md`）。所以这里不为它预留一行「暂无数据」——
 *    真的没有就一条都不出现，而不是显示一个永远填不上的空位。
 */

import type {
  RhymeBook,
  RhymeConfidence,
  RhymeGroupMembership,
  RhymeTone,
} from "../contracts/core";

const BOOK_LABEL: Record<RhymeBook, string> = {
  pingshui: "平水韵",
  cilin: "词林正韵",
  xinyun: "中华新韵",
};

const TONE_LABEL: Record<RhymeTone, string> = {
  level: "平声",
  rising: "上声",
  departing: "去声",
  entering: "入声",
  oblique: "仄声",
};

const CONFIDENCE_LABEL: Record<RhymeConfidence, string> = {
  unambiguous: "唯一读音",
  resolved_by_vote: "多音字，按同篇韵脚推定",
  unresolved: "多音字，未能推定",
};

export interface RhymePanelProps {
  groups: RhymeGroupMembership[];
}

export default function RhymePanel({ groups }: RhymePanelProps) {
  if (groups.length === 0) {
    return null;
  }

  return (
    <section
      className="sourced-block sourced-block--rhyme"
      data-provenance="sourced"
      data-testid="poem-rhyme"
      aria-label="韵部">
      <header className="sourced-block__head">
        <h2 className="sourced-block__title">韵部</h2>
      </header>
      <ul className="rhyme-list">
        {groups.map((group, index) => (
          <li
            className="rhyme"
            data-testid="rhyme-entry"
            key={`${group.book}:${group.group}:${index}`}>
            <span className="rhyme__book">{BOOK_LABEL[group.book]}</span>
            <span className="rhyme__group">{group.group}</span>
            <span className="rhyme__tone">{TONE_LABEL[group.tone]}</span>
            <span className="rhyme__confidence" data-confidence={group.confidence}>
              {CONFIDENCE_LABEL[group.confidence]}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
