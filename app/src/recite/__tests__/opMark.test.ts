/**
 * 五类差异的标记必须两两可分——记号、文案、类名、样式声明四个层面都要分开。
 *
 * # 这一组为什么不只断言「类名不同」
 *
 * 类名不同是最容易满足也最容易骗过的一条：六个类名各不相同、但 `recite.css` 里
 * 六条规则的内容一模一样，界面上照样一片相同的方框。所以这里同时读 CSS，
 * 断言六条规则的**声明集合两两不同**——这是 `theme.test.ts` 用来抓「深色忘了覆盖」
 * 那个缺陷的同一手段：jsdom 不算样式，所以样式层的断言只能靠读源码。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { ReciteOp } from "../../contracts/recite";
import { RECITE_OP_KINDS } from "../../contracts/recite";
import { OP_MARKS, OP_MARK_KINDS, opCells, opLegend, opMarkOf } from "../opMark";

const css = readFileSync(resolve(process.cwd(), "src/recite/recite.css"), "utf8");

/**
 * 五类差异各一个样例，外加一个相符项。
 *
 * 键是 `OpMarkKind`（六个），值是 `ReciteOp`（五个变体）——两者数目不同，因为
 * 替换按 `near_homophone` 占了两个键。替换那两条另行声明成收窄的类型，
 * 于是翻转布尔的那条断言不必对整个联合做类型断言。
 */
const SUBSTITUTION = {
  kind: "substitution",
  reference_index: 12,
  attempt_index: 12,
  reference: "望",
  attempt: "看",
  near_homophone: false,
} as const satisfies ReciteOp;

const NEAR_HOMOPHONE = {
  kind: "substitution",
  reference_index: 17,
  attempt_index: 17,
  reference: "思",
  attempt: "丝",
  near_homophone: true,
} as const satisfies ReciteOp;

const SAMPLES: Record<string, ReciteOp> = {
  normal: { kind: "normal", reference_index: 0, attempt_index: 0, character: "床" },
  deletion: { kind: "deletion", reference_index: 3, reference: "月" },
  insertion: { kind: "insertion", reference_index: 4, attempt_index: 4, attempt: "啊" },
  re_recitation: {
    kind: "re_recitation",
    reference_start: 2,
    reference_end: 5,
    attempt_start: 5,
    attempt_end: 8,
    text: "明月光",
  },
  substitution: SUBSTITUTION,
  near_homophone_substitution: NEAR_HOMOPHONE,
};

/** 取 `.recite-op--X { … }` 这条规则的声明集合，不含伪元素规则。 */
function declarationsOf(className: string): Set<string> {
  const pattern = new RegExp(`\\.${className}\\s*\\{([^}]*)\\}`);
  const match = pattern.exec(css);
  expect(match, `recite.css 里没有 .${className} 这条规则`).not.toBeNull();
  return new Set(
    (match?.[1] ?? "")
      .split(";")
      .map((declaration) => declaration.trim())
      .filter((declaration) => declaration !== ""),
  );
}

describe("五类差异的标记", () => {
  it("六个类别的键与内核词汇表对得上", () => {
    // 前五个必须与 `ReciteOp` 的 `kind` 逐字相同；第六个是替换按 `near_homophone` 拆出来的。
    expect(OP_MARK_KINDS.slice(0, RECITE_OP_KINDS.length)).toEqual([...RECITE_OP_KINDS]);
    expect(OP_MARK_KINDS.at(-1)).toBe("near_homophone_substitution");
  });

  it("记号字符两两不同，且逐字等于命令行的那六个", () => {
    // 命令行的记号来自 `OpOut::mark()`，图例行是
    // 「✓ 相符 · ✗ 漏读 · ＋ 增读 · ↻ 回读 · ≠ 替换 · ≈ 近音替换」。
    // 两处不一致会让同一条差异在终端与界面里像两回事。
    const marks = OP_MARK_KINDS.map((kind) => OP_MARKS[kind].mark);
    expect(marks).toEqual(["✓", "✗", "＋", "↻", "≠", "≈"]);
    expect(new Set(marks).size).toBe(marks.length);
  });

  it("中文名两两不同，且五类差异的名字正是任务要求的那五个", () => {
    const labels = OP_MARK_KINDS.map((kind) => OP_MARKS[kind].label);
    expect(new Set(labels).size).toBe(labels.length);
    expect(labels.slice(1)).toEqual(["漏读", "增读", "回读", "替换", "近音替换"]);
  });

  it("样式类名两两不同", () => {
    const names = OP_MARK_KINDS.map((kind) => OP_MARKS[kind].className);
    expect(new Set(names).size).toBe(names.length);
  });

  it("**六条 CSS 规则的声明集合两两不同**", () => {
    // 这是本组最有牙的一条：把六条规则改成内容相同（只留颜色差异也不够，
    // 因为颜色令牌名也在声明里）——只要有两条完全一样，这里就红。
    const sets = OP_MARK_KINDS.map((kind) => ({
      kind,
      declarations: declarationsOf(OP_MARKS[kind].className),
    }));
    for (const entry of sets) {
      expect(entry.declarations.size, `${entry.kind} 这条规则是空的`).toBeGreaterThan(0);
    }
    for (let left = 0; left < sets.length; left += 1) {
      for (let right = left + 1; right < sets.length; right += 1) {
        const a = sets[left];
        const b = sets[right];
        expect(a, "样例表出错").toBeDefined();
        expect(b, "样例表出错").toBeDefined();
        const same =
          a !== undefined &&
          b !== undefined &&
          a.declarations.size === b.declarations.size &&
          [...a.declarations].every((declaration) => b.declarations.has(declaration));
        expect(same, `${a?.kind} 与 ${b?.kind} 的样式声明完全相同，界面上分不开`).toBe(false);
      }
    }
  });

  it("每一类同时改变颜色、边框与文字装饰三个维度中的至少两个", () => {
    // 只换颜色是不够的：约 8% 的男性有红绿色觉异常，而最需要分开的漏读与增读
    // 恰好落在红橙两个色相上。
    for (const kind of OP_MARK_KINDS.filter((candidate) => candidate !== "normal")) {
      const declarations = [...declarationsOf(OP_MARKS[kind].className)];
      const dimensions = [
        declarations.some((declaration) => declaration.startsWith("color:")),
        declarations.some((declaration) => declaration.startsWith("border:")),
        declarations.some((declaration) => declaration.startsWith("text-decoration:")),
        declarations.some((declaration) => declaration.startsWith("background:")),
      ].filter(Boolean);
      expect(
        dimensions.length,
        `${kind} 只改了 ${dimensions.length} 个维度`,
      ).toBeGreaterThanOrEqual(3);
    }
  });

  it("替换按 near_homophone 分成两类，且不靠前端自己判近音", () => {
    expect(opMarkOf(SAMPLES.substitution!).kind).toBe("substitution");
    expect(opMarkOf(SAMPLES.near_homophone_substitution!).kind).toBe("near_homophone_substitution");
    // 同一对字，只翻转那个布尔，判定就必须跟着翻转——证明判据来自载荷而不是字形。
    const flipped: ReciteOp = { ...NEAR_HOMOPHONE, near_homophone: false };
    expect(opMarkOf(flipped).kind).toBe("substitution");
  });
});

describe("逐字格子", () => {
  it("每一类显示的字取自内核给的那个字段", () => {
    const cells = opCells([
      SAMPLES.normal!,
      SAMPLES.deletion!,
      SAMPLES.insertion!,
      SAMPLES.re_recitation!,
      SAMPLES.substitution!,
    ]);
    expect(cells.map((cell) => cell.text)).toEqual(["床", "月", "啊", "明月光", "看"]);
    // 替换那一格另带应读字；其余类别没有这一项。
    expect(cells.map((cell) => cell.expected)).toEqual([
      undefined,
      undefined,
      undefined,
      undefined,
      "望",
    ]);
  });

  it("顺序与内核给的顺序一致，不重排也不合并相邻同类", () => {
    // 连续漏三个字与漏掉一个词是两件事，合并会把前者显示成后者。
    const three: ReciteOp[] = [
      { kind: "deletion", reference_index: 1, reference: "前" },
      { kind: "deletion", reference_index: 2, reference: "明" },
      { kind: "deletion", reference_index: 3, reference: "月" },
    ];
    expect(opCells(three)).toHaveLength(3);
  });

  it("说明文案逐字对齐命令行的 explain()，含一基下标", () => {
    const cells = opCells([
      SAMPLES.deletion!,
      SAMPLES.insertion!,
      SAMPLES.re_recitation!,
      SAMPLES.substitution!,
      SAMPLES.near_homophone_substitution!,
    ]);
    expect(cells[0]?.description).toBe("第 4 字 漏读：应读「月」");
    expect(cells[1]?.description).toBe("作答第 5 字 增读：多读了「啊」");
    expect(cells[2]?.description).toBe("第 3–5 字 回读：重读了「明月光」");
    expect(cells[3]?.description).toBe("第 13 字 替换：应读「望」，实读「看」");
    expect(cells[4]?.description).toBe("第 18 字 近音替换：应读「思」，实读「丝」");
  });
});

describe("图例", () => {
  it("**图例胶囊自己取消文字装饰，而不是在标签后代上取消**", () => {
    // 亲手 QA 花了两轮才对：祖先的文字装饰会传播到流内后代，**后代无法取消它**。
    // 在标签上写 `text-decoration: none` 会让计算值变成 `none` 而线照旧画着，
    // 加 `display: inline-block` 也没用（flex item 被块化，仍在传播链上）。
    // 唯一有效的位置是**产生**那条线的元素，即胶囊自己。
    //
    // 断言写在 CSS 层而不是 DOM 层，正是因为 DOM 层查不出来——
    // 这个缺陷的整个教训就是「计算样式说没有装饰，截图上线还在」。
    const rule = /\.recite-legend\s+\.recite-legend__item\s*\{([^}]*)\}/.exec(css);
    expect(rule, "缺少胶囊层的 text-decoration 取消规则").not.toBeNull();
    expect(rule?.[1]).toContain("text-decoration: none");
    // 反向确认：正文里的装饰必须还在，否则「取消装饰」会顺手把区分手段也删掉。
    expect([...declarationsOf(OP_MARKS.deletion.className)]).toContain(
      "text-decoration: line-through",
    );
    expect([...declarationsOf(OP_MARKS.insertion.className)]).toContain(
      "text-decoration: underline wavy",
    );
  });

  it("图例里的回读项不画那对方括号，正文里仍然画", () => {
    // 括号表示「跨多字的区间」，图例里只有两个字的类别名，没有区间可标；
    // 留着它会让这一项比其余五项宽出一截。
    expect(css).toContain(".recite-legend__item.recite-op--rerecitation::before");
    expect(css).toMatch(/\.recite-op--rerecitation::before\s*\{\s*content:\s*"［"/);
  });

  it("由描述子表生成，不写死一句话", () => {
    // 写死会在某天改了某个记号之后与实际渲染不一致，而图例与渲染不一致比没有图例更糟。
    const legend = opLegend();
    for (const kind of OP_MARK_KINDS) {
      expect(legend).toContain(`${OP_MARKS[kind].mark} ${OP_MARKS[kind].label}`);
    }
  });
});
