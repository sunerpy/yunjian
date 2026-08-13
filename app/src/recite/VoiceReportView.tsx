/**
 * 语音会话的结果视图。
 *
 * # 这里没有分数，只有观察
 *
 * 六项全部由内核算好送来，界面只用 `toFixed` 定小数位，**不做任何算术**：不换算百分比、
 * 不合成总分、不按阈值推等级。依据是 2026-08-11 的裁决——文言 ASR 实测 CER 77.01%
 * （TTS 合成音的乐观上界），所以 v1 语音契约是「示范 + 复诵 + 只报是否开口/停顿/相对节奏
 * + 用户自评」。
 *
 * # 「节奏连贯度」不是读音评分，这一条必须写在屏上
 *
 * 一个显示成 `0.213` 的指标极容易被读成「读得不够标准」。所以那一句注记与这几个数
 * **同屏且相邻**，不是折叠在某个「详情」里；而且注记里明说它的三项输入来自能量门控，
 * 与识别转写无关。
 *
 * # 等级不在这里选
 *
 * 语音路径的 FSRS 等级由用户在既有的结果区自选（`recite_commit_grade`）。本视图**不给
 * 任何建议值**：给一个建议值就等于偷偷恢复了自动评级，而那是裁决明确禁止的。
 */

import type { VoiceReport } from "../contracts/voice";
import { RELATIVE_RHYTHM_LABEL, VOICE_NO_MACHINE_SCORE_NOTE } from "../contracts/voice";

export interface VoiceReportViewProps {
  report: VoiceReport;
}

/** 小数位数。三位与打字路径的结果视图一致，两处显示同一类数不该有不同精度。 */
const DIGITS = 3;

export default function VoiceReportView({ report }: VoiceReportViewProps) {
  return (
    <section className="recite-section" aria-label="语音跟读结果">
      <h3 className="recite-section__title">语音跟读结果</h3>

      <dl className="recite-scores" data-testid="voice-report">
        <dt>是否开口</dt>
        <dd data-testid="voice-spoke">{report.spoke ? "检测到" : "整段未检测到"}</dd>
        <dt>长停顿</dt>
        <dd data-testid="voice-long-pauses">{report.long_pause_count} 次</dd>
        <dt>相对节奏</dt>
        <dd data-testid="voice-relative-rhythm">{RELATIVE_RHYTHM_LABEL[report.relative_rhythm]}</dd>
        <dt>{report.coherence_label}</dt>
        <dd data-testid="voice-coherence">{report.coherence.toFixed(DIGITS)}</dd>
        <dt>复诵句数</dt>
        <dd data-testid="voice-lines-attempted">{report.lines_attempted} 句</dd>
        <dt>提示次数</dt>
        <dd data-testid="voice-prompt-count">{report.prompt_count} 次</dd>
      </dl>

      {/* 注记紧跟数值表，中间不插别的段落：todo 63 的亲手 QA 已经量到，一句口径说明插在
          中间会把注记推开约 40px，而那个数正需要它就在旁边才不会被读成读音评价。 */}
      <p className="recite-boundary" data-testid="voice-no-score-note">
        {VOICE_NO_MACHINE_SCORE_NOTE}
      </p>

      <details data-testid="voice-coherence-inputs">
        <summary className="recite-field__hint">{report.coherence_label}是怎么算出来的</summary>
        <dl className="recite-scores">
          <dt>起始间隔方差</dt>
          <dd data-testid="voice-gap-variance">{report.gap_variance_ms2.toFixed(1)} ms²</dd>
          <dt>时长比（相对示范音）</dt>
          <dd data-testid="voice-duration-ratio">{report.duration_ratio.toFixed(DIGITS)}</dd>
          <dt>长停顿</dt>
          <dd>{report.long_pause_count} 次</dd>
        </dl>
        <p className="recite-field__hint">
          三项各自归一后相乘。三项都来自能量门控测到的语音活动段，
          <strong>与识别转写无关</strong>
          ——识别器只给出 token 的起始时刻而没有结束时刻，强制对齐上游明确不做，
          因此起始间隔只能取自能量门控。
        </p>
      </details>
    </section>
  );
}
