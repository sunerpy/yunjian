/**
 * 每个可点的东西都必须用光标区分「能操作」与「不能操作」。
 *
 * # 这一组防的是什么，以及为什么现有断言防不到
 *
 * 浏览器给 `<button>` 的默认光标是 `default`（箭头），**不是**手型；而本项目刻意不引入
 * Tailwind preflight（理由见 `tailwind.css` 顶部），所以没有任何一层会替我们补
 * `cursor: pointer`。`tailwind.test.ts` 的「按钮逐个显式写了 cursor-pointer」只扫
 * `shell/` 那两个用 utility 写样式的外壳组件，并在注释里明确把既有组件划给「它们各自的
 * css 管」——而那些 css 里**一条 cursor 都没有**（除了标题栏刻意写的箭头）。
 * 缺口正落在这两条断言的缝里。
 *
 * 实测（真实浏览器 `getComputedStyle`，非 jsdom）：
 *
 * | 元素 | 静息 | 禁用 |
 * | --- | --- | --- |
 * | `.recite-button` / `.settings-button` / `.search-screen__paging button` | `default` | `default` |
 * | `.search-screen__submit` / `.poem-screen__back` / `.result-row__open` | `default` | 不会禁用 |
 * | `.recite-field__control`（输入框） | `text` | `default`（UA 自己改的） |
 * | `.recite-field__control`（滑块） | `default` | `default` |
 * | 外壳按钮（Tailwind `cursor-pointer`） | `pointer` | 不会禁用 |
 *
 * 两个症状各自都足以让用户误判：静息态没有手型 → 「移上去毫无反馈，界面是不是卡住了」；
 * 禁用态与静息态同一个光标 → 「看着能点，点了没反应」。后者是前者的镜像。
 *
 * # 为什么断言从 tsx 出发而不是只读 css
 *
 * 只读 css 的版本有个空洞：把某条规则的选择器改掉（`.recite-button` → `.recite-btn`）
 * 断言照样绿，而组件用的是旧类名，光标当场消失。所以下面每一条都**先从 tsx 里取出组件
 * 真正写的 className**，再回到 css 里找那条类真正生效的规则——这与 `tailwind.test.ts`
 * 里「侧栏令牌名从 Sidebar.tsx 读出来，不写死」是同一条教训：断言若不经过组件真正读的
 * 那条路径，它验的是一份平行事实。
 *
 * # 为什么必须逐个按钮解析作用域，而不是「样式表里出现过 cursor: not-allowed」
 *
 * 后者补对一处就全绿，剩下几处照旧骗人。中间那一版也不行：它在判定一个没有 className 的
 * 按钮（`.search-screen__paging button` 那两个分页钮）时，只要**任何**一条
 * `X button` 规则写了手型就放过全部按钮——同样是补一处就绿。所以下面把每个标签的
 * 作用域（自身 className ∪ 源码里最近的上层 className）算出来，规则必须落在这个作用域上
 * 才算数，并且把全部违规项收集齐再一次性比对空数组。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/** 参与检查的手写样式表。`tailwind.css` 只有指令与 `@theme`，不含规则。 */
const STYLESHEETS = [
  "styles.css",
  "chrome/titlebar.css",
  "search/search.css",
  "poem/poem.css",
  "recite/recite.css",
  "settings/settings.css",
] as const;

/** 含 `<button>` 或可禁用部件的全部组件。 */
const COMPONENTS = [
  "chrome/TitleBar.tsx",
  "poem/PoemDetailScreen.tsx",
  "recite/ReciteScreen.tsx",
  "recite/ModeSelector.tsx",
  "recite/TypingPanel.tsx",
  "recite/ResultView.tsx",
  "recite/VoicePanel.tsx",
  "recite/ReviewQueue.tsx",
  "search/SearchScreen.tsx",
  "search/ResultList.tsx",
  "settings/KeyStoragePanel.tsx",
  "settings/CachePanel.tsx",
  "settings/CorpusPanel.tsx",
  "shell/DictionaryPanel.tsx",
  "shell/Sidebar.tsx",
  "shell/SettingsDialog.tsx",
] as const;

/**
 * 窗口按钮刻意是箭头，不是漏写：它们是标题栏的窗口控件，跟随宿主平台的窗口装饰习惯，
 * `chrome/__tests__/TitleBar.test.tsx` 有一条断言钉住 `.titlebar__button` 的
 * `cursor: default`。列成白名单而不是让规则放过所有找不到的类，是为了让「新增一个真的
 * 漏写的按钮」仍然红；白名单本身另有一条断言要求那个箭头是显式声明的。
 */
const DELIBERATE_DEFAULT_CURSOR = new Set(["titlebar__button"]);

function read(relative: string): string {
  return readFileSync(resolve(process.cwd(), "src", relative), "utf8");
}

/**
 * 去掉块注释之后的样式表全文。
 *
 * 必需的一步：下面按 `;` 切声明，而规则体里的解释性注释会与紧跟它的那条声明黏成同一个
 * 片段、于是那条声明被整条丢掉——`chrome/__tests__/TitleBar.test.tsx` 就是这么漏掉过
 * `cursor: default`（实测）。顺带也让「注释里提到 not-allowed」不被当成声明。
 */
const css = STYLESHEETS.map((relative) => read(relative).replace(/\/\*[\s\S]*?\*\//g, "")).join(
  "\n",
);

type Rule = { selector: string; declarations: Set<string> };

/** 把样式表切成 `{selector, declarations}`；逗号分隔的选择器组拆成多条。 */
function parseRules(source: string): Rule[] {
  const rules: Rule[] = [];
  for (const match of source.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const group = (match[1] ?? "").replace(/\s+/g, " ").trim();
    if (group === "" || group.startsWith("@")) {
      continue;
    }
    const declarations = new Set(
      (match[2] ?? "")
        .split(";")
        .map((declaration) => declaration.replace(/\s+/g, " ").trim())
        .filter((declaration) => declaration !== ""),
    );
    for (const selector of group.split(",").map((one) => one.trim())) {
      if (selector !== "") {
        rules.push({ selector, declarations });
      }
    }
  }
  return rules;
}

const RULES = parseRules(css);

/** 一段选择器片段涉及的类名。 */
function classesOf(fragment: string): string[] {
  return [...fragment.matchAll(/\.([A-Za-z][\w-]*)/g)].map((match) => match[1] ?? "");
}

/** 一个从 tsx 里切出来的元素标签，连带它的作用域。 */
type Tag = {
  file: string;
  raw: string;
  element: string;
  /** 标签自身的 className。 */
  classNames: string[];
  /** 自身 className ∪ 源码里最近的上层 className——用来判定后代选择器落不落在它身上。 */
  scope: string[];
  disabled: boolean;
};

/**
 * 把一份 tsx 里的元素标签切出来。
 *
 * 先把箭头函数的 `=>` 换成 `=~`：不换的话 `onClick={() => {` 里那个 `>` 会被当成标签结束，
 * 写在 `onClick` 之后的 className 与 `disabled` 整个扫不到——`tailwind.test.ts` 里这条
 * 扫描的第一版就是这么漏掉过两个按钮（实测）。
 */
function tagsOf(file: string): Tag[] {
  const source = read(file).replace(/=>/g, "=~");
  const tags: Tag[] = [];
  for (const match of source.matchAll(/<(button|input|textarea|select)\b[^>]*>/g)) {
    const raw = match[0];
    const own = classNamesIn(raw);
    // 没写 className 的按钮（分页钮）靠父级选择器兜，所以要知道最近的上层 className。
    const before = source.slice(0, match.index);
    const outer = [...before.matchAll(/className=(?:"([^"]*)"|\{`?([^}`]*)`?\})/g)].at(-1);
    const ancestor = `${outer?.[1] ?? ""} ${outer?.[2] ?? ""}`
      .split(/\s+/)
      .filter((name) => name !== "");
    tags.push({
      file,
      raw,
      element: match[1] ?? "",
      classNames: own,
      scope: [...new Set([...own, ...ancestor])],
      disabled: /\sdisabled[=\s/>]/.test(raw),
    });
  }
  return tags;
}

function classNamesIn(raw: string): string[] {
  const match = /className=(?:"([^"]*)"|\{`?([^}`]*)`?\})/.exec(raw);
  return `${match?.[1] ?? ""} ${match?.[2] ?? ""}`.split(/\s+/).filter((name) => name !== "");
}

/**
 * 这条规则是否作用在这个标签上。
 *
 * 两条路径：末尾片段直接命中标签自己的类，或者末尾片段是裸元素名（`button`）而某个上层
 * 片段命中标签作用域里的类。后者是 `.search-screen__paging button` 那两个分页钮唯一的
 * 归属途径——**且它要求上层类真的对得上**，不是「任意一条 `X button` 规则存在即可」。
 */
function matches(rule: Rule, tag: Tag): boolean {
  const parts = rule.selector.split(" ").filter((part) => part !== "");
  const last = parts.at(-1) ?? "";
  if (classesOf(last).some((name) => tag.classNames.includes(name))) {
    return true;
  }
  const bare = new RegExp(`^${tag.element}(:disabled)?$`).test(last);
  return (
    bare && parts.slice(0, -1).some((part) => classesOf(part).some((n) => tag.scope.includes(n)))
  );
}

/**
 * 这个标签在指定状态下拿到了指定光标吗。
 *
 * `wantDisabled` 为真时只看 `:disabled` 那一支；为假时只看不带 `:disabled` 的静息规则
 * ——否则「只给禁用态写了光标」会让静息态那条断言误绿。`:hover` / `:active` /
 * `:focus` 与伪元素是瞬时态，不算静息态的光标来源。
 */
function hasCursor(tag: Tag, value: string, wantDisabled: boolean): boolean {
  return RULES.some((rule) => {
    if (rule.selector.includes(":disabled") !== wantDisabled) {
      return false;
    }
    if (/::|:hover|:active|:focus/.test(rule.selector)) {
      return false;
    }
    return matches(rule, tag) && rule.declarations.has(`cursor: ${value}`);
  });
}

function label(tag: Tag): string {
  const id = /data-testid=(?:"([^"]*)"|\{`([^`]*)`\})/.exec(tag.raw);
  const name = id?.[1] ?? id?.[2] ?? tag.classNames.join(".");
  return `${tag.file}: <${tag.element}> ${name === "" ? tag.raw.slice(0, 48) : name}`;
}

const TAGS = COMPONENTS.flatMap((file) => tagsOf(file));
const BUTTONS = TAGS.filter((tag) => tag.element === "button");

describe("光标必须区分「能点」与「不能点」", () => {
  it("扫描前提成立：每个组件都切出了按钮，且找到了带 disabled 的部件", () => {
    // 这一条先立住扫描本身。正则失配时下面几条会因为「一个违规都没找到」而全绿，
    // 那种绿是最危险的——它长得跟修好了一模一样。
    const empty = COMPONENTS.filter((file) =>
      tagsOf(file).every((tag) => tag.element !== "button"),
    );
    expect(empty, `这些组件里切不出 <button>，扫描正则失配：${empty.join("、")}`).toEqual([]);
    expect(BUTTONS.length, "切出的按钮太少，扫描正则可能失配").toBeGreaterThanOrEqual(25);
    expect(
      TAGS.filter((tag) => tag.disabled).length,
      "一个带 disabled 的部件都没切出来，扫描前提不成立",
    ).toBeGreaterThanOrEqual(15);
  });

  it("**每个静息按钮都有手型光标**", () => {
    const offenders = BUTTONS.filter(
      (tag) => !tag.classNames.some((name) => DELIBERATE_DEFAULT_CURSOR.has(name)),
    )
      .filter(
        (tag) =>
          // 外壳组件把样式写成 utility；Sidebar 的导航项还把它提到了 `itemClass` 常量里。
          !/cursor-pointer/.test(tag.raw) &&
          !(/className=\{itemClass\(/.test(tag.raw) && /cursor-pointer/.test(read(tag.file))) &&
          !hasCursor(tag, "pointer", false),
      )
      .map(label);
    expect(
      offenders,
      `这些按钮静息态是箭头，「移上去毫无反馈」会被当成界面卡死：\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("**每个会被禁用的部件都有禁止光标**", () => {
    // 收集全部违规项再比空数组，正是为了不给「补一处就绿」留口子。
    const offenders = TAGS.filter((tag) => tag.disabled)
      .filter((tag) => !/cursor-not-allowed/.test(tag.raw) && !hasCursor(tag, "not-allowed", true))
      .map(label);
    expect(offenders, `这些部件禁用后光标不变，用户会以为能点：\n${offenders.join("\n")}`).toEqual(
      [],
    );
  });

  it("**每条 `:disabled` 规则都写了禁止光标**", () => {
    // 上一条从组件出发，这一条从样式表出发，两个方向都要闭合：新写一条 `:disabled`
    // 规则却只改颜色时，若组件那侧恰好被别的规则兜住，只有这一条会红。
    const rules = RULES.filter((rule) => rule.selector.includes(":disabled"));
    expect(rules.length, "样式表里一条 :disabled 规则都没有，扫描前提不成立").toBeGreaterThan(2);
    const offenders = rules
      .filter((rule) => !rule.declarations.has("cursor: not-allowed"))
      .map((rule) => rule.selector);
    expect(offenders, `这些 :disabled 规则只改了颜色没改光标：${offenders.join("、")}`).toEqual([]);
  });

  it("窗口按钮的箭头光标是显式声明的，不是漏写", () => {
    // 白名单必须由一条真实存在的声明背书，否则它就成了「以后漏写也能混过去」的口子。
    const rule = RULES.find((entry) => entry.selector === ".titlebar__button");
    expect(rule, "titlebar.css 里没有 .titlebar__button 这条规则").not.toBeUndefined();
    expect(rule?.declarations.has("cursor: default"), "白名单里的类并没有显式声明箭头").toBe(true);
  });
});
