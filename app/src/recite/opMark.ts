/**
 * 五类差异各自的视觉标记。
 *
 * # 为什么标记必须多维而不只是颜色
 *
 * 任务要求「五种 op 各有不同视觉标记」。只换颜色是满足不了的：约 8% 的男性有红绿色觉
 * 异常，而这里最需要区分的恰好是「漏读」（红系）与「增读」（橙系）。所以每一类同时
 * 沿**四个维度**分开，任意单一维度失效时其余三个仍然成立：
 *
 * | 类别 | 记号 | 前景与底色 | 文字装饰 | 边框 | 额外结构 |
 * | --- | --- | --- | --- | --- | --- |
 * | 相符 | `✓` | 中性前景，无底色 | 无 | 无（透明占位） | 无 |
 * | 漏读 | `✗` | 红系 | 删除线 2px | 2px 虚线 | 无 |
 * | 增读 | `＋` | 橙系 | 波浪下划线 2px | 2px 实线 | 无 |
 * | 回读 | `↻` | 蓝系 | 双下划线 1px | 3px 双线 | 前后方括号 |
 * | 替换 | `≠` | 紫系 | 实线下划线 3px | 2px 实线 | 右上角标应读字 |
 * | 近音替换 | `≈` | 青系 | 点线下划线 3px | 2px 点线 | 右上角标应读字 |
 *
 * 「额外结构」一列是 DOM 层的差异而不是样式层的：替换类多一个
 * `.recite-op__expected` 元素，回读类由 `::before` / `::after` 加括号。
 * 因此即使某天有人把两类的边框改成一样，它们在无样式的纯 DOM 上仍然可分。
 *
 * **六个记号字符逐字取自命令行的 `OpOut::mark()`**（`crates/yunjian-cli/src/output.rs:1021-1034`）
 * 与它下面那行图例（`output.rs:1202`）：`✓ 相符 · ✗ 漏读 · ＋ 增读 · ↻ 回读 · ≠ 替换 · ≈ 近音替换`。
 * 两处用同一套记号，于是用户在终端与桌面端看到的是同一种语言，而不是两套私有约定。
 *
 * # 「五种」是哪五种：替换按近音与否分成两种
 *
 * 内核的 `AlignOp` 只有五个变体（`align.rs:8-61`），其中 `Substitution` 带一个
 * `near_homophone` 布尔。任务书列的五种是**漏读/增读/回读/替换/近音替换**，
 * 也就是把替换按那个布尔拆开——与命令行的六个记号一致。因此本模块给出**六个**
 * 描述子：上面五种加「相符」。相符也必须有自己的标记，否则「哪里对了」只能靠
 * 「哪里没标错」反推。
 *
 * # 本模块不做任何判定
 *
 * `near_homophone` 由内核的 `classify_substitution` 给出（`output.rs:1006-1013`），
 * 本模块只读那个布尔。前端自己判近音需要读音表，而读音表在 `yunjian-recite` 里；
 * 在这里再写一份就等于把「什么算近音」这条判据分叉成两份。
 */

import type { ReciteOp } from "../contracts/recite";

/**
 * 标记类别键。**它是 `data-op` 属性的取值**，因此也是测试与样式的共同锚点。
 *
 * 前五个与 [`ReciteOp`] 的 `kind` 逐字相同（含 `re_recitation` 那个两词 snake_case），
 * 第六个 `near_homophone_substitution` 是本层新增的——它在 Rust 侧不是一个独立变体，
 * 而是 `substitution` 加一个布尔。取名把这层关系写在名字里，而不是叫 `homophone`
 * 之类看不出出身的短名。
 */
export const OP_MARK_KINDS = [
  "normal",
  "deletion",
  "insertion",
  "re_recitation",
  "substitution",
  "near_homophone_substitution",
] as const;

export type OpMarkKind = (typeof OP_MARK_KINDS)[number];

/** 一类差异的完整视觉描述。 */
export interface OpMark {
  kind: OpMarkKind;
  /** 记号字符，取自命令行的 `OpOut::mark()`。 */
  mark: string;
  /** 中文名，取自命令行的记号图例。 */
  label: string;
  /** 样式类名。四个视觉维度都由这一个类承担，见 `recite.css`。 */
  className: string;
}

/** 六类的描述子表。顺序即图例顺序，与命令行那一行一致。 */
export const OP_MARKS: Record<OpMarkKind, OpMark> = {
  normal: { kind: "normal", mark: "✓", label: "相符", className: "recite-op--normal" },
  deletion: { kind: "deletion", mark: "✗", label: "漏读", className: "recite-op--deletion" },
  insertion: { kind: "insertion", mark: "＋", label: "增读", className: "recite-op--insertion" },
  re_recitation: {
    kind: "re_recitation",
    mark: "↻",
    label: "回读",
    className: "recite-op--rerecitation",
  },
  substitution: {
    kind: "substitution",
    mark: "≠",
    label: "替换",
    className: "recite-op--substitution",
  },
  near_homophone_substitution: {
    kind: "near_homophone_substitution",
    mark: "≈",
    label: "近音替换",
    className: "recite-op--nearhomophone",
  },
};

/**
 * 判出一项操作属于哪一类标记。
 *
 * 唯一的分支是替换那一支读 `near_homophone`；**没有任何一处在猜**。
 */
export function opMarkOf(op: ReciteOp): OpMark {
  if (op.kind === "substitution") {
    return op.near_homophone ? OP_MARKS.near_homophone_substitution : OP_MARKS.substitution;
  }
  return OP_MARKS[op.kind];
}

/**
 * 一项操作在逐字面板上占据的一格。
 *
 * `text` 是显示在格子里的字；`expected` 只有替换类才有，显示为右上角标的应读字。
 * 回读一格里可能有多字（内核给的 `text` 是整个被重读的区间）。
 */
export interface OpCell {
  op: ReciteOp;
  mark: OpMark;
  text: string;
  expected?: string;
  /** 无障碍与悬停用的一句说明，见 [`opExplain`]。 */
  description: string;
}

/**
 * 把操作列表铺成逐字格子。
 *
 * 顺序就是内核给的顺序（按作答过程排列，`align.rs:60-63`），**不重排、不合并**：
 * 合并相邻同类会让「连续漏了三个字」看起来像漏了一个词，而那是两件不同的事。
 */
export function opCells(ops: ReciteOp[]): OpCell[] {
  return ops.map((op) => {
    const mark = opMarkOf(op);
    const description = opExplain(op);
    switch (op.kind) {
      case "normal":
        return { op, mark, text: op.character, description };
      case "deletion":
        return { op, mark, text: op.reference, description };
      case "insertion":
        return { op, mark, text: op.attempt, description };
      case "re_recitation":
        return { op, mark, text: op.text, description };
      case "substitution":
        return { op, mark, text: op.attempt, expected: op.reference, description };
    }
  });
}

/**
 * 一项操作的一句说明。
 *
 * 措辞**逐字对齐命令行的 `OpOut::explain()`**（`output.rs:1037-1080`），包括
 * 「第 N 字」这种一基下标。两处说法不同会让同一条差异在终端与界面里像两回事。
 *
 * 相符项在命令行里返回 `None`（不进差异清单）；这里仍然给一句，因为逐字面板上
 * 每一格都要有可读的 `aria-label`——读屏用户不能靠颜色分辨哪一格是对的。
 */
export function opExplain(op: ReciteOp): string {
  switch (op.kind) {
    case "normal":
      return `第 ${ordinal(op.reference_index)} 字 相符：「${op.character}」`;
    case "deletion":
      return `第 ${ordinal(op.reference_index)} 字 漏读：应读「${op.reference}」`;
    case "insertion":
      return `作答第 ${ordinal(op.attempt_index)} 字 增读：多读了「${op.attempt}」`;
    case "re_recitation":
      return `第 ${ordinal(op.reference_start)}–${op.reference_end} 字 回读：重读了「${op.text}」`;
    case "substitution":
      return (
        `第 ${ordinal(op.reference_index)} 字 ` +
        `${op.near_homophone ? "近音替换" : "替换"}：` +
        `应读「${op.reference}」，实读「${op.attempt}」`
      );
  }
}

/**
 * 把零基下标换成一基显示下标。
 *
 * 这是**下标**上的加一，不是分数上的算术——命令行的 `explain()` 里是同一个
 * `reference_index + 1`（`output.rs:1045`）。抽成具名函数是为了让
 * `noScoreArithmetic` 那条守卫读到的是一个下标函数而不是散落的加号。
 */
function ordinal(zeroBased: number): number {
  return zeroBased + 1;
}

/**
 * 逐字面板下方的记号图例。
 *
 * 由 [`OP_MARKS`] 生成而不是写死一句话：写死会在某天改了某个记号之后与实际渲染
 * 不一致，而图例与渲染不一致比没有图例更糟。分隔符 `·` 与命令行那一行相同。
 */
export function opLegend(): string {
  return OP_MARK_KINDS.map((kind) => `${OP_MARKS[kind].mark} ${OP_MARKS[kind].label}`).join(" · ");
}
