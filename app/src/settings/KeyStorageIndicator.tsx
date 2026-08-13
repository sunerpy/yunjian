/**
 * 存储位置指示条：把 [`StorageReport`] 翻成用户看得懂的一句话。
 *
 * # 这个组件的全部价值在于它读哪个字段
 *
 * 它把 `report` 交给 [`storageIndicator`]，而那个函数的入参类型是
 * `Pick<StorageReport, "persistence" | "protection">`——**后端名进不去**。
 *
 * 本组件确实显示 `backend`，但只作为最弱一档的诊断信息（`.settings-storage__backend`），
 * 且它与指示串的推导毫无关系。这一点值得说清楚：显示后端名是有用的
 * （用户报问题时那是关键线索），把它**当作结论的依据**才是错的。
 */

import type { StorageReport } from "../contracts/settings";
import { STORAGE_INDICATOR_DETAIL } from "../contracts/settings";
import { plaintextWarning } from "./storageFacts";
import { storageIndicator } from "./storageIndicator";

export interface KeyStorageIndicatorProps {
  report: StorageReport;
}

export default function KeyStorageIndicator({ report }: KeyStorageIndicatorProps) {
  // 只把两个字段交出去。写成 `storageIndicator(report)` 也能编译（结构子类型），
  // 但显式解构让「本推导只用这两个字段」在调用点也看得见。
  const indicator = storageIndicator({
    persistence: report.persistence,
    protection: report.protection,
  });
  const warning = plaintextWarning({
    persistence: report.persistence,
    protection: report.protection,
    location: report.location,
  });

  return (
    <>
      <div className="settings-storage" data-testid="key-storage">
        <span className="settings-storage__indicator" data-testid="key-storage-indicator">
          {indicator}
        </span>
        <span className="settings-storage__detail" data-testid="key-storage-detail">
          {STORAGE_INDICATOR_DETAIL[indicator]}
        </span>
        <span className="settings-storage__location" data-testid="key-storage-location">
          位置：{report.location}
        </span>
        {/* 后端名：诊断用，最弱一档。它不参与上面那句话的推导。 */}
        <span className="settings-storage__backend" data-testid="key-storage-backend">
          backend={report.backend} persistence={report.persistence} protection={report.protection}
        </span>
      </div>

      {warning !== null && (
        <div
          className="settings-alert"
          role="alert"
          data-testid="plaintext-warning"
          data-mood={warning.mood}>
          <span className="settings-alert__badge" data-testid="plaintext-warning-badge">
            {/* 图标不只是装饰：告警此前**只靠颜色加文字**承载语义，
                对色觉障碍用户等于少一层冗余。`aria-hidden` 是因为紧邻的文字已经说了同一件事，
                读屏再念一遍符号只会变成噪音。 */}
            <span aria-hidden="true">⚠</span> 明文存储警告
          </span>
          <p className="settings-alert__text">{warning.text}</p>
        </div>
      )}
    </>
  );
}
