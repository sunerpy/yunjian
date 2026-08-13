/**
 * AI 赏析面板。
 *
 * # 它必须在**文字上与视觉上**都不可能被误认为考据成果
 *
 * `docs/AI.zh.md:268-275` 把这件事写成了三条义务，其中两条落在这个文件上：
 * 「视觉分区」与「附『未经人工审校』的说明」。
 *
 * ## 文字层面（四项，缺一条都有测试拦着）
 *
 * 1. 标题是 **`AI 赏析`**，不是「赏析」。
 * 2. 标题旁一枚短标签：`AI 生成，未经人工审校`。
 * 3. 模型名显式显示——它是「这段话是谁写的」的唯一线索。
 * 4. 面板底部一条完整准确性披露，逐字取自既有常量 `AI_UNREVIEWED_DISCLOSURE`。
 *
 * ## 视觉层面（五项手段，与 `sourced-block` 逐项相反）
 *
 * | 维度 | 考据材料 `.sourced-block` | 本面板 `.ai-panel` |
 * | --- | --- | --- |
 * | 字体 | 衬线 `--font-serif`（刻本观感） | 无衬线 `--font-sans` |
 * | 边框 | 1px 实线 | **2px 虚线** |
 * | 底色 | 纸白 `--color-sourced-surface` | 冷灰蓝 `--color-ai-surface` |
 * | 左侧 | 无 | 一道斜纹标尺 `--color-ai-stripe` |
 * | 容器类 | `sourced-block` | `ai-panel`（**不同名**） |
 *
 * 在一个通篇衬线排古典诗词的界面里，无衬线本身就是最强的「这不是原始材料」信号。
 *
 * ## 不交错这一条由结构保证
 *
 * 面板前面有一条分界线与一句 `AI_BOUNDARY_CAPTION`，且它在 DOM 里排在原文与集评之后。
 * 更重要的是**容器互不嵌套**：没有任何 `data-provenance="ai-generated"` 的节点会出现在
 * `data-provenance="sourced"` 的节点里面，反之亦然。这一条有断言直接钉住，
 * 因为「交错」在代码里的形态恰恰就是嵌套。
 */

import type { AppreciationState } from "../contracts/ai";
import {
  AI_BOUNDARY_CAPTION,
  AI_PANEL_LABEL,
  AI_UNREVIEWED_BADGE,
  AI_UNREVIEWED_DISCLOSURE,
  APPRECIATION_SOURCE_LABEL,
} from "../contracts/ai";

export interface AiAppreciationPanelProps {
  state: AppreciationState;
}

export default function AiAppreciationPanel({ state }: AiAppreciationPanelProps) {
  return (
    <>
      {/* 分界不是装饰：它是「以下内容性质不同」这句话的可见形态。 */}
      <hr className="provenance-divider" data-testid="provenance-divider" />
      <p className="provenance-divider__caption" data-testid="ai-boundary-caption">
        {AI_BOUNDARY_CAPTION}
      </p>

      <section
        className="ai-panel"
        data-provenance="ai-generated"
        data-testid="ai-panel"
        aria-label={AI_PANEL_LABEL}>
        <header className="ai-panel__head">
          <h2 className="ai-panel__title" data-testid="ai-panel-label">
            {AI_PANEL_LABEL}
          </h2>
          <span className="ai-panel__badge" data-testid="ai-unreviewed-badge">
            {AI_UNREVIEWED_BADGE}
          </span>
        </header>

        {state.kind === "ready" && (
          <>
            <dl className="ai-panel__provenance">
              <div className="ai-panel__provenance-item">
                <dt>模型</dt>
                <dd data-testid="ai-model">{state.view.model}</dd>
              </div>
              <div className="ai-panel__provenance-item">
                <dt>提示词模板</dt>
                <dd data-testid="ai-template-version">{state.view.template_version}</dd>
              </div>
              {state.view.source !== undefined && (
                <div className="ai-panel__provenance-item">
                  <dt>来源</dt>
                  <dd data-testid="ai-source">{APPRECIATION_SOURCE_LABEL[state.view.source]}</dd>
                </div>
              )}
            </dl>
            <div className="ai-panel__body" data-testid="ai-text">
              {state.view.text
                .split("\n")
                .filter((paragraph) => paragraph.trim() !== "")
                .map((paragraph, index) => (
                  <p className="ai-panel__paragraph" key={index}>
                    {paragraph}
                  </p>
                ))}
            </div>
          </>
        )}

        {state.kind === "absent" && (
          <p className="ai-panel__status" data-testid="ai-absent">
            本篇没有随包赏析，也还没有在本机生成过。
          </p>
        )}

        {state.kind === "configuration_required" && (
          <p className="ai-panel__status" data-testid="ai-configuration-required">
            需要先配置 AI 服务商与密钥：{state.settings_path}
          </p>
        )}

        {state.kind === "failed" && (
          <p
            className="ai-panel__status ai-panel__status--failed"
            role="alert"
            data-testid="ai-failed">
            {state.message}
          </p>
        )}

        {/* 披露常驻，与有没有正文无关：面板本身的性质不因内容缺失而改变。 */}
        <p className="ai-panel__disclosure" data-testid="ai-disclosure">
          {AI_UNREVIEWED_DISCLOSURE}
        </p>
      </section>
    </>
  );
}
