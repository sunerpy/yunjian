/**
 * 原文，可选平仄标注与可选拼音注音层。
 *
 * 这是**考据材料**：容器带 `data-provenance="sourced"`，样式用 `sourced-block` 系列。
 * 与 AI 面板的容器类刻意不同名——两者互换会立刻被快照测试拦下。
 *
 * 平仄标注默认关闭，因为它把一首诗从「读」变成「析」，不该是打开详情页的第一眼。
 * 未知位置显示为 `unknown` 对应的记号而不是留空：留空看起来像「这个字没有平仄」，
 * 而真相是「我们的韵书里查不到它」。同一条道理贯穿注音层的四档。
 *
 * # 两层分居正文两侧，这是构造而不是约定
 *
 * 拼音在字**上方**（ruby），平仄在字**下方**（正文之后的独立一行）。两者永远拿不到同一侧，
 * 因为它们根本不是同一个 DOM 位置——不需要一条「请不要把平仄也放上面」的约定来维持。
 *
 * # 为什么注音打开时正文换一套排布
 *
 * 关闭注音时正文是**一整串文本**，与本功能之前完全一致，一个字节都没动。
 * 打开注音时正文改成逐字定宽格，原因是平仄那一行只为**内容字**建格
 * （`content_chars` 丢掉标点，见 `crates/yunjian-core/src/text.rs:46`），
 * 而正文里是带标点的。两边格数不同，靠字距凑出来的对齐一遇标点就散。
 * 所以注音模式下两层都逐**字符位**建格、共用同一个 `--poem-char-advance`，
 * 标点位在平仄那一行留空格——对齐由格数相同保证，而不是由两处各自算出来的宽度恰好相等保证。
 *
 * 平仄格与内容字的配对**按字形核对**（`tone.character === cell.character`）而不是按下标推算：
 * 两侧「什么算内容字」的判据不在同一个 crate 里，哪天分叉了按下标推算会整行错位一格，
 * 而按字形核对只会少标一个字。
 */

import { useState } from "react";
import type {
  AnnotatedLine,
  PoemAnnotation,
  PoemRecord,
  Reading,
  Tone,
  ToneAnnotation,
} from "../contracts/core";

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

/**
 * 无数据那一档在格内显示的记号。
 *
 * 短横不是一个读音，所以用它并不违反「不造占位读音」；完整措辞在 `title` 里，并由正文
 * 下方那条覆盖说明逐字讲明它的含义。为什么不直接写「暂无注音」四个字：那要缩到 7.2px
 * 才塞得进一格（目视 QA 实测），并且铺满一行之后注音层比留空更难读。
 */
const ABSENT_MARK = "—";

/**
 * 有据破读的记号。
 *
 * 与短横同一条道理：界面上出现的每一个记号都要在正文下方那条说明里讲明含义，否则它只是
 * 一个「看得见但不知道是什么」的装饰。目视 QA 指出过这一点——当时短横有说明而这个上标没有。
 */
const ATTESTED_MARK = "据";

/** 多候选存疑的记号。同样要在正文下方那条说明里讲明含义。 */
const UNCERTAIN_MARK = "存疑";

/** 四档各自的无障碍说明。措辞就是诚实边界本身，不能互换。 */
const READING_TITLE: Record<Reading["kind"], string> = {
  attested: "有据破读",
  generic: "通用拼音，不是古典语境裁决",
  uncertain: "此处读音存疑",
  absent: "暂无注音",
};

/** 一格：字、读音处境、以及按字形配对上的平仄。 */
type Column = {
  character: string;
  reading: Reading | null;
  tone: Tone | null;
};

/**
 * 把一行拼成逐字符位的格。
 *
 * 平仄格只覆盖内容字，所以这里用一个游标去认领：字形对得上才消耗一格，对不上就留空。
 */
function columnsOf(
  line: AnnotatedLine,
  tones: ToneAnnotation["lines"][number] | undefined,
): Column[] {
  let pending = tones?.cells ?? [];
  return line.cells.map((cell) => {
    const next = pending[0];
    const claimed = next !== undefined && next.character === cell.character;
    if (claimed) {
      pending = pending.slice(1);
    }
    return {
      character: cell.character,
      reading: cell.reading,
      tone: claimed && next !== undefined ? next.tone : null,
    };
  });
}

export interface OriginalTextProps {
  poem: PoemRecord;
  tones: ToneAnnotation;
  showTones: boolean;
  /** 整首注音。`null` 表示还没取回或取不到——此时注音层不显示，也不编造。 */
  annotation: PoemAnnotation | null;
  showPinyin: boolean;
  /**
   * 无提示主动回忆。
   *
   * 为真时两层**一律默认隐藏**，无论用户此前把开关持久化成了什么：支架在这一步的作用
   * 是被主动求助的，而不是默认摆在眼前。
   */
  recall?: boolean;
  /**
   * 用户主动揭示了某一层。
   *
   * **入参只有层名，没有任何表示对错的位置。** 揭示要被记为支架使用，但绝不自动判错，
   * 而「不能判错」这件事在这里是类型上的——这个回调没有可以塞进一个判定的地方。
   */
  onReveal?: (layer: "pinyin" | "tones") => void;
}

export default function OriginalText({
  poem,
  tones,
  showTones,
  annotation,
  showPinyin,
  recall = false,
  onReveal,
}: OriginalTextProps) {
  const [revealed, setRevealed] = useState<Set<"pinyin" | "tones">>(new Set());
  const [expanded, setExpanded] = useState<string | null>(null);

  const visible = (layer: "pinyin" | "tones", requested: boolean): boolean =>
    requested && (!recall || revealed.has(layer));

  const reveal = (layer: "pinyin" | "tones") => {
    setRevealed((current) => new Set(current).add(layer));
    onReveal?.(layer);
  };

  const tonesOn = visible("tones", showTones);
  const pinyinOn = visible("pinyin", showPinyin) && annotation !== null;

  const lines = poem.body.split("\n").filter((line) => line.trim() !== "");
  const toneLines = new Map(tones.lines.map((line) => [line.line_index, line]));
  const annotatedLines = new Map((annotation?.lines ?? []).map((line) => [line.line_index, line]));

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

      {recall && (
        <div className="poem-body__recall" data-testid="recall-scaffold">
          <p className="poem-body__recall-note">
            无提示主动回忆：注音与平仄默认隐藏。揭示会记为支架使用，不影响判定。
          </p>
          {(["pinyin", "tones"] as const)
            .filter((layer) => !revealed.has(layer))
            .map((layer) => (
              <button
                type="button"
                className="poem-body__reveal"
                onClick={() => {
                  reveal(layer);
                }}
                data-testid={`reveal-${layer}`}
                key={layer}>
                {layer === "pinyin" ? "揭示拼音" : "揭示平仄"}
              </button>
            ))}
        </div>
      )}

      <div className="poem-body" data-pinyin={pinyinOn ? "on" : "off"} data-testid="poem-body">
        {lines.map((line, index) => {
          const toneLine = tonesOn ? toneLines.get(index) : undefined;
          const annotatedLine = pinyinOn ? annotatedLines.get(index) : undefined;

          if (annotatedLine === undefined) {
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
          }

          const columns = columnsOf(annotatedLine, toneLine);
          return (
            <p className="poem-body__line" key={index}>
              <span className="poem-body__text">
                {columns.map((column, cellIndex) => {
                  const key = `${index}:${cellIndex}`;
                  return (
                    <ruby className="poem-body__ruby" key={cellIndex}>
                      <span className="poem-body__char">{column.character}</span>
                      <rt className="poem-body__rt" data-reading={column.reading?.kind ?? "none"}>
                        {column.reading === null ? (
                          ""
                        ) : (
                          <ReadingMark
                            reading={column.reading}
                            expanded={expanded === key}
                            onToggle={() => {
                              setExpanded((current) => (current === key ? null : key));
                            }}
                          />
                        )}
                      </rt>
                    </ruby>
                  );
                })}
              </span>

              {toneLine !== undefined && (
                <span className="poem-body__tones" data-testid="tone-row">
                  {columns.map((column, cellIndex) => (
                    <span
                      className="poem-body__tone"
                      data-tone={column.tone ?? "none"}
                      {...(column.tone === null
                        ? {}
                        : { title: `${column.character}：${TONE_TITLE[column.tone]}` })}
                      key={cellIndex}>
                      {column.tone === null ? "" : TONE_MARK[column.tone]}
                    </span>
                  ))}
                </span>
              )}

              {columns.map((column, cellIndex) => {
                const key = `${index}:${cellIndex}`;
                if (expanded !== key || column.reading?.kind !== "uncertain") {
                  return null;
                }
                return (
                  <span
                    className="poem-body__uncertain-detail"
                    data-testid="uncertain-detail"
                    key={cellIndex}>
                    {column.reading.candidates.join(" / ")}——此处读音存疑
                  </span>
                );
              })}
            </p>
          );
        })}
      </div>

      {pinyinOn && annotation !== null && (
        <p className="sourced-block__caveat" data-testid="pinyin-coverage">
          本首注音：有据破读 {annotation.coverage.attested} 字（拼音右上标「{ATTESTED_MARK}
          」）、通用拼音 {annotation.coverage.generic} 字、存疑 {annotation.coverage.uncertain}{" "}
          字（标「{UNCERTAIN_MARK}」，点开看并列候选）、暂无注音 {annotation.coverage.absent}{" "}
          字（格内标为「{ABSENT_MARK}」），共 {coverageTotal(annotation)}{" "}
          个内容字。破读依据只覆盖随包名册内的作品，名册之外一律按通用候选处理。
        </p>
      )}

      {tonesOn && tones.unknown_count > 0 && (
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

function coverageTotal(annotation: PoemAnnotation): number {
  const { attested, generic, uncertain, absent } = annotation.coverage;
  return attested + generic + uncertain + absent;
}

/**
 * 一格里显示什么。
 *
 * 四档的**可见形态互不相同，且都不靠颜色**：有据带一个「据」字上标、通用光有拼音、
 * 存疑是一个可点的标记、无数据直接写「暂无注音」。颜色只是加强，不是唯一编码——
 * 这是无障碍那条要求的落地方式。
 */
function ReadingMark({
  reading,
  expanded,
  onToggle,
}: {
  reading: Reading;
  expanded: boolean;
  onToggle: () => void;
}) {
  if (reading.kind === "attested") {
    return (
      <span className="poem-body__reading" title={`${READING_TITLE.attested}：${reading.evidence}`}>
        {reading.pinyin}
        <sup className="poem-body__attested" data-confidence={reading.confidence}>
          {ATTESTED_MARK}
        </sup>
      </span>
    );
  }

  if (reading.kind === "generic") {
    return (
      <span className="poem-body__reading" title={READING_TITLE.generic}>
        {reading.pinyin}
      </span>
    );
  }

  if (reading.kind === "uncertain") {
    return (
      <button
        type="button"
        className="poem-body__uncertain"
        aria-expanded={expanded}
        title={READING_TITLE.uncertain}
        onClick={onToggle}
        data-testid="uncertain-mark">
        {UNCERTAIN_MARK}
      </button>
    );
  }

  return (
    <span className="poem-body__absent" title={READING_TITLE.absent}>
      {ABSENT_MARK}
    </span>
  );
}
