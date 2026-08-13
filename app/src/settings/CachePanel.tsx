/**
 * 赏析缓存管理：体积与清理。
 *
 * # 随包层与本机层必须分开显示，且清理只动后者
 *
 * 两级缓存里随包那一层是产品资产（预生成赏析，让没有密钥的用户也能用），
 * 本机那一层是用户自费生成的结果。`AppreciationCache::purge` 的三个范围全部只
 * `DELETE FROM appreciation_cache`，**没有一个会碰 `appreciation_shipped`**。
 * 界面必须让这件事看得出来，否则「清理缓存」会被读成「删掉随包赏析」。
 *
 * # 体积可能是「未知」，那不是缺陷
 *
 * Rust 的缓存模块只报行数（`CacheCounts`），没有报字节数的函数。磁盘体积要靠 IPC 层
 * `stat` 缓存库文件才拿得到，而那是 todo 64 的实现选择。所以 `database_bytes` 缺省时
 * 显示「未知」——**不编一个数字**。这与 todo 61 对 `AppreciationView.source` 的处理同源。
 */

import { useCallback, useEffect, useState } from "react";
import type { CacheStatus, PurgeScope } from "../contracts/settings";
import type { CachePort } from "../data/settingsPorts";
import { formatBytes } from "./storageFacts";

export interface CachePanelProps {
  port: CachePort;
  /** 当前提示词模板版本，用于「只清理本模板」这一档。 */
  templateVersion: string;
}

export default function CachePanel({ port, templateVersion }: CachePanelProps) {
  const [status, setStatus] = useState<CacheStatus | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(
    (disposed?: () => boolean) => {
      void port
        .cacheStatus()
        .then((next) => {
          if (disposed?.() !== true) {
            setStatus(next);
          }
        })
        .catch((cause: unknown) => {
          if (disposed?.() !== true) {
            setError(cause instanceof Error ? cause.message : String(cause));
          }
        });
    },
    [port],
  );

  useEffect(() => {
    let disposed = false;
    reload(() => disposed);
    return () => {
      disposed = true;
    };
  }, [reload]);

  const purge = useCallback(
    (scope: PurgeScope) => {
      setError(null);
      void port
        .purgeCache(scope)
        .then((removed) => {
          setNotice(`已清理 ${removed} 条本机缓存。随包预生成的赏析未受影响。`);
          reload();
        })
        .catch((cause: unknown) => {
          setError(cause instanceof Error ? cause.message : String(cause));
        });
    },
    [port, reload],
  );

  return (
    <section className="settings-section" aria-label="赏析缓存">
      <h2 className="settings-section__title">赏析缓存</h2>
      <p className="settings-section__note">
        清理只删除你自己生成的那一层；随包预生成的赏析属于产品资产，任何清理范围都不会碰它。
      </p>

      {status === null && error === null && (
        <p className="settings-section__note" data-testid="cache-loading">
          正在读取缓存状态……
        </p>
      )}

      {status !== null && (
        <dl className="settings-facts" data-testid="cache-facts">
          <dt>随包预生成</dt>
          <dd className="settings-facts__number" data-testid="cache-shipped">
            {status.counts.shipped.toLocaleString("zh-CN")} 条
          </dd>
          <dt>本机生成</dt>
          <dd className="settings-facts__number" data-testid="cache-local">
            {status.counts.local.toLocaleString("zh-CN")} 条
          </dd>
          <dt>缓存库体积</dt>
          <dd className="settings-facts__number" data-testid="cache-bytes">
            {status.database_bytes === undefined ? "未知" : formatBytes(status.database_bytes)}
          </dd>
        </dl>
      )}

      <div className="settings-actions">
        <button
          type="button"
          className="settings-button"
          data-testid="purge-template"
          onClick={() => {
            purge({ kind: "template", template_version: templateVersion });
          }}>
          清理当前模板（{templateVersion}）
        </button>
        <button
          type="button"
          className="settings-button"
          data-testid="purge-all"
          onClick={() => {
            purge({ kind: "all" });
          }}>
          清理全部本机缓存
        </button>
      </div>

      {notice !== null && (
        <p className="settings-section__note" data-testid="cache-notice">
          {notice}
        </p>
      )}
      {error !== null && (
        <p className="settings-list__refused" role="alert" data-testid="cache-error">
          {error}
        </p>
      )}
    </section>
  );
}
