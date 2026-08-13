/**
 * 异说：同一正文的每一种归属及其出处。
 *
 * # 为什么每一条都要带出处
 *
 * `work_group` 是**不含作者**的分组键，刻意如此设计正是为了让归属冲突可被检测
 * （`crates/yunjian-corpus/src/model.rs:401-405`）。检索页把重出折成一行加「另见 N 处异说」，
 * 而这里是那句标注的兑现处：逐条列出作者、朝代、题目，以及**它出自哪个来源的哪个 revision**。
 * 少了出处，这个列表就只是一句「有人说是别人写的」，用户无从判断该信哪一个。
 *
 * 本篇自己也是一条归属，排在第一位。只列 `work_group_siblings` 会让用户看到 N-1 种说法，
 * 却看不到「当前显示的是哪一种」。
 */

import type { Attribution, PoemRecord, Provenance } from "../contracts/core";

export interface AttributionPanelProps {
  poem: PoemRecord;
  provenance: Provenance;
  siblings: Attribution[];
  /** 同一正文挂了多个不同作者时为真。仅影响提示语强度，不影响列表内容。 */
  conflicting: boolean;
}

interface Entry {
  key: string;
  author: string;
  dynasty: string;
  title: string;
  sourceLocator: string;
  source: string;
  revision: string;
  current: boolean;
}

export default function AttributionPanel({
  poem,
  provenance,
  siblings,
  conflicting,
}: AttributionPanelProps) {
  const entries: Entry[] = [
    {
      key: poem.stable_id,
      author: poem.author,
      dynasty: poem.dynasty.raw,
      title: poem.title,
      sourceLocator: provenance.source_locator,
      source: provenance.source,
      revision: provenance.revision,
      current: true,
    },
    ...siblings.map((sibling) => ({
      key: sibling.stable_id,
      author: sibling.author,
      dynasty: sibling.dynasty.raw,
      title: sibling.title,
      sourceLocator: sibling.source_locator,
      source: sibling.provenance_source,
      revision: sibling.provenance_revision,
      current: false,
    })),
  ];

  if (siblings.length === 0) {
    return null;
  }

  return (
    <section
      className="sourced-block sourced-block--attribution"
      data-provenance="sourced"
      data-testid="poem-attributions"
      aria-label="异说与归属">
      <header className="sourced-block__head">
        <h2 className="sourced-block__title">异说与归属</h2>
        <p className="sourced-block__byline">
          {conflicting
            ? `同一正文在语料里挂了不同作者，共 ${entries.length} 种说法，逐条列出出处，不替你择一。`
            : `同一正文在语料里另有 ${siblings.length} 处收录，逐条列出出处。`}
        </p>
      </header>

      <ul className="attribution-list">
        {entries.map((entry) => (
          <li
            className={entry.current ? "attribution attribution--current" : "attribution"}
            data-testid="attribution-entry"
            data-stable-id={entry.key}
            key={entry.key}>
            <p className="attribution__who">
              {entry.dynasty} · {entry.author}《{entry.title}》
              {entry.current && <span className="attribution__flag">当前显示</span>}
            </p>
            <p className="attribution__source">
              {entry.source} @ {entry.revision} · {entry.sourceLocator}
            </p>
          </li>
        ))}
      </ul>
    </section>
  );
}
