/**
 * 复习队列：内容必须来自 `recite due` 的输出，等级显示且可调。
 *
 * # 队列里的每一个数都对着载荷断言
 *
 * 替身返回的 `due` 载荷里那三条各有不同的间隔、稳定度与难度，断言逐项对上。
 * 这样「界面自己排了个序」「界面把稳定度当难度显示」这类错都会红——它们在
 * 只断言「有三行」的测试里完全看不见。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReciteCommit, ReciteDue, ReciteStats } from "../../contracts/recite";
import type { ReciteCommitRequest, ReciteReviewPort } from "../../data/recitePorts";
import ReviewQueue from "../ReviewQueue";

const DUE_TODAY: ReciteDue = {
  database: "/tmp/fixture/recite.db",
  scope: "due_today",
  items: [
    {
      poem_id: "fixture-a",
      due_day: 20670,
      last_review_day: 20668,
      scheduled_days: 2,
      stability: 3.41,
      difficulty: 5.62,
      last_grade: "hard",
    },
    {
      poem_id: "fixture-b",
      due_day: 20674,
      last_review_day: 20667,
      scheduled_days: 7,
      stability: 9.08,
      difficulty: 4.15,
      last_grade: "good",
    },
  ],
};

const DUE_ALL: ReciteDue = {
  ...DUE_TODAY,
  scope: "all",
  items: [
    ...DUE_TODAY.items,
    {
      poem_id: "fixture-future",
      due_day: 20702,
      last_review_day: 20672,
      scheduled_days: 30,
      stability: 31.5,
      difficulty: 3.04,
      last_grade: "easy",
    },
  ],
};

const STATS: ReciteStats = {
  database: "/tmp/fixture/recite.db",
  scheduled_total: 3,
  due_today: 2,
  by_last_grade: { again: 0, hard: 1, good: 1, easy: 1 },
  grading: {
    again_completeness_below: 0.6,
    hard_accuracy_lenient_below: 0.85,
    hard_rerecitation_above: 0,
    easy_accuracy_strict_at_least: 0.97,
  },
};

interface Recorder {
  port: ReciteReviewPort;
  commits: ReciteCommitRequest[];
}

function recorder(): Recorder {
  const commits: ReciteCommitRequest[] = [];
  const port: ReciteReviewPort = {
    commitGrade: (request) => {
      commits.push(request);
      const commit: ReciteCommit = {
        grade: request.grade,
        grade_source: request.chosen_by_user ? "user_chosen" : "typed_mapping",
        database: DUE_TODAY.database,
        review: { ...DUE_TODAY.items[0]!, poem_id: request.poem_id, last_grade: request.grade },
      };
      return Promise.resolve(commit);
    },
    due: (includeFuture) => Promise.resolve(includeFuture ? DUE_ALL : DUE_TODAY),
    stats: () => Promise.resolve(STATS),
  };
  return { port, commits };
}

describe("复习队列", () => {
  it("**队列内容逐项反映 `recite due` 的输出**", async () => {
    const { port } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("review-queue")).toBeTruthy();
    });
    const items = screen.getAllByTestId("queue-item");
    expect(items).toHaveLength(2);

    // 顺序就是载荷的顺序：内核已按到期日升序，界面不重排。
    expect(items[0]?.textContent).toContain("fixture-a");
    expect(items[1]?.textContent).toContain("fixture-b");

    // 逐个数字对着载荷断言，而不是只看「有几行」。
    expect(items[0]?.textContent).toContain("间隔 2 天");
    expect(items[0]?.textContent).toContain("到期日序 20670");
    expect(items[0]?.textContent).toContain("上次日序 20668");
    expect(items[0]?.textContent).toContain("稳定度 3.41");
    expect(items[0]?.textContent).toContain("难度 5.62");
    expect(items[1]?.textContent).toContain("间隔 7 天");
    expect(items[1]?.textContent).toContain("稳定度 9.08");
    expect(items[1]?.textContent).toContain("难度 4.15");
  });

  it("**每一项显示最近一次的等级**", async () => {
    const { port } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByTestId("queue-grade-fixture-a").textContent).toContain("困难");
    });
    expect(screen.getByTestId("queue-grade-fixture-b").textContent).toContain("良好");
    // 当前等级在四个按钮上体现为选中态，用户看得出「现在是哪一档」。
    expect(screen.getByTestId("queue-regrade-fixture-a-hard").getAttribute("aria-pressed")).toBe(
      "true",
    );
    expect(screen.getByTestId("queue-regrade-fixture-a-good").getAttribute("aria-pressed")).toBe(
      "false",
    );
  });

  it("**等级可调，且调整是一次真实的复习提交**", async () => {
    const { port, commits } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByTestId("queue-regrade-fixture-a-easy")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("queue-regrade-fixture-a-easy"));
    await waitFor(() => {
      expect(commits).toHaveLength(1);
    });
    expect(commits[0]).toEqual({
      poem_id: "fixture-a",
      grade: "easy",
      chosen_by_user: true,
    });

    // 界面必须说清这不是改标签：内核没有「修正上一次评级」的入口。
    const note = screen.getByTestId("regrade-note").textContent ?? "";
    expect(note).toContain("一次新的复习");
    expect(note).toContain("没有「修正上一次评级」的入口");
  });

  it("含未到期开关切到 `--all` 那一份载荷", async () => {
    const { port } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getAllByTestId("queue-item")).toHaveLength(2);
    });
    fireEvent.click(screen.getByTestId("include-future"));
    await waitFor(() => {
      expect(screen.getAllByTestId("queue-item")).toHaveLength(3);
    });
    expect(screen.getByTestId("queue-grade-fixture-future").textContent).toContain("轻松");
  });

  it("统计与阈值都原样取自载荷，且说明分布的口径", async () => {
    const { port } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByTestId("queue-stats")).toBeTruthy();
    });
    const stats = screen.getByTestId("queue-stats").textContent ?? "";
    expect(stats).toContain("已排程 3 首");
    expect(stats).toContain("今天到期 2 首");
    expect(stats).toContain("困难 1");
    // 分布是「每首最近一次」而不是历次直方图，这一条必须说出来。
    expect(stats).toContain("最近一次");

    const thresholds = screen.getByTestId("grading-thresholds").textContent ?? "";
    expect(thresholds).toContain("0.6");
    expect(thresholds).toContain("0.85");
    expect(thresholds).toContain("0.97");
    expect(thresholds).toContain("等级由内核按严格优先级判定");
  });

  it("空队列时给的是一句能操作的提示，不是空白", async () => {
    const empty: ReciteReviewPort = {
      commitGrade: () => Promise.reject(new Error("不该被调到")),
      due: () => Promise.resolve({ ...DUE_TODAY, items: [] }),
      stats: () => Promise.resolve({ ...STATS, scheduled_total: 0, due_today: 0 }),
    };
    render(<ReviewQueue port={empty} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByTestId("queue-empty").textContent).toContain("没有到期项");
    });
    expect(screen.queryByTestId("review-queue")).toBeNull();
  });

  it("**渲染出来的文字里没有未解析的 Markdown 标记**", async () => {
    // 这一条来自一次亲手 QA：我在 JSX 文字里写了 `**粗体**`，而 JSX 不解析 Markdown，
    // 浏览器上原样显示成两个星号。单元测试当时全绿——它们只查关键词是否出现，
    // 而星号并不影响关键词匹配。所以把「看一眼」得到的结论固化成断言。
    const { port } = recorder();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByTestId("review-queue")).toBeTruthy();
    });
    const rendered = screen.getByLabelText("复习队列").textContent ?? "";
    expect(rendered, "渲染文字里出现了字面的 ** ").not.toContain("**");
    // 顺带钉住那两条说明的措辞：它们现在是常量，不是 JSX 里的散字。
    expect(screen.getByTestId("regrade-note").textContent).toBe(
      "点某一档等级会作为一次新的复习提交给排程器，间隔与稳定度随之改变。" +
        "内核没有「修正上一次评级」的入口，所以这不是改标签。",
    );
    // 跨行 JSX 文字会把换行折叠成空格，于是句号后多一个空格——同一轮 QA 抓到的第二个缺陷。
    expect(screen.getByTestId("regrade-note").textContent).not.toContain("。 ");
  });

  it("「练这一首」把标识交给上层，而不是自己去开语料库补标题", async () => {
    const { port } = recorder();
    const onPractice = vi.fn();
    render(<ReviewQueue port={port} refreshToken={0} onPractice={onPractice} />);
    await waitFor(() => {
      expect(screen.getByTestId("queue-practice-fixture-b")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("queue-practice-fixture-b"));
    expect(onPractice).toHaveBeenCalledWith("fixture-b");
    // 队列行里没有题目与作者：那需要开语料库，而排程本来与语料无关。
    expect(screen.getAllByTestId("queue-item")[1]?.textContent).not.toContain("静夜思");
  });
});
