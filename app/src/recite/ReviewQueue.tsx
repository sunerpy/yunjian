/**
 * 今日复习队列：由 FSRS 排程驱动，显示等级且可调。
 *
 * # 队列里只有 `poem_id`，没有题目与作者，这是刻意的
 *
 * `recite due` 在命令行里**不打开语料库**（已实测缺语料时仍退出 0）：复习状态按
 * `stable_id` 存在独立的 `recite.db` 里，与语料无关。为了给每行补上标题就得开语料库，
 * 那会让「排程能不能看」取决于「语料下没下」，把一个本来无关的依赖装回去。
 *
 * # 「等级可调」在这里是一次真实的复习提交
 *
 * 调整某一项的等级会调 `Scheduler::review(stable_id, grade)`，也就是 FSRS 里的
 * 一次复习——间隔与稳定度会跟着变。这一点必须写在界面上，否则用户会以为自己
 * 只是在改一个标签。**内核没有「修正上一次评级」这种入口**，`review_log`
 * 是私有的（只喂 `optimize_parameters`），所以「改而不是新增」在当前 API 下
 * 做不到；伪装成修正才是撒谎。
 *
 * # 日序号不换算成日期
 *
 * `due_day` 与 `last_review_day` 是 Unix 日序号。把它换成「还有几天」需要一个
 * 「今天是第几天」的基准，而那个基准在内核的 `unix_day_now()` 里；
 * 「还有几天」这件事内核已经用 `scheduled_days` 给出来了。
 */

import { useCallback, useEffect, useState } from "react";
import type { FsrsGradeId, ReciteDue, ReciteStats } from "../contracts/recite";
import {
  EMPTY_QUEUE_NOTE,
  FSRS_GRADE_IDS,
  FSRS_GRADE_LABEL,
  QUEUE_DISTRIBUTION_NOTE,
  QUEUE_REGRADE_NOTE,
  SCORE_LABEL,
} from "../contracts/recite";
import type { ReciteReviewPort } from "../data/recitePorts";

export interface ReviewQueueProps {
  port: ReciteReviewPort;
  /** 落账后自增，用于让队列重新拉取——一次练习提交后队列内容确实变了。 */
  refreshToken: number;
  onPractice(poemId: string): void;
}

export default function ReviewQueue({ port, refreshToken, onPractice }: ReviewQueueProps) {
  const [includeFuture, setIncludeFuture] = useState(false);
  const [due, setDue] = useState<ReciteDue | null>(null);
  const [stats, setStats] = useState<ReciteStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let disposed = false;
    void Promise.all([port.due(includeFuture), port.stats()])
      .then(([nextDue, nextStats]) => {
        if (!disposed) {
          setDue(nextDue);
          setStats(nextStats);
        }
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      disposed = true;
    };
  }, [port, includeFuture, refreshToken]);

  const onRegrade = useCallback(
    (poemId: string, grade: FsrsGradeId) => {
      setBusy(true);
      setError(null);
      void port
        .commitGrade({ poem_id: poemId, grade, chosen_by_user: true })
        .then(() => port.due(includeFuture))
        .then(setDue)
        .catch((cause: unknown) => {
          setError(cause instanceof Error ? cause.message : String(cause));
        })
        .finally(() => {
          setBusy(false);
        });
    },
    [port, includeFuture],
  );

  return (
    <section className="recite-section" aria-label="复习队列">
      <h2 className="recite-section__title">{includeFuture ? "整份排程" : "今天到期"}</h2>

      <div className="recite-actions">
        <label className="recite-field__hint">
          <input
            type="checkbox"
            data-testid="include-future"
            checked={includeFuture}
            onChange={(event) => {
              setIncludeFuture(event.target.checked);
            }}
          />{" "}
          含未到期
        </label>
      </div>

      {stats !== null && (
        <p className="recite-section__note" data-testid="queue-stats">
          已排程 {stats.scheduled_total} 首 · 今天到期 {stats.due_today} 首 · 最近等级分布：
          {FSRS_GRADE_IDS.map(
            (grade) => `${FSRS_GRADE_LABEL[grade]} ${stats.by_last_grade[grade]}`,
          ).join(" · ")}
          。{QUEUE_DISTRIBUTION_NOTE}
        </p>
      )}

      {due === null && error === null && (
        <p className="recite-section__note" data-testid="queue-loading">
          正在读取排程……
        </p>
      )}

      {due !== null && due.items.length === 0 && (
        <p className="recite-section__note" data-testid="queue-empty">
          {EMPTY_QUEUE_NOTE}
        </p>
      )}

      {due !== null && due.items.length > 0 && (
        <ul className="recite-queue" data-testid="review-queue">
          {due.items.map((item) => (
            <li key={item.poem_id} className="recite-queue__item" data-testid="queue-item">
              <span className="recite-queue__id" data-testid={`queue-id-${item.poem_id}`}>
                {item.poem_id}
              </span>
              <span className="recite-queue__meta">
                <span data-testid={`queue-grade-${item.poem_id}`}>
                  最近等级 {FSRS_GRADE_LABEL[item.last_grade]}
                </span>
                <span className="recite-queue__number">间隔 {item.scheduled_days} 天</span>
                <span className="recite-queue__number">到期日序 {item.due_day}</span>
                <span className="recite-queue__number">上次日序 {item.last_review_day}</span>
                <span className="recite-queue__number">稳定度 {item.stability.toFixed(2)}</span>
                <span className="recite-queue__number">难度 {item.difficulty.toFixed(2)}</span>
              </span>
              <span className="recite-actions">
                <button
                  type="button"
                  className="recite-button"
                  data-testid={`queue-practice-${item.poem_id}`}
                  onClick={() => {
                    onPractice(item.poem_id);
                  }}>
                  练这一首
                </button>
                {FSRS_GRADE_IDS.map((grade) => (
                  <button
                    key={grade}
                    type="button"
                    className="recite-button"
                    data-testid={`queue-regrade-${item.poem_id}-${grade}`}
                    aria-pressed={grade === item.last_grade}
                    disabled={busy}
                    onClick={() => {
                      onRegrade(item.poem_id, grade);
                    }}>
                    {FSRS_GRADE_LABEL[grade]}
                  </button>
                ))}
              </span>
            </li>
          ))}
        </ul>
      )}

      <p className="recite-section__note" data-testid="regrade-note">
        {QUEUE_REGRADE_NOTE}
      </p>

      {stats !== null && (
        <p className="recite-section__note" data-testid="grading-thresholds">
          打字路径的评级阈值（来自 config.toml 的 [recite.grading]）：
          {SCORE_LABEL.completeness}低于 {stats.grading.again_completeness_below} 记
          {FSRS_GRADE_LABEL.again} · {SCORE_LABEL.accuracy_lenient}低于{" "}
          {stats.grading.hard_accuracy_lenient_below} 记{FSRS_GRADE_LABEL.hard} · 回读多于{" "}
          {stats.grading.hard_rerecitation_above} 次记{FSRS_GRADE_LABEL.hard} · 首次作答
          {SCORE_LABEL.accuracy_strict}达到 {stats.grading.easy_accuracy_strict_at_least} 才记
          {FSRS_GRADE_LABEL.easy}。等级由内核按严格优先级判定，这里只报阈值。
        </p>
      )}

      {due !== null && (
        <p className="recite-section__note" data-testid="queue-database">
          复习库：{due.database}
        </p>
      )}

      {error !== null && (
        <p className="recite-fallback" role="alert" data-testid="queue-error">
          {error}
        </p>
      )}
    </section>
  );
}
