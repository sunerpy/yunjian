/**
 * 结果视图：四个数字原样搬运、发音边界注记在屏、等级可调且最多提交一次。
 *
 * # 本组最重要的一条：**分数是搬来的，不是算出来的**
 *
 * [`INCONSISTENT_ATTEMPT`] 的 `ops` 与 `score` **刻意互相矛盾**：那份操作列表按内核
 * 公式算出来是完整度 `0.95`、字准 `0.8`，而 `score` 里写的是四个完全不同的数。
 * 于是任何一处在前端重算分数的实现——不管是从 `ops` 推、还是把 `fluency` 写死成
 * 中性满值——都会渲染出与断言不同的数字。
 *
 * 这比「grep 源码里没有乘号」强：grep 只拦得住写成算术表达式的那一种形态，
 * 而这一条拦的是「界面显示的数与内核给的数不同」这件事本身。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReciteAttempt, ReciteCommit, ReciteOp } from "../../contracts/recite";
import ResultView from "../ResultView";

/**
 * 五类差异齐备的操作列表。
 *
 * 按内核公式（参考长度 20、漏读 1、增读 1、替换 2、回读 1）应得完整度 `0.95`、
 * 严格字准 `0.8`。下面的 `score` 故意不是这两个数。
 */
const OPS: ReciteOp[] = [
  { kind: "normal", reference_index: 0, attempt_index: 0, character: "床" },
  { kind: "deletion", reference_index: 1, reference: "前" },
  { kind: "insertion", reference_index: 2, attempt_index: 1, attempt: "啊" },
  {
    kind: "re_recitation",
    reference_start: 2,
    reference_end: 5,
    attempt_start: 2,
    attempt_end: 5,
    text: "明月光",
  },
  {
    kind: "substitution",
    reference_index: 12,
    attempt_index: 12,
    reference: "望",
    attempt: "看",
    near_homophone: false,
  },
  {
    kind: "substitution",
    reference_index: 17,
    attempt_index: 17,
    reference: "思",
    attempt: "丝",
    near_homophone: true,
  },
];

/**
 * 一份分数与操作列表刻意不一致的载荷。
 *
 * 四个数都取成「没有任何公式会算出来」的值：`fluency` 尤其不是打字路径的中性满值
 * `1`，所以把它写死成 `1.000` 的实现也会被这一组抓住。
 */
const INCONSISTENT_ATTEMPT: ReciteAttempt = {
  poem_id: "fixture-jingyesi",
  title: "静夜思",
  author: "李白",
  dynasty: "唐",
  mode: "cloze",
  ratio: 0.3,
  seed: 7,
  prompt: "床前明＿光，疑是地上＿。",
  hidden_indices: [3, 9],
  reference: "床前明月光疑是地上霜举头望明月低头思故乡",
  answer: "床明啊明月光疑是地上霜举头看明月低头丝乡",
  score: {
    completeness: 0.421,
    accuracy_strict: 0.317,
    accuracy_lenient: 0.622,
    fluency: 0.538,
    is_rejected: false,
    ops_summary: {
      normal_count: 1,
      deletion_count: 1,
      insertion_count: 1,
      rerecitation_count: 1,
      substitution_count: 2,
    },
  },
  ops: OPS,
  suggested_grade: "hard",
  first_attempt: false,
  database: "/tmp/fixture/recite.db",
};

const COMMIT: ReciteCommit = {
  grade: "good",
  grade_source: "user_chosen",
  database: "/tmp/fixture/recite.db",
  review: {
    poem_id: "fixture-jingyesi",
    due_day: 20680,
    last_review_day: 20675,
    scheduled_days: 5,
    stability: 6.25,
    difficulty: 5.5,
    last_grade: "good",
  },
};

describe("分数展示", () => {
  it("**四个数原样取自 `score`，与操作列表推出来的数不同也照样显示**", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    // 断言的是 `score` 里那四个数，而不是 `ops` 推出来的 0.95 / 0.8。
    expect(screen.getByTestId("score-completeness").textContent).toBe("0.421");
    expect(screen.getByTestId("score-strict-accuracy").textContent).toBe("0.317");
    expect(screen.getByTestId("score-lenient-accuracy").textContent).toBe("0.622");
    expect(screen.getByTestId("score-fluency").textContent).toBe("0.538");
  });

  it("不出现百分号，也不出现由这四个数合成的总分", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    const facts = screen.getByTestId("score-facts").textContent ?? "";
    expect(facts).not.toContain("%");
    // 只有这四个数值格子，没有第五个「总分」。
    expect(screen.getByTestId("score-facts").querySelectorAll("dd")).toHaveLength(4);
  });

  it("四个标签用的是内核那套说法", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    const facts = screen.getByTestId("score-facts").textContent ?? "";
    for (const label of ["完整度", "严格字准", "宽容字准", "节奏连贯度"]) {
      expect(facts, `缺「${label}」`).toContain(label);
    }
  });
});

describe("发音边界", () => {
  it("**结果视图含「不评估发音标准度」**", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    expect(screen.getByTestId("pronunciation-boundary").textContent).toContain("不评估发音标准度");
  });

  it("注记紧跟分数表，中间不隔别的段落", () => {
    // 亲手 QA 时它被一句口径说明推开了约 40px。「节奏连贯度 1.000」需要那句否认
    // 就在旁边，否则会被读成「读得很标准」。
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    const facts = screen.getByTestId("score-facts");
    const boundary = screen.getByTestId("pronunciation-boundary");
    expect(facts.nextElementSibling, "分数表与注记之间插了别的元素").toBe(boundary);
  });

  it("结果区渲染出来的文字里没有未解析的 Markdown 标记", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    expect(screen.getByLabelText("作答结果").textContent ?? "").not.toContain("**");
  });

  it("节奏连贯度四周不出现任何把它说成发音质量的词", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    const boundary = screen.getByTestId("pronunciation-boundary").textContent ?? "";
    expect(boundary).toContain("不评估发音标准度");
    // 「发音」只允许出现在那句否认里，所以先把那句剔掉再查禁词——否则
    // 「不评估发音标准度」自己就会命中「发音标准」。
    const rest = (screen.getByLabelText("作答结果").textContent ?? "").replace(boundary, "");
    for (const forbidden of ["发音", "读音", "口音", "读得标准"]) {
      expect(rest, `注记之外出现了「${forbidden}」`).not.toContain(forbidden);
    }
  });

  it("语音路径不做机器评分这件事也写在屏上", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    const note = screen.getByTestId("voice-grade-note").textContent ?? "";
    expect(note).toContain("语音路径不做机器评分");
    expect(note).toContain("由你自己选定");
  });

  it("被拒绝识别时说的是「内核建议记为最低档」，不是「已记为」", () => {
    // 桌面端把评级拆成两步，此刻还没落账，说「已记为」就是假话。
    const rejected: ReciteAttempt = {
      ...INCONSISTENT_ATTEMPT,
      score: { ...INCONSISTENT_ATTEMPT.score, is_rejected: true },
      suggested_grade: "again",
    };
    render(<ResultView attempt={rejected} commit={null} busy={false} onCommit={vi.fn()} />);
    const note = screen.getByTestId("rejected-note").textContent ?? "";
    expect(note).toContain("内核判为拒绝识别");
    expect(note).toContain("内核建议记为最低档");
    expect(note).not.toContain("本次记为最低档");
  });
});

describe("等级确认", () => {
  it("默认预选内核建议的等级，并说明它的来源", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={vi.fn()} />,
    );
    expect(screen.getByTestId("grade-hard").getAttribute("aria-pressed")).toBe("true");
    for (const other of ["again", "good", "easy"]) {
      expect(screen.getByTestId(`grade-${other}`).getAttribute("aria-pressed")).toBe("false");
    }
    expect(screen.getByTestId("grade-explanation").textContent).toContain("内核建议「困难」");
  });

  it("四档都能选中，选了别的就记为用户指定", () => {
    const onCommit = vi.fn();
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={onCommit} />,
    );
    fireEvent.click(screen.getByTestId("grade-easy"));
    expect(screen.getByTestId("grade-easy").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("grade-explanation").textContent).toContain("已改为「轻松」");

    fireEvent.click(screen.getByTestId("commit-grade"));
    expect(onCommit).toHaveBeenCalledWith("easy", true);
  });

  it("原样采纳建议值时记为打字映射而不是用户指定", () => {
    const onCommit = vi.fn();
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={onCommit} />,
    );
    fireEvent.click(screen.getByTestId("commit-grade"));
    expect(onCommit).toHaveBeenCalledWith("hard", false);
  });

  it("落账之后按钮与等级选择一起锁住，一局最多提交一次", async () => {
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={null} busy={false} onCommit={onCommit} />,
    );
    rerender(
      <ResultView
        attempt={INCONSISTENT_ATTEMPT}
        commit={COMMIT}
        busy={false}
        onCommit={onCommit}
      />,
    );
    await waitFor(() => {
      expect((screen.getByTestId("commit-grade") as HTMLButtonElement).disabled).toBe(true);
    });
    for (const grade of ["again", "hard", "good", "easy"]) {
      expect((screen.getByTestId(`grade-${grade}`) as HTMLButtonElement).disabled).toBe(true);
    }
    fireEvent.click(screen.getByTestId("commit-grade"));
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("落账后显示排程结果，间隔与到期日序原样取自载荷", () => {
    render(
      <ResultView attempt={INCONSISTENT_ATTEMPT} commit={COMMIT} busy={false} onCommit={vi.fn()} />,
    );
    expect(screen.getByTestId("commit-grade-label").textContent).toBe("良好");
    expect(screen.getByTestId("commit-grade-source").textContent).toContain("手动指定");
    expect(screen.getByTestId("commit-scheduled-days").textContent).toBe("5 天");
    expect(screen.getByTestId("commit-due-day").textContent).toBe("20680");
  });
});
