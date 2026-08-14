import { type FormEvent, useState } from "react";
import type {
  DictionaryCharacter,
  DictionaryLookup,
  DictionaryPronunciation,
  PoyinConfidence,
  RhymeBook,
  RhymeTone,
} from "../contracts/core";
import type { DictionaryPort } from "../data/ports";

interface DictionaryPanelProps {
  port: DictionaryPort;
}

const BOOK_LABELS: Record<RhymeBook, string> = {
  pingshui: "平水韵",
  cilin: "词林正韵",
  xinyun: "中华新韵",
};

const TONE_LABELS: Record<RhymeTone, string> = {
  level: "平",
  rising: "上",
  departing: "去",
  entering: "入",
  oblique: "仄",
};

const CONFIDENCE_LABELS: Record<PoyinConfidence, string> = {
  rhyme_attested: "韵部实证",
  tone_split: "调类分工",
  engine_default: "沿用引擎候选",
};

function pronunciationLabel(pronunciation: DictionaryPronunciation): string {
  switch (pronunciation.kind) {
    case "attested":
      return `${pronunciation.reading} · 有据破读`;
    case "general":
      return `${pronunciation.reading} · 通用读音`;
    case "uncertain":
      return `${pronunciation.candidates.join(" / ")} · 待读音裁决`;
    case "unavailable":
      return "暂无读音数据";
  }
}

function CharacterFacts({ entry }: { entry: DictionaryCharacter }) {
  return (
    <article
      data-testid={`dictionary-character-${entry.character}`}
      className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
      <header className="flex items-baseline gap-4 border-b border-[var(--color-border)] pb-3">
        <span className="font-serif text-2xl text-[var(--color-text)]">{entry.character}</span>
        <span className="text-sm text-[var(--color-text-muted)]">
          {pronunciationLabel(entry.pronunciation)}
        </span>
        {entry.character !== entry.normalized && (
          <span className="ml-auto text-xs text-[var(--color-citation-text)]">
            规范字形：{entry.normalized}
          </span>
        )}
      </header>

      {entry.variants.length > 0 && (
        <p className="text-sm text-[var(--color-text-muted)]">
          异体关系：
          {entry.variants
            .map((relation) => `${relation.variant} → ${relation.normalized}`)
            .join("、")}
        </p>
      )}

      {entry.poyin !== null && (
        <div className="mt-3 border-l-2 border-[var(--color-sourced-rule)] pl-3 font-serif text-sm leading-7">
          <p className="m-0">
            <strong>{CONFIDENCE_LABELS[entry.poyin.confidence]}</strong> · {entry.poyin.evidence}
          </p>
          <code className="font-mono text-xs text-[var(--color-citation-text)]">
            {entry.poyin.source_locator}
          </code>
        </div>
      )}

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        <section
          data-source-layer="rhyme"
          className="border-l-4 border-solid border-[var(--color-sourced-rule)] bg-[var(--color-sourced-surface)] p-3 font-serif">
          <h3 className="m-0 text-sm">韵书事实</h3>
          {entry.rhymes.length === 0 ? (
            <p className="mb-0 text-xs text-[var(--color-text-muted)]">随包韵书未收此字。</p>
          ) : (
            <ul className="mb-0 list-none space-y-3 p-0 text-sm">
              {entry.rhymes.map((fact) => (
                <li key={fact.source_locator}>
                  <strong>{BOOK_LABELS[fact.book]}</strong> · {fact.tone_raw} {fact.rhyme_group} ·
                  {TONE_LABELS[fact.tone]}
                  <code className="mt-1 block break-all font-mono text-xs text-[var(--color-citation-text)]">
                    {fact.source_locator}
                  </code>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section
          data-source-layer="public-lexicon"
          className="border border-dotted border-[var(--color-citation-text)] bg-[var(--color-surface-raised)] p-3 font-serif">
          <h3 className="m-0 text-sm">公有领域字书</h3>
          <p className="mb-0 text-xs leading-6 text-[var(--color-text-muted)]">
            v1 尚未接入字书释义；不以现代辞书或推测填空。
          </p>
        </section>

        <section
          data-source-layer="ai"
          className="border-2 border-dashed border-[var(--color-ai-border)] bg-[var(--color-ai-surface)] p-3 font-sans text-[var(--color-ai-text)]">
          <h3 className="m-0 text-sm">AI 释义</h3>
          <p className="mb-0 text-xs leading-6">
            未生成。AI 内容必须单独标注，且不参与读音、异体或韵部裁决。
          </p>
        </section>
      </div>
      <p className="mb-0 mt-3 text-xs leading-6 text-[var(--color-text-muted)]">
        韵书记录声部和韵部，不等于现代释义，也不能单独推出当前拼音。
      </p>
    </article>
  );
}

export default function DictionaryPanel({ port }: DictionaryPanelProps) {
  const [query, setQuery] = useState("斜阳");
  const [context, setContext] = useState("远上寒山石径斜");
  const [lookup, setLookup] = useState<DictionaryLookup | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      setLookup(
        await port.lookupDictionary({ query, context: context.trim() === "" ? null : context }),
      );
    } catch (reason) {
      setLookup(null);
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setPending(false);
    }
  }

  return (
    <section
      data-testid="dictionary-panel"
      data-shell-chrome=""
      className="mx-auto max-w-[64rem] px-6 py-8">
      <header className="mb-6 grid gap-2">
        <p className="m-0 font-mono text-xs tracking-[0.18em] text-[var(--color-citation-text)]">
          LOCAL · VERIFIABLE · 1–2 字
        </p>
        <h1 className="m-0 font-serif text-2xl">内置字典</h1>
        <p className="m-0 max-w-[46rem] text-sm leading-7 text-[var(--color-text-muted)]">
          查异体、破读与随包韵书原始事实。双字作为一次请求，但只按顺序返回逐字事实，不合成词义。
        </p>
      </header>

      <form
        onSubmit={submit}
        className="grid gap-3 rounded-lg bg-[var(--color-surface)] p-4 md:grid-cols-[10rem_1fr_auto]">
        <label className="grid gap-1 text-xs text-[var(--color-text-muted)]">
          查询字词
          <input
            data-testid="dictionary-query"
            value={query}
            maxLength={2}
            onChange={(event) => {
              setQuery(event.currentTarget.value);
            }}
            className="min-w-0 rounded-md border border-solid border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 font-serif text-lg text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
          />
        </label>
        <label className="grid gap-1 text-xs text-[var(--color-text-muted)]">
          原句语境（可选）
          <input
            data-testid="dictionary-context"
            value={context}
            onChange={(event) => {
              setContext(event.currentTarget.value);
            }}
            className="min-w-0 rounded-md border border-solid border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 font-serif text-base text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
          />
        </label>
        <button
          type="submit"
          data-testid="dictionary-submit"
          disabled={pending || Array.from(query.trim()).length === 0}
          className="self-end rounded-md bg-[var(--color-accent)] px-4 py-2 text-sm text-[var(--color-surface)] enabled:cursor-pointer disabled:cursor-not-allowed disabled:opacity-50">
          {pending ? "查证中…" : "查证"}
        </button>
      </form>

      {error !== null && (
        <p
          role="alert"
          className="border border-solid border-[var(--color-error-border)] bg-[var(--color-error-surface)] p-3 text-sm text-[var(--color-error-text)]">
          {error}
        </p>
      )}

      {lookup === null && error === null && (
        <div
          data-testid="dictionary-empty"
          className="mt-6 border border-dashed border-[var(--color-border)] p-8 text-center text-sm text-[var(--color-text-muted)]">
          输入一至二字开始查证；默认示例可展示双字逐字结果与破读依据。
        </div>
      )}

      {lookup !== null && (
        <div data-testid="dictionary-results" className="mt-6 grid gap-4">
          <p className="m-0 text-xs text-[var(--color-text-muted)]">
            {lookup.kind === "character_sequence" ? "双字请求 · 逐字事实" : "单字事实"} · 共
            {lookup.characters.length} 字
          </p>
          {lookup.characters.map((entry) => (
            <CharacterFacts key={`${lookup.query}-${entry.character}`} entry={entry} />
          ))}
        </div>
      )}
    </section>
  );
}
