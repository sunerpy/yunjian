/**
 * 评分权威只在内核：前端不得对分数字段做算术。
 *
 * # 任务书给的判据与它的两个缺口
 *
 * 任务规定的门禁是一条 grep：
 *
 * ```
 * ! grep -rnE '(completeness|accuracy)\s*[*+\/-]' app/src/
 * ```
 *
 * 本文件在 TypeScript 里把它跑起来（与 `theme.test.ts` 读 `styles.css` 同一种做法：
 * 样式与源码层面的约束只能靠读源码来验），并**补上它的两个缺口**：
 *
 * 1. **只写禁令的话，把分数整段删掉的实现也能通过。** 所以另有一组正向断言，
 *    确认那四个字段真的被读了、真的渲染进 DOM 了。这条补法照抄命令行侧
 *    `the_cli_carries_no_scoring_logic_of_its_own` 的思路（`learnings.md` 的 todo 58）。
 * 2. **grep 只拦住写成算术表达式的那一种形态。** 「先取出来存进局部变量再算」、
 *    「用 `Math` 里的函数算」都绕得过去。所以再补两条：禁止百分号换算的常见写法，
 *    以及 `ResultView.test.tsx` 那份**分数与操作列表刻意矛盾**的载荷——
 *    后者拦的是「界面显示的数与内核给的数不同」这件事本身，与写法无关。
 */

import { readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const SOURCE_ROOT = resolve(process.cwd(), "src");

/** 本文件自己要谈论那条正则，因此排除自己，否则会自我命中。 */
const SELF = resolve(SOURCE_ROOT, "recite/__tests__/noScoreArithmetic.test.ts");

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
      } else if (/\.(ts|tsx)$/.test(entry.name) && path !== SELF) {
        found.push(path);
      }
    }
  }
  return found.sort();
}

/** 命中项：文件、行号、行内容。与 `grep -rn` 的输出形状一致，便于照抄修。 */
interface Hit {
  file: string;
  line: number;
  text: string;
}

function scan(pattern: RegExp): Hit[] {
  const hits: Hit[] = [];
  for (const file of sourceFiles()) {
    const lines = readFileSync(file, "utf8").split("\n");
    lines.forEach((text, index) => {
      // 逐行匹配，与 grep 的行为一致：`\s*` 不跨行。
      if (new RegExp(pattern.source, pattern.flags.replace("g", "")).test(text)) {
        hits.push({ file: relative(SOURCE_ROOT, file), line: index + 1, text: text.trim() });
      }
    });
  }
  return hits;
}

function report(hits: Hit[]): string {
  return hits.map((hit) => `${hit.file}:${hit.line}: ${hit.text}`).join("\n");
}

describe("禁止前端算分", () => {
  it("**任务书那条 grep 在整个 `app/src/` 上没有命中**", () => {
    // 与 `! grep -rnE '(completeness|accuracy)\s*[*+/-]' app/src/` 逐字等价。
    const hits = scan(/(completeness|accuracy)\s*[*+/-]/);
    expect(hits.length, `在分数字段上发现算术：\n${report(hits)}`).toBe(0);
  });

  it("节奏连贯度与拒绝标志同样不参与算术", () => {
    // 任务书那条只点了两个字段名，但「合成一个总分」最可能拿来乘权重的正是 fluency。
    const hits = scan(/(fluency|is_rejected)\s*[*+/-]/);
    expect(hits.length, `在分数字段上发现算术：\n${report(hits)}`).toBe(0);
  });

  it("不出现把比例换算成百分比的常见写法", () => {
    // grep 那条拦不住 `Math.round(x * 100)` 里 x 是局部变量的形态，这条从另一头拦。
    const hits = scan(/\*\s*100|100\s*\*|toFixed\(\s*[01]\s*\)\s*\+\s*["'`]%/);
    expect(hits.length, `疑似百分比换算：\n${report(hits)}`).toBe(0);
  });

  it("前端不重复内核的评级阈值字面量", () => {
    // `grade_typed` 的四个阈值只应作为**载荷字段**出现（`stats.grading.*`），
    // 写死一个 0.85 就意味着前端有了第二套评级规则。
    //
    // 两类文件豁免，理由相同：它们在**扮演后端**，返回的是
    // `GradingConfig::default()` 的镜像值，而不是在判等级。
    // 一个是 `__tests__/` 下的替身载荷，一个是 `data/sample*Ports.ts`
    // 这些非 Tauri 宿主下的样例端口。
    const offenders = sourceFiles()
      .filter((file) => !file.includes("__tests__") && !/[/\\]sample\w*Ports\.ts$/.test(file))
      .flatMap((file) => {
        const lines = readFileSync(file, "utf8").split("\n");
        return lines
          .map((text, index) => ({ file: relative(SOURCE_ROOT, file), line: index + 1, text }))
          .filter(
            (entry) =>
              /(?:^|[^.\w])0\.85(?![\d])|(?:^|[^.\w])0\.97(?![\d])/.test(entry.text) &&
              !entry.text.trimStart().startsWith("*") &&
              !entry.text.trimStart().startsWith("//"),
          )
          .map((entry) => ({ ...entry, text: entry.text.trim() }));
      });
    expect(offenders.length, `疑似复制了内核阈值：\n${report(offenders)}`).toBe(0);
  });

  it("前端没有自带的对齐实现", () => {
    // 对齐是内核的 Wagner-Fischer 加回读检测；前端出现编辑距离的迹象就说明有第二份实现。
    const hits = scan(/levenshtein|editDistance|wagnerFischer|function\s+align\b/i);
    expect(hits.length, `疑似前端自带对齐：\n${report(hits)}`).toBe(0);
  });
});

describe("正向确认：分数确实被读了", () => {
  const resultView = readFileSync(resolve(SOURCE_ROOT, "recite/ResultView.tsx"), "utf8");

  it("结果视图逐个读取内核给的四个分数字段", () => {
    // 只有禁令的话，把这四行删掉的实现同样能通过上面全部断言。
    for (const field of ["completeness", "accuracy_strict", "accuracy_lenient", "fluency"]) {
      expect(resultView, `结果视图没有读 score.${field}`).toContain(`attempt.score.${field}`);
    }
  });

  it("结果视图渲染的是内核给的等级建议，而不是自己判出来的", () => {
    expect(resultView).toContain("attempt.suggested_grade");
  });

  it("对齐操作列表来自载荷，且交给逐字面板渲染", () => {
    expect(resultView).toContain("attempt.ops");
    expect(resultView).toContain("<OpFeedback ops={attempt.ops} />");
  });

  it("发音边界注记是常量而不是就地写的字面量", () => {
    // 写成字面量就会在某天被「顺手改一下措辞」抹掉；常量在 `contracts/recite.ts` 里，
    // 且有一条断言盯着它的内容。
    expect(resultView).toContain("NO_PRONUNCIATION_NOTE");
    const contracts = readFileSync(resolve(SOURCE_ROOT, "contracts/recite.ts"), "utf8");
    expect(contracts).toContain("不评估发音标准度");
  });
});
