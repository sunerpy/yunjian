/**
 * 原文，可选平仄标注。
 *
 * 这是**考据材料**：容器带 `data-provenance="sourced"`，样式用 `sourced-block` 系列。
 * 与 AI 面板的容器类刻意不同名——两者互换会立刻被快照测试拦下。
 *
 * 平仄标注默认关闭，因为它把一首诗从「读」变成「析」，不该是打开详情页的第一眼。
 * 未知位置显示为 `unknown` 对应的记号而不是留空：留空看起来像「这个字没有平仄」，
 * 而真相是「我们的韵书里查不到它」。
 */

import type { PoemRecord, Tone, ToneAnnotation } from "../contracts/core";

const TONE_MARK: Record<Tone, string> = {
  level: "平",
  oblique: "仄",
  either: "中",
  unknown: "？",
};

const TONE_TITLE: Record<Tone, string> = {
  level: "平声",
  oblique: "仄声",
  either: "平仄两可",
  unknown: "韵书未收，未知",
};

export interface OriginalTextProps {
  poem: PoemRecord;
  tones: ToneAnnotation;
  showTones: boolean;
}

export default function OriginalText({ poem, tones, showTones }: OriginalTextProps) {
  const lines = poem.body.split("\n").filter((line) => line.trim() !== "");
  const toneLines = new Map(tones.lines.map((line) => [line.line_index, line]));

  return (
    <section
      className="sourced-block sourced-block--original"
      data-provenance="sourced"
      data-testid="poem-original"
      aria-label="原文">
      <header className="sourced-block__head">
        <h2 className="sourced-block__title">
          {poem.title}
          {poem.ci_tune === null ? "" : `（${poem.ci_tune}）`}
        </h2>
        <p className="sourced-block__byline">
          {poem.dynasty.raw} · {poem.author}
        </p>
      </header>

      <div className="poem-body">
        {lines.map((line, index) => {
          const toneLine = showTones ? toneLines.get(index) : undefined;
          return (
            <p className="poem-body__line" key={index}>
              <span className="poem-body__text">{line}</span>
              {toneLine !== undefined && (
                <span className="poem-body__tones" data-testid="tone-row">
                  {toneLine.cells.map((cell, cellIndex) => (
                    <span
                      className="poem-body__tone"
                      data-tone={cell.tone}
                      title={`${cell.character}：${TONE_TITLE[cell.tone]}`}
                      key={cellIndex}>
                      {TONE_MARK[cell.tone]}
                    </span>
                  ))}
                </span>
              )}
            </p>
          );
        })}
      </div>

      {showTones && tones.unknown_count > 0 && (
        <p className="sourced-block__caveat" data-testid="tone-unknown-note">
          有 {tones.unknown_count} 字在
          {tones.book === "pingshui" ? "平水韵" : tones.book === "cilin" ? "词林正韵" : "中华新韵"}
          里查不到，标为「？」而不是猜一个。
        </p>
      )}

      {poem.body_original !== poem.body && (
        <details className="sourced-block__aside">
          <summary>上游原字形</summary>
          <div className="poem-body poem-body--original">
            {poem.body_original
              .split("\n")
              .filter((line) => line.trim() !== "")
              .map((line, index) => (
                <p className="poem-body__line" key={index}>
                  <span className="poem-body__text">{line}</span>
                </p>
              ))}
          </div>
        </details>
      )}
    </section>
  );
}
