/**
 * 令牌层的浅色与深色必须逐个对应。
 *
 * # 这条断言的来历
 *
 * 它不是预防性设计，是一次目视 QA 抓出来的真实缺陷。第一版只往 `:root` 里加了
 * `--color-sourced-*` 与 `--color-ai-*`，忘了在 `prefers-color-scheme: dark` 里覆盖，
 * 于是深色下得到「深色页面背景 + 写死的浅色卡片 + 浅色正文」——原文几乎不可读。
 *
 * **这个缺陷在单元测试里完全看不见**：jsdom 不做样式计算，157 个测试全绿。
 * 它只在真的看一眼截图时才暴露，与 todo 65 的图标在 16 px 下糊成白斑是同一类问题。
 * 所以这里把「看一眼」得到的结论固化成机制：两处的令牌名集合必须相等。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");

/** 取 `:root { … }` 与深色 media query 内 `:root { … }` 的规则体。 */
function themeBlocks(): { light: string; dark: string } {
  const darkAt = css.indexOf("@media (prefers-color-scheme: dark)");
  expect(darkAt).toBeGreaterThan(0);
  return { light: css.slice(0, darkAt), dark: css.slice(darkAt) };
}

function tokenNames(source: string): Set<string> {
  return new Set(source.match(/--[a-z0-9-]+(?=\s*:)/g) ?? []);
}

describe("浅色与深色的令牌集合", () => {
  const { light, dark } = themeBlocks();
  const lightTokens = tokenNames(light);
  const darkTokens = tokenNames(dark);

  it("深色里声明的令牌，浅色里也都有", () => {
    const orphans = [...darkTokens].filter((token) => !lightTokens.has(token));
    expect(orphans).toEqual([]);
  });

  it("凡是颜色令牌，深色里必须逐个覆盖", () => {
    // 只管 `--color-*`：间距、圆角、字号与标题栏几何量与配色方案无关，
    // 强制它们成对反而会逼出一堆重复声明。
    const missing = [...lightTokens]
      .filter((token) => token.startsWith("--color-"))
      .filter((token) => !darkTokens.has(token));
    expect(
      missing,
      `这些颜色令牌只在浅色里声明，深色下会沿用浅色值：${missing.join("、")}。` +
        "「深色底 + 浅色卡片 + 浅色文字」正是这么来的。",
    ).toEqual([]);
  });

  it("两个语域的颜色令牌在深色下也各自成组", () => {
    for (const token of [
      "--color-sourced-surface",
      "--color-sourced-border",
      "--color-ai-surface",
      "--color-ai-border",
      "--color-ai-badge-bg",
      "--color-error-surface",
      "--color-highlight-bg",
    ]) {
      expect(darkTokens.has(token), `深色缺 ${token}`).toBe(true);
    }
  });

  it("考据与 AI 的底色在两套配色下都不相同", () => {
    // 相同就意味着视觉区分只剩边框，而边框是这套区分里最弱的一项。
    function valueOf(source: string, token: string): string {
      const match = new RegExp(`${token}\\s*:\\s*([^;]+);`).exec(source);
      return (match?.[1] ?? "").trim();
    }
    for (const [name, source] of [
      ["浅色", light],
      ["深色", dark],
    ] as const) {
      const sourced = valueOf(source, "--color-sourced-surface");
      const ai = valueOf(source, "--color-ai-surface");
      expect(sourced).not.toBe("");
      expect(ai).not.toBe("");
      expect(sourced, `${name}下两个语域底色相同`).not.toBe(ai);
    }
  });
});
