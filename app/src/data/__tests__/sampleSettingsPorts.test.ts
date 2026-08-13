/**
 * 样例宿主的存储档位开关：四档都必须真能到达。
 *
 * # 为什么这一组必须存在
 *
 * `?sample-key-tier=` 是浏览器目视验收唯一的入口——尤其是明文那一档，它是四档里唯一
 * 会显示告警的。若某个档位名拼错或映射漏了，症状是「打开那个 URL 什么也没变」，
 * 而那在任何组件测试里都看不见（组件测试直接注入 report，绕过了这个开关）。
 */

import { describe, expect, it } from "vitest";
import { storageIndicator } from "../../settings/storageIndicator";
import { createSampleSettingsPorts } from "../sampleSettingsPorts";

/** 把 jsdom 的 URL 换成带指定查询参数的地址。 */
function withTier(tier: string | null): void {
  const search = tier === null ? "" : `?sample-key-tier=${tier}`;
  window.history.replaceState({}, "", `/index.html${search}`);
}

describe("样例宿主的存储档位", () => {
  it("不给参数时是空柜，指示串为「尚未存储」", async () => {
    withTier(null);
    const ports = createSampleSettingsPorts();
    const status = await ports.keyStore.keyStatus("deepseek");
    expect(status.needs_reprompt).toBe(true);
    expect(storageIndicator(status.report)).toBe("尚未存储");
  });

  it("不给参数保存后落到 keyutils 档——本机真实会到的那一档", async () => {
    withTier(null);
    const ports = createSampleSettingsPorts();
    const report = await ports.keyStore.setKey("deepseek", "sk-x");
    expect(report.backend).toBe("keyutils");
    expect(storageIndicator(report)).toBe("系统密钥环（重启后失效）");
  });

  it.each([
    ["keyutils", "系统密钥环（重启后失效）"],
    ["keychain", "系统钥匙串（持久）"],
    ["session", "仅本次会话"],
    ["plaintext", "明文配置文件"],
  ])("?sample-key-tier=%s 预置一枚密钥并渲染 %s", async (tier, expected) => {
    withTier(tier);
    const ports = createSampleSettingsPorts();
    const status = await ports.keyStore.keyStatus("deepseek");
    // 显式指定档位时预置密钥，于是打开 URL 就能看到指示串，不必先点保存。
    expect(status.needs_reprompt).toBe(false);
    expect(storageIndicator(status.report)).toBe(expected);
  });

  it("四档覆盖了任务规定的四种指示串，一个不漏", async () => {
    const seen = new Set<string>();
    for (const tier of ["keyutils", "keychain", "session", "plaintext"]) {
      withTier(tier);
      const status = await createSampleSettingsPorts().keyStore.keyStatus("deepseek");
      seen.add(storageIndicator(status.report));
    }
    expect([...seen].sort()).toEqual(
      ["仅本次会话", "明文配置文件", "系统密钥环（重启后失效）", "系统钥匙串（持久）"].sort(),
    );
  });

  it("无法识别的档位名回落到默认档，不抛异常也不给出空报告", async () => {
    withTier("does-not-exist");
    const ports = createSampleSettingsPorts();
    const status = await ports.keyStore.keyStatus("deepseek");
    // 回落到默认档且**不预置**：拼错档位名的人应该看到首启态，
    // 而不是一个看起来像「已生效」的预置密钥。
    expect(status.needs_reprompt).toBe(true);
    expect(status.report.location).toContain("keyutils");
  });

  it("预置的那枚密钥同样读不出来", async () => {
    withTier("plaintext");
    const ports = createSampleSettingsPorts();
    // 端口上根本没有读取密钥的方法，这里断言的是那件事在类型与运行时都成立。
    expect("getKey" in ports.keyStore).toBe(false);
    expect(Object.keys(ports.keyStore).sort()).toEqual(["deleteKey", "keyStatus", "setKey"]);
    const status = await ports.keyStore.keyStatus("deepseek");
    expect(JSON.stringify(status)).not.toContain("sample-preset");
  });
});
