/**
 * 「密钥实际存在哪里」这句话的推导。**本文件是这个 todo 的核心。**
 *
 * # 一句话：按 `persistence` 推导，不按 `backend` 名推导
 *
 * `docs/AI.zh.md:57-75` 把理由写成了产品契约：
 *
 * > **这条必须显式写出来，因为把它称作「系统钥匙串」是一句假话。**
 * > `Backend::Keyutils` 的文档原文是「Linux 内核 keyutils。**纯内存，重启即失效**」……
 * > `keyutils` 和 Secret Service 都是「钥匙串」，但**持久性根本不同**，这正是
 * > `StorageReport.persistence` 这个字段必须存在、且界面标签必须从它推导的全部理由。
 *
 * Linux 无头环境（CI、容器、纯 SSH 会话）下 Secret Service 必然不可用，
 * 降级链落到 keyutils——那是一个**真正的 keychain entry**，内核提供、总是可用，
 * 按后端名归类完全说得通。而它重启即失。所以「按后端名推导」这条错法既自然又致命：
 * 它产出的是一句**用户只有在丢了密钥之后才会发现的假话**。
 *
 * # 这条纪律在类型上落地，而不是靠注释提醒
 *
 * [`storageIndicator`] 的入参是 [`KeyStorageFacts`]，即
 * `Pick<StorageReport, "persistence" | "protection">`。
 * **`backend` 不在作用域里**，于是「按后端名拼文案」不是被禁止，而是写不出来——
 * 想那么做必须先把入参类型改宽，那是一次在 diff 里看得见的决定。
 *
 * 这是 todo 61 那条经验的同一路做法：**用类型让错误无从表达**，
 * 而不是依赖后来者记得某条规矩。
 *
 * # 与 Rust 侧的对应关系
 *
 * 本函数是 `StorageReport::settings_summary()`（`crates/yunjian-ai/src/keystore.rs:167-199`）
 * 在前端的镜像，分支顺序**逐一照抄**，因为顺序本身承载语义：
 *
 * | # | Rust 分支 | 本文件 | 指示串 |
 * | --- | --- | --- | --- |
 * | 1 | `(Persistent, OsEncrypted)` | 同 | `系统钥匙串（持久）` |
 * | 2 | `(LoginSession, _)` | 同 | `系统密钥环（重启后失效）` |
 * | 3 | `(ProcessOnly, _)` | 同 | `仅本次会话` |
 * | 4 | `(None, _)` | 同 | `尚未存储` |
 * | 5 | `(Persistent, Plaintext)` | 同 | `明文配置文件` |
 * | 6 | `(Persistent, ProcessMemory)` | 同 | `保护方式与持久性不一致` |
 *
 * 注意 `protection` 只在 `persistence === "persistent"` 这一族里起区分作用
 * （分支 1 / 5 / 6）。非持久的两族（分支 2、3）无论受什么保护都不许出现「持久」二字，
 * 所以那两支刻意通配 `protection`。「由 persistence 推导」这句话的确切含义就是这个：
 * **persistence 选定族，protection 只在族内消歧，backend 从不参与。**
 */

import type {
  KeyPersistence,
  KeyProtection,
  StorageIndicator,
  StorageReport,
} from "../contracts/settings";

/**
 * 推导指示串所需、且**仅需**的两个字段。
 *
 * 刻意不是 `StorageReport`：那会把 `backend` 带进作用域，
 * 而本函数的全部价值就在于它看不见后端名。
 */
export type KeyStorageFacts = Pick<StorageReport, "persistence" | "protection">;

/**
 * 由存储事实推导出面向用户的指示串。
 *
 * 参数刻意不含 `backend`，理由见文件头。
 */
export function storageIndicator(facts: KeyStorageFacts): StorageIndicator {
  const persistence: KeyPersistence = facts.persistence;
  const protection: KeyProtection = facts.protection;

  // 分支 4：什么都没存。先判它是因为此时另外两个字段描述的是「假如存了会在哪」，
  // 不是「密钥现在受什么保护」——拿它们去选族会替一个不存在的密钥宣布位置。
  if (persistence === "none") {
    return "尚未存储";
  }

  // 分支 2：活到注销或重启。keyutils 属于这一档。
  // **这一支通配 protection**：keyutils 是 os_encrypted，但加密与能活多久无关，
  // 让 protection 在这里参与判断就会把它拐回「系统钥匙串」。
  if (persistence === "login_session") {
    return "系统密钥环（重启后失效）";
  }

  // 分支 3：活到本进程退出。
  if (persistence === "process_only") {
    return "仅本次会话";
  }

  // 到这里 persistence 必然是 "persistent"。持久这一族才需要看 protection：
  // 「能活过重启」有三种代价完全不同的实现方式，混成一句话就抹掉了明文的风险。
  switch (protection) {
    // 分支 1：只有这一个组合允许出现「系统钥匙串」四个字。
    case "os_encrypted":
      return "系统钥匙串（持久）";
    // 分支 5：显式 opt-in 的明文文件。持久，但未加密。
    case "plaintext":
      return "明文配置文件";
    // 分支 6：声称持久却只在进程内存里。当前实现出不来这个组合，
    // 但它在类型上可表达，因此必须有落点——落进上面任何一支都是在替它背书。
    case "process_memory":
      return "保护方式与持久性不一致";
  }
}

/**
 * 指示串是否声称密钥能活过重启。
 *
 * 供界面决定要不要提示「下次启动需要重新输入」，也供测试直接钉住
 * 「keyutils 那一档不得声称持久」这条断言。
 */
export function claimsDurability(indicator: StorageIndicator): boolean {
  return indicator === "系统钥匙串（持久）" || indicator === "明文配置文件";
}
