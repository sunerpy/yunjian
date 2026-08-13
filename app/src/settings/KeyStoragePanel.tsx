/**
 * AI 服务商与密钥面板。
 *
 * # 密钥输入的三条硬约束，逐条对应一个实现手段
 *
 * 1. **遮罩显示** — `type="password"`。
 * 2. **永不记录日志** — 本组件不含任何 `console.*`；`docs/AI.zh.md:91` 记的是 Rust 侧
 *    「平台错误先经 `redact_credentials(...)` 才进 `tracing::info!`」，前端这一侧对应的
 *    做法就是**根本不写那行日志**。保存失败时显示的是端口返回的错误消息，
 *    而那条消息来自已脱敏的 Rust 错误。
 * 3. **保存后永不回显** — 两层：
 *    - **类型层**：`KeyStorePort` 没有任何能取回密钥的方法（见 `data/settingsPorts.ts`），
 *      所以已保存的密钥**没有可读来源**，回显不是被禁止而是写不出来；
 *    - **状态层**：草稿在保存成功后清空，输入框因此为空。
 *
 *    第二层单独存在是不够的（「记得清空」是纪律，会被后来者的重构抹掉），
 *    第一层单独存在也不够（草稿本身在用户输入期间就在 DOM 里）。两层都要有。
 *
 * # 端点与模型留空是正常状态
 *
 * `AiConfig.endpoint` 与 `model` 在 Rust 侧都是 `Option<String>`，留空即「用服务商默认」。
 * 所以这两个输入框的 placeholder 显示的是 `default_base_url()` / `default_model()` 的实际值，
 * **但不把它们写进配置**——把默认值物化进配置文件会让上游改默认时用户被钉在旧值上。
 */

import { useCallback, useEffect, useState } from "react";
import type { AiSettings, KeyStatus, ProviderId, StorageReport } from "../contracts/settings";
import {
  PROVIDER_DEFAULTS,
  PROVIDER_IDS,
  PROVIDER_LABEL,
  PROVIDER_NONE,
} from "../contracts/settings";
import type { AiSettingsPort, KeyStorePort } from "../data/settingsPorts";
import KeyStorageIndicator from "./KeyStorageIndicator";

export interface KeyStoragePanelProps {
  keyStorePort: KeyStorePort;
  aiSettingsPort: AiSettingsPort;
}

function isProviderId(value: string): value is ProviderId {
  return (PROVIDER_IDS as readonly string[]).includes(value);
}

export default function KeyStoragePanel({ keyStorePort, aiSettingsPort }: KeyStoragePanelProps) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [status, setStatus] = useState<KeyStatus | null>(null);
  // 草稿。**它是唯一持有密钥的地方，且只在用户输入到保存之间存在。**
  const [draft, setDraft] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const loaded = await aiSettingsPort.readAiSettings();
        if (disposed) {
          return;
        }
        setSettings(loaded);
        const current = await keyStorePort.keyStatus(loaded.provider);
        if (!disposed) {
          setStatus(current);
        }
      } catch (cause) {
        if (!disposed) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      }
    })();
    // `disposed` 守卫的理由与 `chrome/useWindowChrome.ts` 相同：StrictMode 会二次调用
    // effect，卸载后再写状态会被 React 判为泄漏。
    return () => {
      disposed = true;
    };
  }, [aiSettingsPort, keyStorePort]);

  const applySettings = useCallback(
    (next: AiSettings) => {
      setSettings(next);
      void aiSettingsPort.writeAiSettings(next).catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      });
    },
    [aiSettingsPort],
  );

  const onProviderChange = useCallback(
    (provider: string) => {
      if (settings === null) {
        return;
      }
      applySettings({ ...settings, provider });
      // 切服务商就是换钥匙串里的 account，报告必须跟着换；
      // 沿用旧报告会让界面对新服务商的密钥位置说一句它没查过的话。
      setStatus(null);
      setNotice(null);
      // 切换时清空草稿：把 A 服务商的 key 留在框里、用户按下保存就写进了 B。
      setDraft("");
      void keyStorePort
        .keyStatus(provider)
        .then(setStatus)
        .catch((cause: unknown) => {
          setError(cause instanceof Error ? cause.message : String(cause));
        });
    },
    [applySettings, keyStorePort, settings],
  );

  const onSave = useCallback(() => {
    if (settings === null || draft === "") {
      return;
    }
    setError(null);
    void keyStorePort
      .setKey(settings.provider, draft)
      .then((report: StorageReport) => {
        setStatus({ report, needs_reprompt: false });
        // 清空发生在拿到报告之后：保存失败时草稿要留着，否则用户得重新粘一遍。
        setDraft("");
        setNotice("密钥已保存。");
      })
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      });
  }, [draft, keyStorePort, settings]);

  const onDelete = useCallback(() => {
    if (settings === null) {
      return;
    }
    setError(null);
    void keyStorePort
      .deleteKey(settings.provider)
      .then((report: StorageReport) => {
        setStatus({ report, needs_reprompt: true });
        setDraft("");
        setNotice("密钥已删除。");
      })
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      });
  }, [keyStorePort, settings]);

  if (settings === null) {
    return (
      <section className="settings-section" aria-label="AI 服务商与密钥">
        <h2 className="settings-section__title">AI 服务商与密钥</h2>
        <p className="settings-section__note" data-testid="key-panel-loading">
          正在读取配置……
        </p>
        {error !== null && (
          <p className="settings-list__refused" role="alert" data-testid="key-panel-error">
            {error}
          </p>
        )}
      </section>
    );
  }

  const provider = settings.provider;
  const defaults = isProviderId(provider) ? PROVIDER_DEFAULTS[provider] : null;

  return (
    <section className="settings-section" aria-label="AI 服务商与密钥">
      <h2 className="settings-section__title">AI 服务商与密钥</h2>
      <p className="settings-section__note">
        云笺是自带密钥（BYOK）：请求直连你选择的服务商，本项目不代理请求、不持有凭据。
        没有密钥也完整可用——常见名篇随包带有预生成的赏析。
      </p>

      <div className="settings-field">
        <label className="settings-field__label" htmlFor="settings-provider">
          服务商
        </label>
        <select
          id="settings-provider"
          className="settings-field__control"
          data-testid="provider-select"
          value={provider}
          onChange={(event) => {
            onProviderChange(event.target.value);
          }}>
          <option value={PROVIDER_NONE}>不配置（只用随包赏析）</option>
          {PROVIDER_IDS.map((id) => (
            <option key={id} value={id}>
              {PROVIDER_LABEL[id]}
            </option>
          ))}
        </select>
      </div>

      <div className="settings-field">
        <label className="settings-field__label" htmlFor="settings-endpoint">
          自定义 base URL
        </label>
        <input
          id="settings-endpoint"
          className="settings-field__control"
          data-testid="endpoint-input"
          type="url"
          value={settings.endpoint ?? ""}
          placeholder={defaults === null ? "先选择服务商" : defaults.base_url}
          onChange={(event) => {
            const raw = event.target.value.trim();
            applySettings({ ...settings, endpoint: raw === "" ? null : raw });
          }}
        />
        <span className="settings-field__hint">留空即使用服务商默认地址。不要把密钥写进 URL。</span>
      </div>

      <div className="settings-field">
        <label className="settings-field__label" htmlFor="settings-model">
          模型
        </label>
        <input
          id="settings-model"
          className="settings-field__control"
          data-testid="model-input"
          type="text"
          value={settings.model ?? ""}
          placeholder={defaults === null ? "先选择服务商" : defaults.model}
          onChange={(event) => {
            const raw = event.target.value.trim();
            applySettings({ ...settings, model: raw === "" ? null : raw });
          }}
        />
        <span className="settings-field__hint">留空即使用该服务商的默认模型。</span>
      </div>

      <div className="settings-field">
        <label className="settings-field__label" htmlFor="settings-api-key">
          API Key
        </label>
        <input
          id="settings-api-key"
          className="settings-field__control settings-field__control--secret"
          data-testid="api-key-input"
          // 遮罩。加上关掉自动填充与拼写检查：两者都会把密钥交给浏览器的其它子系统。
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={draft}
          placeholder={
            status !== null && !status.needs_reprompt ? "已保存（不回显）" : "粘贴你的 API Key"
          }
          onChange={(event) => {
            setDraft(event.target.value);
            setNotice(null);
          }}
        />
        <span className="settings-field__hint" data-testid="api-key-hint">
          已保存的密钥不会被显示出来——界面没有读取它的途径。要更换就直接粘贴新的。
        </span>
      </div>

      <div className="settings-actions">
        <button
          type="button"
          className="settings-button"
          data-testid="save-key"
          disabled={draft === ""}
          onClick={onSave}>
          保存密钥
        </button>
        <button
          type="button"
          className="settings-button"
          data-testid="delete-key"
          onClick={onDelete}>
          删除密钥
        </button>
      </div>

      {notice !== null && (
        <p className="settings-section__note" data-testid="key-notice">
          {notice}
        </p>
      )}
      {error !== null && (
        <p className="settings-list__refused" role="alert" data-testid="key-panel-error">
          {error}
        </p>
      )}

      {status === null ? (
        <p className="settings-section__note" data-testid="key-storage-pending">
          正在查询密钥存储位置……
        </p>
      ) : (
        <>
          <KeyStorageIndicator report={status.report} />
          {status.needs_reprompt && (
            <p className="settings-section__note" data-testid="key-needs-reprompt">
              当前没有可用的密钥，需要重新输入。
            </p>
          )}
        </>
      )}
    </section>
  );
}
