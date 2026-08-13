/**
 * 诚实链的核心断言：指示串由 `persistence` 推导，绝不由 `backend` 名推导。
 *
 * # 这一组为什么要穷举 `(backend, persistence)` 而不是只测 persistence
 *
 * 只测 `persistence` 的话，一个「按 backend 名推导」的实现照样能全绿——只要它恰好对
 * 被测的那几个后端给出了对的答案。真正要钉住的是**同一个 persistence 下不同后端必须给出
 * 同一个答案**，以及**同一个后端在不同 persistence 下必须给出不同答案**。
 * 所以下面这张表逐组合列出，且额外有两条方向相反的断言。
 *
 * # 诚实链有两面，两面都要测
 *
 * `.omo/notepads/yunjian/learnings.md` 记着 todo 38 得出的这条：
 *
 * > keyutils 那条测「**不得**说系统钥匙串」，Windows 凭据管理器那条测「**应当**说系统钥匙串」。
 * > 只测前者的话，把 `settings_summary` 写成永远不说「系统钥匙串」也能全绿——那同样是在骗用户。
 *
 * 本文件照此办：`keyutils_*` 那条断言不含「持久」，`windows_credential` 那条断言含。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { KeyBackend, KeyPersistence, KeyProtection } from "../../contracts/settings";
import { STORAGE_INDICATOR_DETAIL } from "../../contracts/settings";
import { claimsDurability, storageIndicator } from "../storageIndicator";

/**
 * `(backend, persistence, protection)` → 期望指示串。
 *
 * 三列的取值全部来自 `crates/yunjian-ai/src/keystore.rs`，`persistence` 那一列照
 * `from_credential_persistence`（`keystore.rs:107-125`）与 todo 38 的实测结论填：
 * Secret Service / Windows / Apple / Android 四个 store 实测都返回 `UntilDelete`（→ 持久），
 * keyutils 实测返回 `UntilReboot`（→ `login_session`），
 * keyring-core 的 `mock::Store` 实测返回 `ProcessOnly`。
 */
const CASES: {
  backend: KeyBackend;
  persistence: KeyPersistence;
  protection: KeyProtection;
  expected: string;
}[] = [
  // ── 四个真正持久的操作系统钥匙串 ──
  {
    backend: "secret_service",
    persistence: "persistent",
    protection: "os_encrypted",
    expected: "系统钥匙串（持久）",
  },
  {
    backend: "windows_credential",
    persistence: "persistent",
    protection: "os_encrypted",
    expected: "系统钥匙串（持久）",
  },
  {
    backend: "apple_keychain",
    persistence: "persistent",
    protection: "os_encrypted",
    expected: "系统钥匙串（持久）",
  },
  {
    backend: "android_keystore",
    persistence: "persistent",
    protection: "os_encrypted",
    expected: "系统钥匙串（持久）",
  },
  // ── 这一行是整个 todo 的理由 ──
  // keyutils 是一个真正的 keychain entry（内核提供、总是可用、os_encrypted），
  // 但它纯内存、重启即失。按后端名推导会把它报成「系统钥匙串（持久）」。
  {
    backend: "keyutils",
    persistence: "login_session",
    protection: "os_encrypted",
    expected: "系统密钥环（重启后失效）",
  },
  // ── 本会话内存 ──
  {
    backend: "session_memory",
    persistence: "process_only",
    protection: "process_memory",
    expected: "仅本次会话",
  },
  // ── 显式 opt-in 的明文文件：持久，但未加密 ──
  {
    backend: "plaintext_file",
    persistence: "persistent",
    protection: "plaintext",
    expected: "明文配置文件",
  },
  // ── 什么都没存。`backend: absent` 时 persistence 一律 none。 ──
  {
    backend: "absent",
    persistence: "none",
    protection: "os_encrypted",
    expected: "尚未存储",
  },
];

describe("存储位置指示串", () => {
  it.each(CASES)(
    "$backend / $persistence / $protection → $expected",
    ({ persistence, protection, expected }) => {
      expect(storageIndicator({ persistence, protection })).toBe(expected);
    },
  );

  it("八个后端全部被覆盖，一个不漏", () => {
    // 漏掉一个后端就等于那一档的文案从未被验证过，而这类漏洞只有在真机上才暴露。
    const covered = new Set(CASES.map((entry) => entry.backend));
    const all: KeyBackend[] = [
      "secret_service",
      "keyutils",
      "windows_credential",
      "apple_keychain",
      "android_keystore",
      "session_memory",
      "plaintext_file",
      "absent",
    ];
    expect([...all].filter((backend) => !covered.has(backend))).toEqual([]);
  });

  it("keyutils 绝不渲染「持久」二字", () => {
    // 本 todo 最核心的一条。它同时守住三种说错法：字面「持久」、
    // 「系统钥匙串」这个词，以及 `claimsDurability` 这个语义判断。
    const indicator = storageIndicator({
      persistence: "login_session",
      protection: "os_encrypted",
    });
    expect(indicator).not.toContain("持久");
    expect(indicator).not.toContain("系统钥匙串");
    expect(claimsDurability(indicator)).toBe(false);
    // 详情行也不许暗示它能活过重启。
    expect(STORAGE_INDICATOR_DETAIL[indicator]).toContain("重启");
    expect(STORAGE_INDICATOR_DETAIL[indicator]).toContain("失效");
  });

  it("Windows 凭据管理器则**应当**渲染「系统钥匙串」", () => {
    // 反方向的断言。缺了它，一个「永远不说系统钥匙串」的实现也能全绿——
    // 那同样是在骗用户，只是骗的方向相反。
    const indicator = storageIndicator({
      persistence: "persistent",
      protection: "os_encrypted",
    });
    expect(indicator).toBe("系统钥匙串（持久）");
    expect(claimsDurability(indicator)).toBe(true);
  });

  it("同一个 persistence 下，不同后端得到同一个指示串", () => {
    // 「按 persistence 推导」的正面含义：后端换了但持久性没换，结论就不该变。
    const durable = CASES.filter((entry) => entry.expected === "系统钥匙串（持久）");
    expect(durable.length).toBeGreaterThan(1);
    const results = new Set(
      durable.map((entry) =>
        storageIndicator({ persistence: entry.persistence, protection: entry.protection }),
      ),
    );
    expect(results.size).toBe(1);
  });

  it("非持久的两档无论受什么保护都不许声称持久", () => {
    // `protection` 只在 persistent 一族里消歧。让它在这两族里参与判断，
    // 就会把 os_encrypted 的 keyutils 拐回「系统钥匙串」。
    for (const protection of ["os_encrypted", "process_memory", "plaintext"] as const) {
      for (const persistence of ["login_session", "process_only"] as const) {
        const indicator = storageIndicator({ persistence, protection });
        expect(claimsDurability(indicator), `${persistence}/${protection} 声称了持久`).toBe(false);
      }
    }
  });

  it("四种任务规定的指示串各自可达，且互不相同", () => {
    const required = [
      "系统钥匙串（持久）",
      "系统密钥环（重启后失效）",
      "仅本次会话",
      "明文配置文件",
    ];
    const reached = required.map(
      (want) => CASES.find((entry) => entry.expected === want)?.expected,
    );
    expect(reached).toEqual(required);
    expect(new Set(required).size).toBe(4);
  });

  it("每种指示串都有自己的详情行，没有一条落空", () => {
    for (const entry of CASES) {
      const indicator = storageIndicator({
        persistence: entry.persistence,
        protection: entry.protection,
      });
      expect(STORAGE_INDICATOR_DETAIL[indicator]).toBeTruthy();
    }
  });

  it("类型上可表达的第六种组合也有落点，不冒充钥匙串", () => {
    // `(persistent, process_memory)` 当前实现出不来，但它在类型上存在。
    // Rust 侧对它的处置是「按不可信处理」，这里必须一致——落进「系统钥匙串」
    // 就等于替一个自相矛盾的报告背书。
    const indicator = storageIndicator({
      persistence: "persistent",
      protection: "process_memory",
    });
    expect(indicator).toBe("保护方式与持久性不一致");
    expect(indicator).not.toContain("系统钥匙串");
  });
});

describe("推导函数看不见后端名（源码守卫）", () => {
  const source = readFileSync(resolve(process.cwd(), "src/settings/storageIndicator.ts"), "utf8");

  it("`backend` 一词只出现在注释里，不出现在任何表达式里", () => {
    // 入参类型是 `Pick<StorageReport, "persistence" | "protection">`，
    // 所以 `facts.backend` 编译期就不存在。这条守卫拦的是另一种改法：
    // 有人把入参放宽成 `StorageReport` 再去读它。
    const code = source
      // 去掉块注释与行注释，只留可执行部分。
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .replace(/^\s*\/\/.*$/gm, "");
    expect(code).not.toMatch(/\.\s*backend/);
    expect(code).not.toMatch(/"(secret_service|keyutils|windows_credential|plaintext_file)"/);
  });

  it("入参类型确实只取两个字段", () => {
    expect(source).toMatch(/Pick<StorageReport,\s*"persistence"\s*\|\s*"protection">/);
  });
});
