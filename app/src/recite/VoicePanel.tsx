/**
 * 语音跟读面板：可用性 → 示范 → 复诵 → 结果，失败一律切到打字模式并说清原因。
 *
 * # 状态机的形状就是产品契约
 *
 * ```
 * probing ──不可用──▶ typed（携带那一条独有的原因）
 *    │                    │
 *    │                 缺模型时另给「下载模型」一支，带进度且可取消
 *    └──可用──▶ ready ──示范──▶ ready（karaoke 高亮由拼接时间戳驱动）
 *                 └──录音──▶ running ──▶ reported
 *                                └──中途失败──▶ typed（已完成行数不清零）
 * ```
 *
 * **五种失败各走各的文案**：未编译语音、系统过低、权限被拒/被限/未问、无设备、缺模型、
 * 设备被占、识别不接受、采集失败。文案一律由 Rust 侧 `permission::explain()` 给出，
 * 界面**不自己编一句**——那个函数按平台给出「去哪个设置页」，前端重写必然漂移。
 *
 * # 三条不得越界的线
 *
 * 1. **不阻塞 UI 线程。** 采集与识别全在 Rust 侧的会话线程上，前端只订阅 Channel。
 * 2. **不显示源自偏置假设的分数。** 偏置转写只驱动「已匹配前缀」这一种提示性高亮，
 *    并且始终与它自带的诊断说明同屏；结果区的六项全部来自能量门控。
 * 3. **不给等级建议。** 语音路径的 FSRS 等级由用户自选，给建议值等于偷偷恢复自动评级。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  FootMark,
  ModelFetch,
  TypedFallback,
  VoiceAvailability,
  VoiceDemonstration,
  VoiceProgress,
  VoiceReport,
} from "../contracts/voice";
import {
  DEGRADE_REASON_LABEL,
  MODEL_FETCH_LABEL,
  NO_OVERLAP_NOTE,
  VOICE_STAGE_LABEL,
  markAt,
} from "../contracts/voice";
import type { VoicePort } from "../data/voicePorts";
import KaraokeLines from "./KaraokeLines";
import VoiceReportView from "./VoiceReportView";
import { contentChars, matchedPrefixLength } from "./voiceHighlight";

export interface VoicePanelProps {
  port: VoicePort;
  poemId: string;
  /**
   * 逐行诗文，用于高亮。
   *
   * **只在调用方确实持有原文时给。** 空数组时本组件从 `marks` 反推行文本（见
   * [`linesFromMarks`]），那条路与时间戳同源，因此二者不可能对不上。
   *
   * 这个形状是亲手 QA 抓出来的：先前由背诵端点的**提示文本**取行，而语音形态在打字端点
   * 上会退化成挖空，于是提示里带着 `＿`——那些字符被正文判据剥掉之后，界面上「床前明月光」
   * 显示成了「床前明光」。单元测试全绿，因为它们传的是真实行文本。
   */
  lines: readonly string[];
  /** 语音走不通时切到打字练习。**携带原因**，由上层决定怎么呈现。 */
  onDegrade(fallback: TypedFallback): void;
}

/** 缺省要下载的模型名。与 Rust 侧 `voice_rig_enabled.rs` 的两个缺省名一致。 */
const REQUIRED_MODELS = [
  "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
  "vits-melo-tts-zh_en",
] as const;

/**
 * 从逐音步标记反推行文本。
 *
 * 同一行的音步按 `index_in_line` 顺序首尾相接且不重叠（`prosody::cut` 保证），所以按行
 * 分组再按行内序号拼接就是原行。**这与拼接时间戳同源**，于是「字」与「亮起来的时刻」不
 * 可能对不上——而它们对不上正是亲手 QA 抓到的那个缺陷。
 */
export function linesFromMarks(marks: readonly FootMark[]): string[] {
  const byLine = new Map<number, FootMark[]>();
  for (const mark of marks) {
    const bucket = byLine.get(mark.line) ?? [];
    bucket.push(mark);
    byLine.set(mark.line, bucket);
  }
  return [...byLine.keys()]
    .sort((left, right) => left - right)
    .map((line) =>
      (byLine.get(line) ?? [])
        .slice()
        .sort((left, right) => left.index_in_line - right.index_in_line)
        .map((mark) => mark.text)
        .join(""),
    );
}

type Phase =
  | { kind: "probing" }
  | { kind: "ready" }
  | { kind: "running" }
  | { kind: "reported"; report: VoiceReport }
  | { kind: "typed"; fallback: TypedFallback };

export default function VoicePanel({ port, poemId, lines, onDegrade }: VoicePanelProps) {
  const [availability, setAvailability] = useState<VoiceAvailability | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "probing" });
  const [demonstration, setDemonstration] = useState<VoiceDemonstration | null>(null);
  const [atMs, setAtMs] = useState(0);
  const [progress, setProgress] = useState<VoiceProgress | null>(null);
  const [biased, setBiased] = useState<string | null>(null);
  const [partialNote, setPartialNote] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<{ next_chars: string; reason: string } | null>(null);
  const [sessionMarks, setSessionMarks] = useState<FootMark[]>([]);
  const [fetching, setFetching] = useState<ModelFetch | null>(null);
  const [fetchOperation, setFetchOperation] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);

  const degrade = useCallback(
    (fallback: TypedFallback) => {
      setPhase({ kind: "typed", fallback });
      onDegrade(fallback);
    },
    [onDegrade],
  );

  useEffect(() => {
    let live = true;
    void port
      .availability()
      .then((next) => {
        if (!live) {
          return;
        }
        setAvailability(next);
        if (next.kind === "typed") {
          setPhase({
            kind: "typed",
            fallback: { reason: next.reason, message: next.message, completed_lines: 0 },
          });
          onDegrade({ reason: next.reason, message: next.message, completed_lines: 0 });
        } else {
          setPhase({ kind: "ready" });
        }
      })
      .catch((cause: unknown) => {
        if (live) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      live = false;
    };
  }, [port, onDegrade]);

  const onDemonstrate = useCallback(() => {
    setError(null);
    void port
      .demonstrate(poemId)
      .then((next) => {
        setDemonstration(next);
        setAtMs(0);
      })
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      });
  }, [port, poemId]);

  const onRecord = useCallback(() => {
    setError(null);
    setPhase({ kind: "running" });
    setBiased(null);
    setPrompt(null);
    setSessionMarks([]);
    void port
      .startSession({ poem_id: poemId, demonstrate: true }, (event) => {
        if (event.type === "progress") {
          setProgress(event.payload);
          return;
        }
        if (event.type === "failed") {
          setError(event.payload.message);
          return;
        }
        if (event.type !== "item") {
          return;
        }
        const item = event.payload;
        switch (item.item) {
          case "demonstrated":
            setSessionMarks((previous) => [...previous, ...item.marks]);
            break;
          case "asr_partial":
            setBiased(item.biased);
            setPartialNote(item.note);
            break;
          case "prompt":
            setPrompt({ next_chars: item.next_chars, reason: item.reason });
            break;
          case "line_observed":
            // 换行即清掉上一行的提示与前缀：它们都是**本行**的观察，留着会让下一行
            // 一开口就显示一段属于上一行的匹配。
            setPrompt(null);
            setBiased(null);
            break;
          case "report":
          case "fallback":
            break;
        }
      })
      .then((outcome) => {
        if (outcome.kind === "reported") {
          setPhase({ kind: "reported", report: outcome.report });
        } else {
          degrade(outcome.fallback);
        }
      })
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
        setPhase({ kind: "ready" });
      });
  }, [port, poemId, degrade]);

  const onFetchModels = useCallback(() => {
    setError(null);
    const operationId = `voice-model-${Date.now()}`;
    setFetchOperation(operationId);
    setFetching({ stage: "downloading", bytes_done: 0, bytes_total: 0 });
    const next = async () => {
      for (const name of REQUIRED_MODELS) {
        const outcome = await port.fetchModel({ name, operation_id: operationId }, (event) => {
          if (event.type === "progress") {
            setFetching(event.payload);
          }
        });
        if (outcome.kind === "unavailable") {
          setFetching(null);
          setFetchOperation(null);
          degrade(outcome.fallback);
          return;
        }
      }
      setFetching(null);
      setFetchOperation(null);
      const again = await port.availability();
      setAvailability(again);
      setPhase(
        again.kind === "voice"
          ? { kind: "ready" }
          : {
              kind: "typed",
              fallback: { reason: again.reason, message: again.message, completed_lines: 0 },
            },
      );
    };
    void next().catch((cause: unknown) => {
      setFetching(null);
      setFetchOperation(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    });
  }, [port, degrade]);

  const onCancelFetch = useCallback(() => {
    if (fetchOperation === null) {
      return;
    }
    void port.cancel(fetchOperation);
  }, [port, fetchOperation]);

  const marks = useMemo(
    () => (demonstration === null ? sessionMarks : demonstration.marks),
    [demonstration, sessionMarks],
  );
  const activeMark = useMemo(() => markAt(marks, atMs), [marks, atMs]);
  // 行文本优先用调用方给的原文；拿不到时从 marks 反推，于是它与时间戳同源。
  const displayLines = useMemo(
    () => (lines.length > 0 ? lines : linesFromMarks(marks)),
    [lines, marks],
  );

  const listeningLine = progress?.stage.stage === "listening" ? progress.stage.line : -1;
  const currentLine = listeningLine >= 0 ? (displayLines[listeningLine] ?? "") : "";
  const matchedChars = matchedPrefixLength(currentLine, biased);

  if (phase.kind === "typed") {
    return (
      <section className="recite-section" aria-label="语音不可用">
        <h3 className="recite-section__title">语音跟读不可用</h3>
        <p className="recite-fallback" role="alert" data-testid="voice-degraded">
          <strong data-testid="voice-degraded-reason">
            {DEGRADE_REASON_LABEL[phase.fallback.reason]}
          </strong>
          {" —— "}
          {phase.fallback.message}
        </p>
        {phase.fallback.completed_lines > 0 && (
          <p className="recite-section__note" data-testid="voice-completed-lines">
            已复诵完的 {phase.fallback.completed_lines} 句进度保留着，不需要重头再来。
          </p>
        )}
        {phase.fallback.reason === "model_unavailable" && (
          <div className="recite-actions">
            <button
              type="button"
              className="recite-button"
              data-testid="voice-fetch-models"
              disabled={fetching !== null}
              onClick={onFetchModels}>
              下载语音模型
            </button>
            {fetching !== null && (
              <button
                type="button"
                className="recite-button"
                data-testid="voice-cancel-fetch"
                onClick={onCancelFetch}>
                取消下载
              </button>
            )}
          </div>
        )}
        {fetching !== null && (
          <p className="recite-section__note" data-testid="voice-fetch-progress">
            {MODEL_FETCH_LABEL[fetching.stage]}
            {fetching.stage === "downloading" &&
              `：已取 ${fetching.bytes_done} / ${fetching.bytes_total === 0 ? "未知" : fetching.bytes_total} 字节`}
          </p>
        )}
        {error !== null && (
          <p className="recite-fallback" role="alert" data-testid="voice-error">
            {error}
          </p>
        )}
      </section>
    );
  }

  return (
    <section className="recite-section" aria-label="语音跟读">
      <h3 className="recite-section__title">语音跟读</h3>

      {availability?.kind === "voice" && (
        <p className="recite-boundary" data-testid="voice-availability-note">
          {availability.note}
        </p>
      )}

      <p className="recite-section__note" data-testid="voice-no-overlap-note">
        {NO_OVERLAP_NOTE}
      </p>

      <div className="recite-actions">
        <button
          type="button"
          className="recite-button"
          data-testid="voice-demonstrate"
          disabled={phase.kind !== "ready"}
          onClick={onDemonstrate}>
          示范朗读
        </button>
        <button
          type="button"
          className="recite-button"
          data-testid="voice-record"
          disabled={phase.kind !== "ready"}
          onClick={onRecord}>
          开始跟读
        </button>
      </div>

      {progress !== null && (
        <p className="recite-section__note" data-testid="voice-stage">
          {VOICE_STAGE_LABEL[progress.stage.stage]}（{progress.completed_lines} /{" "}
          {progress.total_lines} 句）
        </p>
      )}

      {demonstration !== null && (
        <audio
          ref={audio}
          data-testid="voice-audio"
          src={demonstration.audio_url}
          controls
          onTimeUpdate={(event) => {
            setAtMs(Math.round(event.currentTarget.currentTime * 1000));
          }}
        />
      )}

      {displayLines.length === 0 && (
        <p className="recite-section__note" data-testid="voice-lines-pending">
          点「示范朗读」载入诗句：**界面上的字与高亮时刻取自同一份合成结果**，
          所以在合成之前不显示一份可能与它对不上的文本。
        </p>
      )}

      <KaraokeLines
        lines={displayLines}
        marks={marks}
        activeMark={activeMark}
        listeningLine={listeningLine}
        matchedChars={matchedChars}
      />

      {prompt !== null && (
        <p className="recite-prompt" role="status" data-testid="voice-prompt">
          {prompt.reason === "no_speech_yet" ? "还没听到你开口" : "这里停了一会儿"}
          ，下一句起头是「{prompt.next_chars}」。
        </p>
      )}

      {biased !== null && (
        <details data-testid="voice-diagnostics">
          <summary className="recite-field__hint">查看识别转写（仅供诊断）</summary>
          <p className="recite-field__hint" data-testid="voice-partial-note">
            {partialNote}
          </p>
          <p className="recite-ops" data-testid="voice-partial-biased">
            {biased}
          </p>
          <p className="recite-field__hint">
            已在上方高亮的是这段转写与本句的公共前缀，长度 {matchedChars} 字（本句共{" "}
            {contentChars(currentLine).length} 字）。它<strong>不是完整度</strong>
            ：偏置解码把诗文本当提示词，因此它倾向于吐出原文，哪怕你跳过了整句。
          </p>
        </details>
      )}

      {phase.kind === "reported" && <VoiceReportView report={phase.report} />}

      {error !== null && (
        <p className="recite-fallback" role="alert" data-testid="voice-error">
          {error}
        </p>
      )}
    </section>
  );
}
