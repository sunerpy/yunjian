/**
 * Tailwind 的刻度与 `styles.css` 的令牌必须逐个相等。
 *
 * # 这条断言防的是什么
 *
 * 引入 Tailwind 以后，同一个视觉决定有了两条表达路径：`p-3` 与 `padding: var(--space-3)`、
 * `text-xl` 与 `font-size: var(--text-xl)`。两条路径背后是**两份取值**——一份在
 * `node_modules/tailwindcss/theme.css`，一份在 `src/styles.css`。
 *
 * 只要它们不等，界面上就会出现两种间距刻度、两种字号阶梯，而**没有任何现有测试会红**：
 * jsdom 不算样式，产物构建也不会因为两个数不同而失败。这与 `theme.test.ts` 抓的
 * 「深色忘了覆盖令牌」是同一类缺陷——只在截图上看得见。
 *
 * 所以这里把两份取值直接摆在一起比对。它同时是 **Tailwind 升级改了默认刻度时唯一会响的
 * 警报**：`--text-xl` 从 1.25rem 变成别的数，这条会红，而不是等到某天有人截图时才发现。
 *
 * # 为什么读 node_modules 而不是读构建产物
 *
 * 产物里的 utility 是按用量摇树出来的：没人写过 `text-2xl` 时产物里就没有它，
 * 于是「刻度不一致」会因为暂时没被用到而蒙混过关，等到第一次有人用才暴露。
 * 默认刻度文件里则是全的。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const tokens = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");
const entry = readFileSync(resolve(process.cwd(), "src/tailwind.css"), "utf8");
const defaults = readFileSync(resolve(process.cwd(), "node_modules/tailwindcss/theme.css"), "utf8");
/** `tailwind.css` 去掉块注释后的正文。禁令类断言必须扫这一份，见「preflight」那一条。 */
const directives = entry.replace(/\/\*[\s\S]*?\*\//g, "");

/** 取一份 CSS 文本里某个自定义属性的值；取不到返回 `null`。 */
function declared(source: string, token: string): string | null {
  const match = new RegExp(`${token}\\s*:\\s*([^;]+);`).exec(source);
  return match === null ? null : (match[1] ?? "").replace(/\s+/g, " ").trim();
}

/**
 * Tailwind 实际生效的刻度值：默认值，被 `tailwind.css` 的 `@theme` 覆盖时取覆盖值。
 *
 * `@theme` 块从 `entry` 里整段切出来再查，避免把注释里提到的数字当成声明。
 */
function effective(token: string): string | null {
  const block = /@theme\s*\{([\s\S]*?)\n\}/.exec(entry);
  expect(block, "tailwind.css 里找不到 @theme 块").not.toBeNull();
  return declared(block?.[1] ?? "", token) ?? declared(defaults, token);
}

describe("引入的是哪几层", () => {
  it("theme 与 utilities 两层都在", () => {
    expect(entry).toContain('@import "tailwindcss/theme.css" layer(theme)');
    expect(entry).toContain('@import "tailwindcss/utilities.css" layer(utilities)');
  });

  it("**preflight 刻意没有引入**", () => {
    // 加回来的症状是四条标题规则的字重塌成常规、三十余个 `<p>` 与四处 `<dd>` 失去
    // UA 默认外边距，而 313 条前端测试一条都不会红（jsdom 不算样式）。
    // 完整清点与理由见 `tailwind.css` 顶部「preflight 刻意不引入」。
    //
    // 扫**去掉注释之后**的正文：`tailwind.css` 顶部那段解释里必然出现「preflight」这个词，
    // 直接扫全文会让禁令的解释把禁令本身判成违规。这个坑本仓库已经踩到第三次——
    // todo 59 在 Tauri 官方日志插件那条禁令上，`chrome/__tests__/contracts.test.ts`
    // 在 Electron 窗口拖动区属性那条禁令上，以及本条：这段注释的初稿写出了那个属性的
    // 词根，于是被那条既有禁令当场判红。**解释一条禁令的文字不能命中该禁令。**
    expect(directives).not.toContain("preflight");
    // 顺带确认没有用整包导入绕回来：`@import "tailwindcss"` 会把 preflight 一起带进来。
    expect(directives).not.toMatch(/@import\s+["']tailwindcss["']/);
  });

  it("**外壳部件的 UA 默认值复位在 `@layer base` 里，且真的进了产物**", () => {
    // 目视 QA 实测：没有 preflight 时新按钮拿到 UA 的 `buttonface` 灰底、`2px outset`
    // 四边立体边框与 Arial 13.3px。这条复位就是为了填那个坑。
    //
    // 层级是关键：放进 `base` 才会输给 `utilities`（否则 `font: inherit` 会盖掉
    // `text-sm`）也输给既有无层手写 CSS（否则那 1700 行会被这条改动波及）。
    // 写成无层或放进 `utilities` 都是缺陷，所以这里同时断言层名。
    const base = /@layer base\s*\{([\s\S]*?)\n\}/.exec(entry);
    expect(base, "tailwind.css 里没有 @layer base 块").not.toBeNull();
    const body = base?.[1] ?? "";
    expect(body).toContain("[data-shell-chrome] button");
    for (const declaration of [
      "background-color: transparent",
      "border: 0 solid",
      "font: inherit",
      // UA 的 `padding: 1px 6px` 左右不对称，漏掉它会让没写全 padding 的按钮偏出中心 3px。
      "padding: 0",
    ]) {
      expect(body, `复位里少了 ${declaration}`).toContain(declaration);
    }

    // 两个宿主元素都要带上那个属性，否则复位作用不到。
    for (const relative of ["shell/Sidebar.tsx", "shell/SettingsDialog.tsx"]) {
      const source = readFileSync(resolve(process.cwd(), "src", relative), "utf8");
      expect(source, `${relative} 上没有 data-shell-chrome`).toContain("data-shell-chrome");
    }
  });

  it("按钮逐个显式写了 cursor-pointer", () => {
    // 没有 preflight，也就没人替我们补手型光标；而浏览器对 `<button>` 的默认光标是
    // `default`。少写一处的症状是「鼠标移到按钮上毫无反馈」，用户会以为界面卡住了。
    // 这条只扫用 utility 写样式的外壳组件——既有组件的光标归它们各自的 css 管。
    const files = ["shell/DictionaryPanel.tsx", "shell/Sidebar.tsx", "shell/SettingsDialog.tsx"];
    const offenders: string[] = [];
    for (const relative of files) {
      const source = readFileSync(resolve(process.cwd(), "src", relative), "utf8");
      // 先把箭头函数的 `=>` 换掉再切标签。不换的话 `onClick={() => {` 里那个 `>`
      // 会被当成标签结束，于是写在 `onClick` 之后的 className 整个扫不到——
      // 这条扫描第一版就是这么漏掉两个按钮的（实测）。
      const scannable = source.replace(/=>/g, "=~");
      const buttons = scannable.match(/<button\b[^>]*>/g) ?? [];
      expect(buttons.length, `${relative} 里一个 <button> 都没有，扫描前提不成立`).toBeGreaterThan(
        0,
      );
      // Sidebar 的导航项把 className 提到了 `ITEM_BASE` 常量里，所以先看标签自身，
      // 再退一步看整份源码里那个常量有没有写。
      for (const tag of buttons) {
        const inlineHasCursor = /cursor-(pointer|not-allowed)/.test(tag);
        const viaConstant = /className=\{itemClass\(/.test(tag) && /cursor-pointer/.test(source);
        if (!inlineHasCursor && !viaConstant) {
          offenders.push(`${relative}: ${tag.slice(0, 60)}…`);
        }
      }
    }
    expect(offenders, `这些按钮没有手型光标：\n${offenders.join("\n")}`).toEqual([]);
  });
});

describe("间距刻度", () => {
  /**
   * Tailwind v4 不逐档声明间距，它只有一个基数 `--spacing`，`p-3` 展开成
   * `calc(var(--spacing) * 3)`。所以对齐条件是 `--space-N === N × --spacing`。
   */
  it("`--space-N` 恰好等于 N 倍的 Tailwind 间距基数", () => {
    const base = declared(defaults, "--spacing");
    expect(base, "Tailwind 默认刻度里没有 --spacing").not.toBeNull();
    const step = Number.parseFloat(base ?? "");
    expect(step).toBeGreaterThan(0);
    expect(base?.endsWith("rem"), "间距基数不再是 rem，下面的换算失效").toBe(true);

    const declaredSpaces = [...tokens.matchAll(/--space-(\d+)\s*:\s*([^;]+);/g)];
    expect(declaredSpaces.length, "styles.css 里一个 --space-* 都没有").toBeGreaterThan(0);

    const mismatched = declaredSpaces
      .map(([, index, value]) => ({
        token: `--space-${index}`,
        ours: Number.parseFloat(value ?? ""),
        tailwind: Number.parseFloat(index ?? "") * step,
      }))
      .filter(({ ours, tailwind }) => Math.abs(ours - tailwind) > 1e-9);

    expect(
      mismatched,
      `这些间距令牌与 Tailwind 的 p-N / gap-N 不是同一个数：` +
        mismatched.map((m) => `${m.token}=${m.ours}rem 对 ${m.tailwind}rem`).join("、") +
        "。同屏出现两套间距刻度只有截图看得出来。",
    ).toEqual([]);
  });
});

describe("字号与圆角刻度", () => {
  it.each(["--text-xs", "--text-sm", "--text-base", "--text-lg", "--text-xl", "--text-2xl"])(
    "%s 两处取值相同",
    (token) => {
      expect(declared(tokens, token), `styles.css 缺 ${token}`).not.toBeNull();
      expect(effective(token), `Tailwind 侧取不到 ${token}`).toBe(declared(tokens, token));
    },
  );

  it.each(["--radius-sm", "--radius-md", "--radius-lg"])("%s 两处取值相同", (token) => {
    expect(declared(tokens, token), `styles.css 缺 ${token}`).not.toBeNull();
    expect(effective(token), `Tailwind 侧取不到 ${token}`).toBe(declared(tokens, token));
  });

  it.each(["--font-serif", "--font-sans", "--font-mono"])("%s 两处字体族逐字相同", (token) => {
    // 字体族不等的症状最隐蔽：`font-serif` 与 `var(--font-serif)` 回落到不同的
    // 平台字体，于是同一屏上出现两种中文衬线，而两处「都是衬线」所以不易察觉。
    expect(declared(tokens, token), `styles.css 缺 ${token}`).not.toBeNull();
    expect(effective(token), `Tailwind 侧取不到 ${token}`).toBe(declared(tokens, token));
  });
});

describe("侧栏选中态的底色在两套配色下都必须看得出来", () => {
  /**
   * 与 `theme.test.ts` 的「考据与 AI 的底色在两套配色下都不相同」同一手段，防的是同一类
   * 缺陷：一对颜色在某一套配色下恰好几乎相同，于是**只在那一套配色下**看不见。
   * jsdom 不算样式，所以只能在令牌层比对取值。
   *
   * 来历是一次真实的目视发现：侧栏 `--color-surface` + 选中项 `--color-surface-raised`
   * 在浅色下是 #ffffff 与 #fffefb（每通道差 4），选中态的底色信号等于不存在。
   * 深色下这一对差 7，勉强能看——而深色是本机默认配色，所以缺陷藏在浅色里。
   *
   * **两个令牌名从 `Sidebar.tsx` 里读出来，不写死。** 写死的话把组件改成用另一对令牌
   * 这条断言照样绿——第一版就是这样，注入「把侧栏改回 --color-surface」后 20 条全过
   * （实测）。断言必须盯着组件真正用的那一对。
   */
  const sidebar = readFileSync(resolve(process.cwd(), "src/shell/Sidebar.tsx"), "utf8");
  const light = tokens.slice(0, tokens.indexOf("@media (prefers-color-scheme: dark)"));
  const dark = tokens.slice(tokens.indexOf("@media (prefers-color-scheme: dark)"));

  /** 从一段 className 里取 `bg-[var(--x)]` 的令牌名。 */
  function backgroundToken(source: string, label: string): string {
    const match = /bg-\[var\((--color-[a-z-]+)\)\]/.exec(source);
    expect(match, `${label} 里找不到 bg-[var(--color-*)]`).not.toBeNull();
    return match?.[1] ?? "";
  }

  /** `<nav>` 那一段 className：容器的底色。 */
  const navToken = backgroundToken(
    /className=\{`flex shrink-0[\s\S]*?`\}/.exec(sidebar)?.[0] ?? "",
    "侧栏容器的 className",
  );
  /** `itemClass` 的选中分支：选中项的底色。 */
  const activeToken = backgroundToken(
    /return active\s*\?([\s\S]*?):/.exec(sidebar)?.[1] ?? "",
    "itemClass 的选中分支",
  );

  it("两个底色用的是两个不同的令牌", () => {
    expect(navToken).not.toBe("");
    expect(activeToken).not.toBe("");
    expect(activeToken, "侧栏与选中项用了同一个令牌，选中态没有底色差异").not.toBe(navToken);
  });

  /** `#rrggbb` -> 三个通道。令牌层全是六位十六进制，非此格式直接报错而不是静默取 0。 */
  function channels(value: string): [number, number, number] {
    const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value.trim());
    expect(match, `取值 ${value} 不是六位十六进制，这条比对的前提不成立`).not.toBeNull();
    return [
      Number.parseInt(match?.[1] ?? "0", 16),
      Number.parseInt(match?.[2] ?? "0", 16),
      Number.parseInt(match?.[3] ?? "0", 16),
    ];
  }

  it.each([
    ["浅色", light],
    ["深色", dark],
  ])("%s：侧栏底色与选中项底色至少差 6/通道", (_name, source) => {
    const navValue = declared(source, navToken);
    const activeValue = declared(source, activeToken);
    expect(navValue, `这套配色里没有 ${navToken}`).not.toBeNull();
    expect(activeValue, `这套配色里没有 ${activeToken}`).not.toBeNull();
    const a = channels(navValue ?? "");
    const b = channels(activeValue ?? "");
    const gap = Math.max(...a.map((value, index) => Math.abs(value - (b[index] ?? 0))));
    // 6 是下限而不是目标：低于它的那一版（Δ4）在浅色下肉眼分不出来。
    expect(
      gap,
      `${navToken} 与 ${activeToken} 只差 ${gap}/通道，` +
        "选中态的底色信号看不见（只剩 2px 标尺与字重）",
    ).toBeGreaterThanOrEqual(6);
  });
});

describe("字典三层来源保持不同视觉语域", () => {
  const dictionary = readFileSync(resolve(process.cwd(), "src/shell/DictionaryPanel.tsx"), "utf8");

  function layer(name: string): string {
    const match = new RegExp(`data-source-layer="${name}"[\\s\\S]*?className="([^"]+)"`).exec(
      dictionary,
    );
    expect(match, `找不到 ${name} 来源层`).not.toBeNull();
    return match?.[1] ?? "";
  }

  it("韵书、字书与 AI 三层的边框、底色和字体声明两两不同", () => {
    const declarations = [layer("rhyme"), layer("public-lexicon"), layer("ai")];
    expect(new Set(declarations).size).toBe(3);
    expect(declarations[0]).toContain("border-solid");
    expect(declarations[0]).toContain("font-serif");
    expect(declarations[1]).toContain("border-dotted");
    expect(declarations[1]).toContain("bg-[var(--color-surface-raised)]");
    expect(declarations[2]).toContain("border-dashed");
    expect(declarations[2]).toContain("bg-[var(--color-ai-surface)]");
    expect(declarations[2]).toContain("font-sans");
  });
});

describe("颜色一律走令牌层", () => {
  /**
   * Tailwind 自带的 22 组调色板。用它们等于绕开 `styles.css` 的语义令牌，
   * 后果是深色配色对不上（`theme.test.ts` 那条断言管不到 utility），
   * 且考据 / AI 两个语域的颜色分组失去意义。
   */
  const PALETTES = [
    "slate",
    "gray",
    "zinc",
    "neutral",
    "stone",
    "red",
    "orange",
    "amber",
    "yellow",
    "lime",
    "green",
    "emerald",
    "teal",
    "cyan",
    "sky",
    "blue",
    "indigo",
    "violet",
    "purple",
    "fuchsia",
    "pink",
    "rose",
  ];

  it("源码里没有 `bg-slate-800` 这类调色板 utility", () => {
    const utilities = ["bg", "text", "border", "ring", "outline", "fill", "stroke", "from", "to"];
    // 只扫 className 字面量所在的 tsx，css 里写不出 utility。
    const files = [
      "App.tsx",
      "shell/DictionaryPanel.tsx",
      "shell/Sidebar.tsx",
      "shell/SettingsDialog.tsx",
      "shell/icons.tsx",
    ];
    const pattern = new RegExp(
      `\\b(?:${utilities.join("|")})-(?:${PALETTES.join("|")})-\\d{2,3}\\b`,
    );
    const offenders = files.filter((relative) => {
      const source = readFileSync(resolve(process.cwd(), "src", relative), "utf8");
      return pattern.test(source);
    });
    expect(
      offenders,
      `这些文件用了 Tailwind 自带调色板而不是 --color-* 令牌：${offenders.join("、")}。` +
        "深色配色的成对断言（theme.test.ts）管不到 utility，于是深色下会留下浅色硬编码。",
    ).toEqual([]);
  });

  it("Tailwind 侧刻意没有桥接任何 --color-* 令牌", () => {
    // 桥接会让 `bg-surface` 这种写法出现，而它读起来看不出对应哪个语义令牌。
    // 颜色一律写 `bg-[var(--color-surface)]`，语域信息留在 className 里。
    const block = /@theme\s*\{([\s\S]*?)\n\}/.exec(entry);
    expect(block).not.toBeNull();
    expect(block?.[1]).not.toContain("--color-");
  });
});
