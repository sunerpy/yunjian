/**
 * 形态选择：挖空（比例可调）、首字提示、遮挡（档位可调）、语音。
 *
 * # 四个形态都必须出现在选择器里，包括走不通的那一个
 *
 * 语音会话（todo 66）还没接进来，所以此刻选「语音」一定会退化成打字。
 * 但它**不能从选项里消失**：消失会让用户以为这个产品没有语音功能，
 * 而真实情况是「有这条路，本版本还没接上，原因如下」。这与
 * `cli.rs` 把中华新韵留在 `--book` 取值域里的理由是同一条——
 * 「没有这个东西」必须能被请求到并得到明确回答。
 *
 * 退化原因由**内核**给出（`command.rs` 的 `VoiceFallback::message`），
 * 界面只显示它，不自己编一句「语音暂不可用」。三种原因（未开特性 / 缺模型 /
 * 会话未接入）对用户的下一步动作完全不同，糊成一句就把那个区别抹掉了。
 *
 * # 比例与档位是形态的一部分，不是全局设置
 *
 * `--ratio` 只对挖空有意义，`--masked-lines` 只对遮挡有意义
 * （`cli.rs` 的 `Mode::practice`）。所以这两个控件跟着形态出现与消失，
 * 而不是常驻——常驻会让用户在首字提示下调一个不起作用的比例。
 */

import type { ReciteModeId } from "../contracts/recite";
import {
  DEFAULT_CLOZE_RATIO,
  RECITE_MODE_HINT,
  RECITE_MODE_IDS,
  RECITE_MODE_LABEL,
} from "../contracts/recite";

export interface ModeSelectorProps {
  mode: ReciteModeId;
  ratio: number;
  maskedLines: number;
  /** 遮挡档位的上界，来自上一局的 `line_count`；未出题过时给一个保守值。 */
  maxMaskedLines: number;
  disabled: boolean;
  onModeChange(mode: ReciteModeId): void;
  onRatioChange(ratio: number): void;
  onMaskedLinesChange(lines: number): void;
}

/**
 * 挖空比例的滑块步长。
 *
 * 取 0.05 而不是更细：内核按比例乘字数再取整挑位置，一首五言绝句只有 20 个正文字，
 * 0.01 的步长里有多档会挖出同样多的空，滑起来像坏了。
 */
const RATIO_STEP = 0.05;

export default function ModeSelector({
  mode,
  ratio,
  maskedLines,
  maxMaskedLines,
  disabled,
  onModeChange,
  onRatioChange,
  onMaskedLinesChange,
}: ModeSelectorProps) {
  return (
    <section className="recite-section" aria-label="练习形态">
      <h2 className="recite-section__title">练习形态</h2>

      <div className="recite-actions" role="group" aria-label="形态选择">
        {RECITE_MODE_IDS.map((candidate) => (
          <button
            key={candidate}
            type="button"
            className="recite-button"
            data-testid={`mode-${candidate}`}
            aria-pressed={candidate === mode}
            disabled={disabled}
            onClick={() => {
              onModeChange(candidate);
            }}>
            {RECITE_MODE_LABEL[candidate]}
          </button>
        ))}
      </div>

      <p className="recite-section__note" data-testid="mode-hint">
        {RECITE_MODE_HINT[mode]}
      </p>

      {mode === "cloze" && (
        <div className="recite-field">
          <label className="recite-field__label" htmlFor="cloze-ratio">
            挖空比例
            {/* 比例原样显示成小数，与载荷里的 `ratio` 同一口径。
                换算成百分比要乘 100，而那是在一个内核给出的数上做算术。 */}
            <span className="recite-queue__number" data-testid="cloze-ratio-value">
              {" "}
              {ratio.toFixed(2)}
            </span>
          </label>
          <input
            id="cloze-ratio"
            className="recite-field__control"
            data-testid="cloze-ratio"
            type="range"
            min={RATIO_STEP}
            max={1}
            step={RATIO_STEP}
            value={ratio}
            disabled={disabled}
            onChange={(event) => {
              onRatioChange(Number(event.target.value));
            }}
          />
          <p className="recite-field__hint">
            默认 {DEFAULT_CLOZE_RATIO.toFixed(2)}。挖空位置由内核按韵脚、实词、虚词的优先级挑选，
            同一种子在任何机器上给出同一组空位；出题后载荷会回显那个种子。
          </p>
        </div>
      )}

      {mode === "masked" && (
        <div className="recite-field">
          <label className="recite-field__label" htmlFor="masked-lines">
            遮挡句数
            <span className="recite-queue__number" data-testid="masked-lines-value">
              {" "}
              {maskedLines}
            </span>
          </label>
          <input
            id="masked-lines"
            className="recite-field__control"
            data-testid="masked-lines"
            type="range"
            min={0}
            max={maxMaskedLines}
            step={1}
            value={maskedLines}
            disabled={disabled}
            onChange={(event) => {
              onMaskedLinesChange(Number(event.target.value));
            }}
          />
          <p className="recite-field__hint">
            0 为全文可见。超过实际句数时内核会收敛为全遮挡，因此这里的上界只是个方便值， 不是判据。
          </p>
        </div>
      )}
    </section>
  );
}
