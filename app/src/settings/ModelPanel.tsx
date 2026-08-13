/**
 * 语音模型管理：逐个列出体积、许可与本机状态。
 *
 * # 每一行都必须显示许可，这不是信息密度偏好
 *
 * 项目只放行经核实的 MIT 与 Apache-2.0（`models.toml` + `xtask verify-models`），
 * 被拒的模型连同理由记在 `models/DENYLIST.md`。界面把许可显示出来，
 * 是让这条立场在用户那一侧可见而不是只活在构建期。
 *
 * `ModelCache::statuses()` 的注释说明了为什么被拒的模型也要列出来：
 *
 * > **被拒的模型也列出来**，只是带上 `refused`。……一旦有人加进来，
 * > `list` 必须让它显形而不是把它藏起来——藏起来只会让人以为清单没被改过。
 *
 * 所以本组件不过滤 `refused !== null` 的行，而是把理由显示在那一行里。
 */

import { useEffect, useState } from "react";
import type { ModelKind, ModelRole, ModelStatus } from "../contracts/settings";
import type { ModelPort } from "../data/settingsPorts";
import { MODEL_PRESENCE_LABEL, formatBytes, modelPresence } from "./storageFacts";

export interface ModelPanelProps {
  port: ModelPort;
}

/** 用途的中文名。取值域来自 `ModelKind`，不是自由字符串。 */
const KIND_LABEL: Record<ModelKind, string> = {
  asr: "语音识别",
  tts: "语音合成",
};

/** 是否进产品路径。`smoke` 的存在必须显示：它不是给用户用的。 */
const ROLE_LABEL: Record<ModelRole, string> = {
  production: "产品路径",
  smoke: "仅构建冒烟",
};

export default function ModelPanel({ port }: ModelPanelProps) {
  const [models, setModels] = useState<ModelStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void port
      .listModels()
      .then((next) => {
        if (!disposed) {
          setModels(next);
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
  }, [port]);

  return (
    <section className="settings-section" aria-label="语音模型">
      <h2 className="settings-section__title">语音模型</h2>
      <p className="settings-section__note">
        不随包分发任何模型权重，全部按需下载，且只接受经核实的 MIT 或 Apache-2.0 许可。
        体积是压缩包大小。
      </p>

      {models === null && error === null && (
        <p className="settings-section__note" data-testid="models-loading">
          正在读取模型清单……
        </p>
      )}

      {models !== null && models.length === 0 && (
        <p className="settings-section__note" data-testid="models-empty">
          清单里没有模型条目。
        </p>
      )}

      {models !== null && models.length > 0 && (
        <ul className="settings-list" data-testid="model-list">
          {models.map((model) => {
            const presence = modelPresence(model);
            return (
              <li className="settings-list__item" key={model.name} data-testid="model-row">
                <span className="settings-list__name">{model.name}</span>
                <span className="settings-list__meta">
                  <span data-testid="model-kind">{KIND_LABEL[model.kind]}</span>
                  <span data-testid="model-role">{ROLE_LABEL[model.role]}</span>
                  {/* 许可：每一行都有，`data-testid` 让「逐项都显示」这条可被断言。 */}
                  <span data-testid="model-license">许可 {model.license}</span>
                  <span data-testid="model-size">{formatBytes(model.size_bytes)}</span>
                  <span data-testid="model-presence">{MODEL_PRESENCE_LABEL[presence]}</span>
                  <span data-testid="model-attribution">署名 {model.attribution}</span>
                </span>
                {model.refused !== null && (
                  <p className="settings-list__refused" data-testid="model-refused">
                    许可门禁拒绝：{model.refused}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {error !== null && (
        <p className="settings-list__refused" role="alert" data-testid="models-error">
          {error}
        </p>
      )}
    </section>
  );
}
