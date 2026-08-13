/**
 * 结果视图：内核给的分数、逐字反馈，以及等级确认。
 *
 * # 四个数字全部原样搬运
 *
 * `completeness`、`accuracy_strict`、`accuracy_lenient`、`fluency` 都是内核算好的
 * `[0, 1]` 小数。这里只用 `toFixed(3)` 定小数位，**不做任何算术**：
 *
 * - 不换算百分比（那要乘 100）；
 * - 不合成「总分」（那要加权）；
 * - 不按阈值重判等级（等级由 `grade_typed` 给出，界面只显示与覆盖）。
 *
 * 这条边界有一条可证伪的守卫盯着（`__tests__/noScoreArithmetic.test.ts`），
 * 而且它是正反两向的：既禁止算术，也确认这四个字段真的被读了——只写禁令的话，
 * 把分数整段删掉的实现同样能通过。
 *
 * # 「节奏连贯度」不是发音质量，这一条必须写在屏上
 *
 * `fluency` 在打字路径上恒为中性满值（内核注释：「打字路径没有时序信号，使用中性满值
 * 且**不表示发音质量**」）。一个显示成 `1.000` 的「节奏连贯度」极容易被读成
 * 「读得很标准」，所以 `NO_PRONUNCIATION_NOTE` 与这四个数**同屏且相邻**，
 * 不是折叠在某个「详情」里。
 *
 * 语音路径按 2026-08-11 的裁决同样不评发音：文言识别的字错率实测 77.01%（合成音的
 * 乐观上界），v1 契约是「示范 + 复诵 + 只报是否开口/停顿/相对节奏 + 用户自评」。
 * 所以这句注记在两条路径上都成立，不是打字路径的临时说明。
 *
 * # 等级为什么要用户确认
 *
 * 打字路径有 `grade_typed` 的映射，界面把它作为**建议值**预选；语音路径没有任何
 * 自动映射，只能手选。两条路径共用同一个选择器，于是「等级最终由谁定」在界面上
 * 是一致的：由你定，内核给建议。
 */

import { useState } from "react";
import type { FsrsGradeId, ReciteAttempt, ReciteCommit } from "../contracts/recite";
import {
  FSRS_GRADE_HINT,
  FSRS_GRADE_IDS,
  FSRS_GRADE_LABEL,
  GRADE_SOURCE_LABEL,
  NO_PRONUNCIATION_NOTE,
  REJECTED_NOTE,
  SCORE_LABEL,
  VOICE_GRADE_IS_MANUAL_NOTE,
} from "../contracts/recite";
import OpFeedback from "./OpFeedback";

export interface ResultViewProps {
  attempt: ReciteAttempt;
  /** 已落账的结果；未提交时为 `null`。 */
  commit: ReciteCommit | null;
  busy: boolean;
  onCommit(grade: FsrsGradeId, chosenByUser: boolean): void;
}

/** 小数位数。三位与命令行的 `{:.3}` 一致，两处显示同一个数不该有不同精度。 */
const SCORE_DIGITS = 3;

export default function ResultView({ attempt, commit, busy, onCommit }: ResultViewProps) {
  const [grade, setGrade] = useState<FsrsGradeId>(attempt.suggested_grade);
  const chosenByUser = grade !== attempt.suggested_grade;
  const committed = commit !== null;

  return (
    <section className="recite-section" aria-label="作答结果">
      <h2 className="recite-section__title">作答结果</h2>

      <OpFeedback ops={attempt.ops} />

      <dl className="recite-scores" data-testid="score-facts">
        <dt>{SCORE_LABEL.completeness}</dt>
        <dd data-testid="score-completeness">{attempt.score.completeness.toFixed(SCORE_DIGITS)}</dd>
        <dt>{SCORE_LABEL.accuracy_strict}</dt>
        <dd data-testid="score-strict-accuracy">
          {attempt.score.accuracy_strict.toFixed(SCORE_DIGITS)}
        </dd>
        <dt>{SCORE_LABEL.accuracy_lenient}</dt>
        <dd data-testid="score-lenient-accuracy">
          {attempt.score.accuracy_lenient.toFixed(SCORE_DIGITS)}
        </dd>
        <dt>{SCORE_LABEL.fluency}</dt>
        <dd data-testid="score-fluency">{attempt.score.fluency.toFixed(SCORE_DIGITS)}</dd>
      </dl>

      {/* 注记紧跟分数表，中间不插别的段落：亲手 QA 时它被一句「均为 0 至 1 的比例」
          推开了约 40px，而「节奏连贯度 1.000」正需要它就在旁边才不会被读成
          「读得很标准」。口径说明改排在注记之后。 */}
      <p className="recite-boundary" data-testid="pronunciation-boundary">
        {NO_PRONUNCIATION_NOTE}
      </p>

      <p className="recite-section__note">均为 0 至 1 的比例，由背诵内核给出，界面不做换算。</p>

      {attempt.score.is_rejected && (
        <p className="recite-fallback" role="alert" data-testid="rejected-note">
          {REJECTED_NOTE}
        </p>
      )}

      <div className="recite-field">
        <span className="recite-field__label" id="grade-picker-label">
          本次评级
        </span>
        <div className="recite-actions" role="group" aria-labelledby="grade-picker-label">
          {FSRS_GRADE_IDS.map((candidate) => (
            <button
              key={candidate}
              type="button"
              className="recite-button"
              data-testid={`grade-${candidate}`}
              aria-pressed={candidate === grade}
              disabled={busy || committed}
              onClick={() => {
                setGrade(candidate);
              }}>
              {FSRS_GRADE_LABEL[candidate]}
            </button>
          ))}
        </div>
        <p className="recite-field__hint" data-testid="grade-explanation">
          内核建议「{FSRS_GRADE_LABEL[attempt.suggested_grade]}」（
          {GRADE_SOURCE_LABEL.typed_mapping}
          {attempt.first_attempt ? "，本次为首次作答" : "，本次非首次作答"}）。
          {FSRS_GRADE_HINT[grade]}
          {chosenByUser &&
            `已改为「${FSRS_GRADE_LABEL[grade]}」，将记为${GRADE_SOURCE_LABEL.user_chosen}。`}
        </p>
        <p className="recite-field__hint" data-testid="voice-grade-note">
          {VOICE_GRADE_IS_MANUAL_NOTE}
        </p>
      </div>

      <div className="recite-actions">
        <button
          type="button"
          className="recite-button"
          data-testid="commit-grade"
          disabled={busy || committed}
          onClick={() => {
            onCommit(grade, chosenByUser);
          }}>
          {committed ? "已计入排程" : "确认并计入排程"}
        </button>
      </div>

      {commit !== null && (
        <dl className="recite-scores" data-testid="commit-facts">
          <dt>已提交等级</dt>
          <dd data-testid="commit-grade-label">{FSRS_GRADE_LABEL[commit.grade]}</dd>
          <dt>等级来源</dt>
          <dd data-testid="commit-grade-source">{GRADE_SOURCE_LABEL[commit.grade_source]}</dd>
          <dt>下次间隔</dt>
          <dd data-testid="commit-scheduled-days">{commit.review.scheduled_days} 天</dd>
          <dt>到期日序</dt>
          <dd data-testid="commit-due-day">{commit.review.due_day}</dd>
          <dt>复习库</dt>
          <dd data-testid="commit-database">{commit.database}</dd>
        </dl>
      )}

      <details>
        <summary className="recite-field__hint">查看归一化后的参考诗文与作答</summary>
        <dl className="recite-scores">
          <dt>参考</dt>
          <dd data-testid="attempt-reference">{attempt.reference}</dd>
          <dt>作答</dt>
          <dd data-testid="attempt-answer">{attempt.answer}</dd>
        </dl>
      </details>
    </section>
  );
}
