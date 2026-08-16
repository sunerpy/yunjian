/**
 * 交给 Tauri 的事件通道必须是**已订阅**的。
 *
 * # 这条守卫拦的是一个已经发生过的缺陷
 *
 * `data/__tests__/tauriChannelContract.test.ts` 从行为侧问「事件有没有落到调用方」，
 * 但那只能覆盖它逐个点到的命令。本文件从结构侧封住整类写法：
 * **`new Channel` 只允许出现在 `data/progressChannel.ts` 里**，
 * 而那个函数的函数体一定会给 `onmessage` 赋值。于是「建了通道却不读」在调用点写不出来。
 *
 * 时间线（`.omo/notepads/yunjian/issues.md`）：
 *
 * 1. PR #104 的真机验收发现 `fetch_corpus` 与 `appreciate_poem` 压根不建 Channel，
 *    两个功能在真机上被 Tauri 直接拒掉。
 * 2. PR #105 补齐了 Channel，但 `fetch_corpus` 那个**建了不订阅**，
 *    首启物化 474,043 首诗全程无反馈。当时的行为断言只问「Channel 在不在实参里」，照绿。
 *
 * 两次都是同一件事的两半：一半是没建，一半是没读。所以守卫盯的是**构造点**——
 * 只要构造只能发生在那一个已经订阅了的函数里，两半就都关上了。
 *
 * # 为什么要剔掉注释再判
 *
 * 本项目已经**五次**踩到「解释一条规则的文字命中这条规则」：PR #107 的注入验证里把校验
 * 代码删掉、注释留着，断言照绿（`crates/yunjian-app/tests/bundle_targets.rs` 记过一次，
 * 第二版还是踩了）。本文件自己就要在注释里写出 `new Channel` 这几个字，
 * 因此扫描前先把注释与字符串字面量剔掉，否则它会命中自己的说明文字。
 */

import { readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const SOURCE_ROOT = resolve(process.cwd(), "src");

/** 唯一允许构造 Channel 的文件，相对 `src/`。 */
const CHANNEL_FACTORY = "data/progressChannel.ts";

function sourceFiles(): string[] {
  const found: string[] = [];
  const pending = [SOURCE_ROOT];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) {
        pending.push(path);
      } else if (/\.(ts|tsx)$/.test(entry.name)) {
        found.push(path);
      }
    }
  }
  return found.sort();
}

/**
 * 去掉注释与字符串字面量之后的代码。
 *
 * 逐字符扫而不是拿正则一把替：正则分不清 `"// 不是注释"` 与真注释，
 * 而两种都会出现在本仓库里。行号靠把每个被剔掉的字符换成同宽空白来保住。
 */
function codeOnly(source: string): string {
  const out: string[] = [];
  let index = 0;
  const blank = (character: string) => (character === "\n" ? "\n" : " ");
  while (index < source.length) {
    const two = source.slice(index, index + 2);
    if (two === "//") {
      while (index < source.length && source[index] !== "\n") {
        out.push(" ");
        index += 1;
      }
      continue;
    }
    if (two === "/*") {
      const end = source.indexOf("*/", index + 2);
      const stop = end === -1 ? source.length : end + 2;
      for (; index < stop; index += 1) {
        out.push(blank(source[index] as string));
      }
      continue;
    }
    const quote = source[index] as string;
    if (quote === '"' || quote === "'" || quote === "`") {
      out.push(quote);
      index += 1;
      while (index < source.length) {
        const character = source[index] as string;
        if (character === "\\") {
          out.push(" ", " ");
          index += 2;
          continue;
        }
        if (character === quote) {
          out.push(quote);
          index += 1;
          break;
        }
        out.push(blank(character));
        index += 1;
      }
      continue;
    }
    out.push(quote);
    index += 1;
  }
  return out.join("");
}

interface Hit {
  file: string;
  line: number;
}

function constructionSites(): Hit[] {
  const hits: Hit[] = [];
  for (const path of sourceFiles()) {
    const lines = codeOnly(readFileSync(path, "utf8")).split("\n");
    lines.forEach((text, offset) => {
      if (/\bnew\s+Channel\b/.test(text)) {
        hits.push({ file: relative(SOURCE_ROOT, path), line: offset + 1 });
      }
    });
  }
  return hits;
}

describe("事件通道的构造点", () => {
  it("`new Channel` 只出现在 `data/progressChannel.ts` 里", () => {
    const stray = constructionSites().filter((hit) => hit.file !== CHANNEL_FACTORY);
    expect(
      stray,
      `这些地方自己建了事件通道：${stray.map((hit) => `${hit.file}:${hit.line}`).join("、")}。` +
        `自己建就要自己记得订阅 \`onmessage\`，而漏掉订阅没有任何报错——` +
        `事件被丢在地上，界面只表现为「过程中什么都不说」。改成调用 \`progressChannel\`。`,
    ).toEqual([]);
  });

  it("那个唯一的构造点确实给 `onmessage` 赋了值", () => {
    // 上一条只保证「构造集中在一处」。如果那一处自己也不订阅，集中就毫无意义——
    // 缺陷会从「散在四个调用点」变成「藏在一个函数里」，而且更难发现。
    const factory = codeOnly(readFileSync(resolve(SOURCE_ROOT, CHANNEL_FACTORY), "utf8"));
    expect(factory).toMatch(/\bnew\s+Channel\b/);
    expect(factory, `${CHANNEL_FACTORY} 建了通道却没有订阅 onmessage`).toMatch(/\.onmessage\s*=/);
  });

  it("剔注释这一步真的在起作用", () => {
    // 这条断言的存在理由：本文件与 `data/progressChannel.ts` 都在注释里写了
    // `new Channel` 这几个字。如果 `codeOnly` 哪天退化成恒等函数，
    // 上面第一条会因为命中自己的说明文字而变红——那是一次误报，但读起来像真报。
    // 所以直接钉住剔除行为本身。
    const sample = [
      "// new Channel<Foo>()",
      'const label = "new Channel";',
      "const real = new Channel();",
    ].join("\n");
    const stripped = codeOnly(sample).split("\n");
    expect(stripped).toHaveLength(3);
    expect(stripped[0]).not.toMatch(/\bnew\s+Channel\b/);
    expect(stripped[1]).not.toMatch(/\bnew\s+Channel\b/);
    expect(stripped[2]).toMatch(/\bnew\s+Channel\b/);
  });
});
