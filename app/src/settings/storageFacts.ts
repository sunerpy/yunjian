/**
 * 明文存储的告警文案，以及模型下载状态的人话化。
 *
 * 两者都是「把后端给的事实翻成一句话」，都刻意不引入后端没有的概念。
 */

import type { ModelStatus, StorageReport } from "../contracts/settings";

/**
 * 明文告警需要的三个字段。同样不含 `backend`。
 *
 * **`persistence` 是后加的，加它的理由是一次真实的假话**：见 [`plaintextWarning`]。
 */
export type PlaintextFacts = Pick<StorageReport, "persistence" | "protection" | "location">;

/** 告警的两种语气。陈述句只在密钥确实存在时使用。 */
export type WarningMood = "actual" | "prospective";

export interface PlaintextWarning {
  mood: WarningMood;
  text: string;
}

/**
 * 明文存储的告警；非明文时为 `null`。
 *
 * # 语气必须随密钥是否存在而变，否则它自己就是一句假话
 *
 * 第一版只看 `protection`，于是在**尚未保存任何密钥**时也照着 Rust 的陈述句渲染
 * 「密钥以明文保存在 …」。那一屏上同时出现三句话：
 *
 * > 尚未保存任何密钥，需要输入密钥。（指示条）
 * > 密钥**以明文保存在** ~/.config/yunjian/keys.toml。（告警框）
 * > 当前没有可用的密钥，需要重新输入。（指示条下方）
 *
 * **在没有密钥的时候断言「密钥已明文保存」，与把 keyutils 报成持久是同一类假话，
 * 只是方向相反**：前者夸大风险、后者隐瞒风险，两者都让界面说了一件它没核实过的事。
 * 这个缺陷只有在真浏览器里看整屏文案才会暴露——单测各自断言一句话时，
 * 每一句单看都是对的。
 *
 * 所以按 `persistence` 分两种语气：
 *
 * - `persistence !== "none"`（密钥确实在那儿）→ `actual`，措辞**逐字取自**
 *   `StorageReport::plaintext_warning()`（`crates/yunjian-ai/src/keystore.rs:201-210`）；
 * - `persistence === "none"`（还没有密钥）→ `prospective`，条件式措辞。
 *   **刻意仍然显示**而不是隐藏：首启就告知代价，比存进去之后才警告有用得多——
 *   那时用户已经付出了这个代价。
 *
 * 判据始终是 `protection === "plaintext"` 而非 `backend === "plaintext_file"`：
 * 与指示串同一条纪律——按事实判，不按后端名判。
 */
export function plaintextWarning(facts: PlaintextFacts): PlaintextWarning | null {
  if (facts.protection !== "plaintext") {
    return null;
  }
  const consequence =
    "该文件未加密，仅靠文件权限保护；一旦被备份、同步或打包带走，密钥即随之泄露。";
  if (facts.persistence === "none") {
    return {
      mood: "prospective",
      text: `若在此保存密钥，它将以明文写入 ${facts.location}。${consequence}`,
    };
  }
  return {
    mood: "actual",
    text: `密钥以明文保存在 ${facts.location}。${consequence}`,
  };
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
