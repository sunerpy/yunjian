/**
 * 语料状态面板：版本、记录数、取用/更新动作。
 *
 * # 「没有语料库」不是错误
 *
 * README 说明随包语料要下载 211 MiB，所以首次启动必然是 `absent`。
 * 把它渲染成错误横幅会让每个新用户第一眼看到一条红字。这里是一句陈述加一个按钮。
 *
 * # 字段名从 `CorpusMeta` 抄来
 *
 * 版本是 `corpus_version`，记录数是 **`poem_count`**（不是 `record_count`）。
 * 另外三个字段（`index_detail_mode` / `derived_indexes` / `shipped_scope`）也一起显示：
 * 「检索结构首启本机派生」是一个用户能感知到的行为（约 10 分钟），
 * 只显示版本与首数会让那件事变成一个没有解释的卡顿。
 */

import { useCallback, useEffect, useState } from "react";
import type { CorpusStatus } from "../contracts/settings";
import type { CorpusPort } from "../data/settingsPorts";

export interface CorpusPanelProps {
  port: CorpusPort;
}

export default function CorpusPanel({ port }: CorpusPanelProps) {
  const [status, setStatus] = useState<CorpusStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void port
      .corpusStatus()
      .then((next) => {
        if (!disposed) {
          setStatus(next);
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

  const onFetch = useCallback(() => {
    setBusy(true);
    setError(null);
    void port
      .fetchCorpus()
      .then(setStatus)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        setBusy(false);
      });
  }, [port]);

  return (
    <section className="settings-section" aria-label="语料库">
      <h2 className="settings-section__title">语料库</h2>

      {status === null && error === null && (
        <p className="settings-section__note" data-testid="corpus-loading">
          正在读取语料库状态……
        </p>
      )}

      {status !== null && status.kind === "absent" && (
        <p className="settings-section__note" data-testid="corpus-absent">
          尚未下载语料库。检索与阅读需要它；下载后检索结构会在首次启动时在本机派生。
        </p>
      )}

      {status !== null && status.kind === "ready" && (
        <dl className="settings-facts" data-testid="corpus-facts">
          <dt>语料版本</dt>
          <dd data-testid="corpus-version">{status.meta.corpus_version}</dd>
          <dt>收录作品</dt>
          {/* `toLocaleString` 给千分位：47 万这个量级不分组就读不出来。 */}
          <dd className="settings-facts__number" data-testid="corpus-poem-count">
            {status.meta.poem_count.toLocaleString("zh-CN")} 首
          </dd>
          <dt>schema 版本</dt>
          <dd className="settings-facts__number" data-testid="corpus-schema-version">
            {status.meta.schema_version}
          </dd>
          <dt>构建时间</dt>
          <dd data-testid="corpus-built-at">{status.meta.built_at}</dd>
          <dt>索引模式</dt>
          <dd data-testid="corpus-index-mode">{status.meta.index_detail_mode}</dd>
          <dt>派生索引</dt>
          <dd data-testid="corpus-derived-indexes">{status.meta.derived_indexes}</dd>
          <dt>随包范围</dt>
          <dd data-testid="corpus-shipped-scope">{status.meta.shipped_scope}</dd>
        </dl>
      )}

      <div className="settings-actions">
        <button
          type="button"
          className="settings-button"
          data-testid="fetch-corpus"
          disabled={busy}
          onClick={onFetch}>
          {status !== null && status.kind === "ready" ? "检查更新" : "下载语料库"}
        </button>
      </div>

      {error !== null && (
        <p className="settings-list__refused" role="alert" data-testid="corpus-error">
          {error}
        </p>
      )}
    </section>
  );
}
