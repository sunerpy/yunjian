/**
 * 背诵屏：形态选择 → 出题 → 作答 → 结果与评级，右侧常驻复习队列。
 *
 * # 「一局最多提交一次」的落点就在这里
 *
 * `commitGrade` 每调一次就是 FSRS 里的一次真实复习，所以一局练习最多能调一次。
 * 端口层拦不住这件事（端口无状态），所以状态机放在本组件：`commit !== null`
 * 之后确认按钮禁用，`ResultView` 里的等级选择也一并锁住。
 * 对应 todo 56 被裁决改写后的验收「用户确认后最多提交一次」。
 *
 * # 一局的四个阶段
 *
 * ```
 * idle  ──出题──▶ session ──提交作答──▶ attempt ──确认等级──▶ committed
 *   ▲                                                            │
 *   └──────────────────── 再练一首 ◀────────────────────────────┘
 * ```
 *
 * 阶段不做成路由：这四步是一局练习内部的进展，用户按浏览器后退键回到「作答之前」
 * 是没有意义的（作答已经评过了），做成 URL 只会造出一个能回到不一致状态的入口。
 */

import { useCallback, useState } from "react";
import type {
  FsrsGradeId,
  ReciteAttempt,
  ReciteCommit,
  ReciteModeId,
  ReciteSession,
} from "../contracts/recite";
import { DEFAULT_CLOZE_RATIO } from "../contracts/recite";
import type { RecitePorts } from "../data/recitePorts";
import ModeSelector from "./ModeSelector";
import ResultView from "./ResultView";
import ReviewQueue from "./ReviewQueue";
import TypingPanel from "./TypingPanel";
import "./recite.css";

export interface ReciteScreenProps {
  ports: RecitePorts;
  /**
   * 默认要练的作品标识。
   *
   * 默认值与 `data/samplePorts.ts` 里的第一首样例一致，于是样例模式下一进屏就能出题。
   * 真实路径上由复习队列的「练这一首」或检索页传进来。
   */
  defaultPoemId?: string;
}

/** 遮挡档位滑块的初始上界。出题一次之后换成载荷里的 `line_count`。 */
const INITIAL_MAX_MASKED_LINES = 4;

export default function ReciteScreen({
  ports,
  defaultPoemId = "sample-jingyesi",
}: ReciteScreenProps) {
  const [poemId, setPoemId] = useState(defaultPoemId);
  const [mode, setMode] = useState<ReciteModeId>("cloze");
  const [ratio, setRatio] = useState(DEFAULT_CLOZE_RATIO);
  const [maskedLines, setMaskedLines] = useState(2);
  const [session, setSession] = useState<ReciteSession | null>(null);
  const [answer, setAnswer] = useState("");
  const [attempt, setAttempt] = useState<ReciteAttempt | null>(null);
  const [commit, setCommit] = useState<ReciteCommit | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);

  const request = { poem_id: poemId, mode, ratio, masked_lines: maskedLines };

  const onStart = useCallback(() => {
    setBusy(true);
    setError(null);
    setAttempt(null);
    setCommit(null);
    setAnswer("");
    void ports.practice
      .startSession(request)
      .then(setSession)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        setBusy(false);
      });
    // `request` 是每次渲染新建的对象，放进依赖会让这个回调每帧重建；
    // 它的每一项都在依赖里逐个列出，效果相同而不会抖动。
  }, [ports, poemId, mode, ratio, maskedLines]);

  const onSubmit = useCallback(() => {
    if (session === null) {
      return;
    }
    setBusy(true);
    setError(null);
    void ports.practice
      // 种子原样带回：内核的会话是无状态重建的，少带一项就会重建成另一局挖空。
      // 展开而不是写 `seed: session.seed`：`exactOptionalPropertyTypes` 下
      // 显式的 `undefined` 与「这个键不存在」是两回事，而后者才是「没有种子」。
      .submitAnswer({
        ...request,
        ...(session.seed === undefined ? {} : { seed: session.seed }),
        answer,
      })
      .then(setAttempt)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        setBusy(false);
      });
  }, [ports, session, answer, poemId, mode, ratio, maskedLines]);

  const onCommit = useCallback(
    (grade: FsrsGradeId, chosenByUser: boolean) => {
      setBusy(true);
      setError(null);
      void ports.review
        .commitGrade({ poem_id: poemId, grade, chosen_by_user: chosenByUser })
        .then((next) => {
          setCommit(next);
          setRefreshToken((token) => token + 1);
        })
        .catch((cause: unknown) => {
          setError(cause instanceof Error ? cause.message : String(cause));
        })
        .finally(() => {
          setBusy(false);
        });
    },
    [ports, poemId],
  );

  const onPractice = useCallback((nextPoemId: string) => {
    setPoemId(nextPoemId);
    setSession(null);
    setAttempt(null);
    setCommit(null);
    setAnswer("");
  }, []);

  return (
    <div className="recite-screen" data-testid="recite-screen">
      <h1 className="recite-screen__title">背诵</h1>

      <section className="recite-section" aria-label="选择作品">
        <h2 className="recite-section__title">选择作品</h2>
        <div className="recite-field">
          <label className="recite-field__label" htmlFor="recite-poem-id">
            作品标识
          </label>
          <input
            id="recite-poem-id"
            className="recite-field__control"
            data-testid="recite-poem-id"
            value={poemId}
            disabled={busy}
            onChange={(event) => {
              onPractice(event.target.value);
            }}
          />
          <p className="recite-field__hint">
            用检索页找到作品后可以复制它的标识；复习队列里的「练这一首」会直接填进来。
          </p>
        </div>
      </section>

      <ModeSelector
        mode={mode}
        ratio={ratio}
        maskedLines={maskedLines}
        maxMaskedLines={session?.line_count ?? INITIAL_MAX_MASKED_LINES}
        disabled={busy}
        onModeChange={setMode}
        onRatioChange={setRatio}
        onMaskedLinesChange={setMaskedLines}
      />

      <div className="recite-actions">
        <button
          type="button"
          className="recite-button"
          data-testid="start-session"
          disabled={busy || poemId.trim() === ""}
          onClick={onStart}>
          {session === null ? "出题" : "重新出题"}
        </button>
      </div>

      {error !== null && (
        <p className="recite-fallback" role="alert" data-testid="recite-error">
          {error}
        </p>
      )}

      {session !== null && (
        <TypingPanel
          session={session}
          answer={answer}
          busy={busy}
          submitted={attempt !== null}
          onAnswerChange={setAnswer}
          onSubmit={onSubmit}
        />
      )}

      {attempt !== null && (
        <ResultView attempt={attempt} commit={commit} busy={busy} onCommit={onCommit} />
      )}

      <ReviewQueue port={ports.review} refreshToken={refreshToken} onPractice={onPractice} />
    </div>
  );
}
