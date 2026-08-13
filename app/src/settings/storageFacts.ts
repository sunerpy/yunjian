/**
 * 明文存储的告警文案，以及模型下载状态的人话化。
 *
 * 两者都是「把后端给的事实翻成一句话」，都刻意不引入后端没有的概念。
 */

import type { ModelStatus, StorageReport } from "../contracts/settings";

/** 明文告警只需要这两个字段。同样不含 `backend`。 */
export type PlaintextFacts = Pick<StorageReport, "protection" | "location">;

/**
 * 明文存储的告警文案；非明文时为 `null`。
 *
 * 措辞**逐字取自** `StorageReport::plaintext_warning()`
 * （`crates/yunjian-ai/src/keystore.rs:201-210`），不自行改写：
 *
 * ```rust
 * "密钥以明文保存在 {}。该文件未加密，仅靠文件权限保护；\
 *  一旦被备份、同步或打包带走，密钥即随之泄露。"
 * ```
 *
 * 判据是 `protection === "plaintext"` 而不是 `backend === "plaintext_file"`：
 * 与指示串同一条纪律——按事实判，不按后端名判。
 */
export function plaintextWarning(facts: PlaintextFacts): string | null {
  if (facts.protection !== "plaintext") {
    return null;
  }
  return (
    `密钥以明文保存在 ${facts.location}。该文件未加密，仅靠文件权限保护；` +
    "一旦被备份、同步或打包带走，密钥即随之泄露。"
  );
}

/** 模型在本机的三种处境。**不是** Rust 侧的类型，是两个布尔的显示投影。 */
export type ModelPresence = "unpacked" | "archived_only" | "missing";

/**
 * 由 `unpacked` / `archived` 两位推导本机处境。
 *
 * Rust 侧没有 `DownloadState` 枚举（`crates/yunjian-voice/src/models.rs:654-675` 只有
 * 这两个布尔），所以这里只做投影，不发明一个后端给不出的状态机。
 *
 * `unpacked` 优先：解包目录就位就说明模型可用，归档还在不在只影响能不能免下载重装。
 */
export function modelPresence(model: Pick<ModelStatus, "unpacked" | "archived">): ModelPresence {
  if (model.unpacked) {
    return "unpacked";
  }
  return model.archived ? "archived_only" : "missing";
}

/** 三种处境各自的中文说明。 */
export const MODEL_PRESENCE_LABEL: Record<ModelPresence, string> = {
  unpacked: "已就位",
  archived_only: "已下载，未解包",
  missing: "未下载",
};

/**
 * 字节数的人类可读形态。
 *
 * 用 1024 进制并标 KiB / MiB / GiB，与 README 里「gzip 211 MiB」「下载 211 MiB」的写法一致。
 * 混用 MB 与 MiB 会让同一个文件在两处显示成两个数。
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "未知";
  }
  const units = ["B", "KiB", "MiB", "GiB", "TiB"] as const;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // 整字节不显示小数：`117 B` 比 `117.0 B` 更像一个准确值。
  const rendered = unit === 0 ? String(Math.round(value)) : value.toFixed(1);
  return `${rendered} ${units[unit]}`;
}
