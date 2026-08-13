/**
 * 背诵屏的整局流程：出题 → 作答 → 结果 → 落账，以及「最多提交一次」。
 *
 * # 这一组盯的是接线，不是各个面板自己的行为
 *
 * 面板级测试能全绿而整局走不通——todo 62 的教训正是这样：14 个文件、37 条断言全绿，
 * 但页面上没有任何入口能到它。所以这一组从 `<ReciteScreen />` 出发，只点用户看得见的
 * 按钮，并且**逐一记录端口收到的参数**：形态、比例、种子有没有真的传下去。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ReciteAttempt, ReciteSession } from "../../contracts/recite";
import type {
  ReciteAnswerRequest,
  ReciteCommitRequest,
  RecitePorts,
  ReciteSessionRequest,
} from "../../data/recitePorts";
import ReciteScreen from "../ReciteScreen";

const SESSION: ReciteSession = {
  poem_id: "fixture-jingyesi",
  title: "静夜思",
  author: "李白",
  dynasty: "唐",
  mode: "cloze",
  ratio: 0.3,
  seed: 4242,
  prompt: "床前明＿光，疑是地上＿。举头望明月，低＿思故乡。",
  hidden_indices: [3, 9, 16],
  line_count: 4,
};

const ATTEMPT: ReciteAttempt = {
  ...SESSION,
  reference: "床前明月光疑是地上霜举头望明月低头思故乡",
  answer: "床前明月光疑是地上霜举头看明月低头思故乡",
  score: {
    completeness: 1,
    accuracy_strict: 0.95,
    accuracy_lenient: 0.95,
    fluency: 1,
    is_rejected: false,
    ops_summary: {
      normal_count: 19,
      deletion_count: 0,
      insertion_count: 0,
      rerecitation_count: 0,
      substitution_count: 1,
    },
  },
  ops: [
    { kind: "normal", reference_index: 0, attempt_index: 0, character: "床" },
    {
      kind: "substitution",
      reference_index: 12,
      attempt_index: 12,
      reference: "望",
      attempt: "看",
      near_homophone: false,
    },
  ],
  suggested_grade: "good",
  first_attempt: true,
  database: "/tmp/fixture/recite.db",
};

interface Recorder {
  ports: RecitePorts;
  sessions: ReciteSessionRequest[];
  answers: ReciteAnswerRequest[];
  commits: ReciteCommitRequest[];
}

function recorder(session: ReciteSession = SESSION): Recorder {
  const sessions: ReciteSessionRequest[] = [];
  const answers: ReciteAnswerRequest[] = [];
  const commits: ReciteCommitRequest[] = [];
  const ports: RecitePorts = {
    practice: {
      startSession: (request) => {
        sessions.push(request);
        return Promise.resolve(session);
      },
      submitAnswer: (request) => {
        answers.push(request);
        return Promise.resolve(ATTEMPT);
      },
    },
    review: {
      commitGrade: (request) => {
        commits.push(request);
        return Promise.resolve({
          grade: request.grade,
          grade_source: request.chosen_by_user ? "user_chosen" : "typed_mapping",
          database: ATTEMPT.database,
          review: {
            poem_id: request.poem_id,
            due_day: 20681,
            last_review_day: 20675,
            scheduled_days: 6,
            stability: 7.5,
            difficulty: 5,
            last_grade: request.grade,
          },
        });
      },
      due: () =>
        Promise.resolve({ database: ATTEMPT.database, scope: "due_today" as const, items: [] }),
      stats: () =>
        Promise.resolve({
          database: ATTEMPT.database,
          scheduled_total: 0,
          due_today: 0,
          by_last_grade: { again: 0, hard: 0, good: 0, easy: 0 },
          grading: {
            again_completeness_below: 0.6,
            hard_accuracy_lenient_below: 0.85,
            hard_rerecitation_above: 0,
            easy_accuracy_strict_at_least: 0.97,
          },
        }),
    },
  };
  return { ports, sessions, answers, commits };
}

/** 走到「已出题」那一步。 */
async function start(recorded: Recorder): Promise<void> {
  render(<ReciteScreen ports={recorded.ports} defaultPoemId="fixture-jingyesi" />);
  fireEvent.click(screen.getByTestId("start-session"));
  await waitFor(() => {
    expect(screen.getByTestId("session-prompt")).toBeTruthy();
  });
}

describe("整局流程", () => {
  it("四种形态都能选，选中态跟着换", async () => {
    const recorded = recorder();
    render(<ReciteScreen ports={recorded.ports} />);
    for (const mode of ["cloze", "first-char", "masked", "voice"]) {
      fireEvent.click(screen.getByTestId(`mode-${mode}`));
      expect(screen.getByTestId(`mode-${mode}`).getAttribute("aria-pressed")).toBe("true");
    }
  });

  it("**挖空比例改动真的传到出题请求里**", async () => {
    // 这一条盯的是 todo 62 踩过的坑的同一形态：控件动了但值没传下去。
    // 断言的是端口收到的参数，而不是界面上那个数字。
    const recorded = recorder();
    render(<ReciteScreen ports={recorded.ports} defaultPoemId="fixture-jingyesi" />);
    fireEvent.change(screen.getByTestId("cloze-ratio"), { target: { value: "0.65" } });
    expect(screen.getByTestId("cloze-ratio-value").textContent).toContain("0.65");

    fireEvent.click(screen.getByTestId("start-session"));
    await waitFor(() => {
      expect(recorded.sessions).toHaveLength(1);
    });
    expect(recorded.sessions[0]?.ratio).toBe(0.65);
    expect(recorded.sessions[0]?.mode).toBe("cloze");
    expect(recorded.sessions[0]?.poem_id).toBe("fixture-jingyesi");
  });

  it("遮挡档位改动同样传下去，且比例控件在该形态下不出现", async () => {
    const recorded = recorder();
    render(<ReciteScreen ports={recorded.ports} defaultPoemId="fixture-jingyesi" />);
    fireEvent.click(screen.getByTestId("mode-masked"));
    expect(screen.queryByTestId("cloze-ratio")).toBeNull();

    fireEvent.change(screen.getByTestId("masked-lines"), { target: { value: "3" } });
    fireEvent.click(screen.getByTestId("start-session"));
    await waitFor(() => {
      expect(recorded.sessions).toHaveLength(1);
    });
    expect(recorded.sessions[0]?.masked_lines).toBe(3);
    expect(recorded.sessions[0]?.mode).toBe("masked");
  });

  it("出题后显示提示文本与空位数，且屏上没有参考诗文", async () => {
    const recorded = recorder();
    await start(recorded);
    expect(screen.getByTestId("session-prompt").textContent).toBe(SESSION.prompt);
    expect(screen.getByTestId("session-mode").textContent).toContain("挖空 3 处");
    expect(screen.getByTestId("session-mode").textContent).toContain("种子 4242");
    // 参考诗文在类型上就不在出题载荷里，所以它不可能出现在屏上。
    expect(document.body.textContent).not.toContain(ATTEMPT.reference);
  });

  it("空作答不能提交", async () => {
    const recorded = recorder();
    await start(recorded);
    expect((screen.getByTestId("submit-answer") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByTestId("empty-answer-hint").textContent).toContain("你没做过的记录");
    fireEvent.click(screen.getByTestId("submit-answer"));
    expect(recorded.answers).toHaveLength(0);
  });

  it("**提交作答后渲染逐字反馈，并把种子原样带回去**", async () => {
    const recorded = recorder();
    await start(recorded);
    fireEvent.change(screen.getByTestId("recite-answer"), {
      target: { value: "床前明月光，疑是地上霜。举头看明月，低头思故乡。" },
    });
    fireEvent.click(screen.getByTestId("submit-answer"));

    await waitFor(() => {
      expect(screen.getByTestId("op-feedback")).toBeTruthy();
    });
    expect(recorded.answers).toHaveLength(1);
    // 种子必须回传：内核的会话是无状态重建的，少带它就会重建成另一局挖空。
    expect(recorded.answers[0]?.seed).toBe(4242);
    expect(recorded.answers[0]?.answer).toContain("举头看明月");
    // 结果视图的三块都在。
    expect(screen.getByTestId("score-facts")).toBeTruthy();
    expect(screen.getByTestId("pronunciation-boundary").textContent).toContain("不评估发音标准度");
    // 提交过之后输入锁住，避免同一局评第二次。
    expect((screen.getByTestId("recite-answer") as HTMLTextAreaElement).disabled).toBe(true);
  });

  it("**一局最多落账一次**", async () => {
    const recorded = recorder();
    await start(recorded);
    fireEvent.change(screen.getByTestId("recite-answer"), { target: { value: "床前明月光" } });
    fireEvent.click(screen.getByTestId("submit-answer"));
    await waitFor(() => {
      expect(screen.getByTestId("commit-grade")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("commit-grade"));
    await waitFor(() => {
      expect(screen.getByTestId("commit-facts")).toBeTruthy();
    });
    // 再点两次都不该产生第二次提交——`commitGrade` 每调一次就是 FSRS 里的一次复习。
    fireEvent.click(screen.getByTestId("commit-grade"));
    fireEvent.click(screen.getByTestId("commit-grade"));
    expect(recorded.commits).toHaveLength(1);
    expect(recorded.commits[0]).toEqual({
      poem_id: "fixture-jingyesi",
      grade: "good",
      chosen_by_user: false,
    });
  });

  it("语音形态的退化原因来自载荷，界面不自己编一句", async () => {
    const fallback: ReciteSession = {
      ...SESSION,
      requested_mode: "voice",
      fallback_reason:
        "语音会话尚未接入本版本；已退化为挖空打字练习并照常计入排程；评分内核与语音路径完全相同",
    };
    const recorded = recorder(fallback);
    render(<ReciteScreen ports={recorded.ports} defaultPoemId="fixture-jingyesi" />);
    fireEvent.click(screen.getByTestId("mode-voice"));
    fireEvent.click(screen.getByTestId("start-session"));

    await waitFor(() => {
      expect(screen.getByTestId("voice-fallback")).toBeTruthy();
    });
    const note = screen.getByTestId("voice-fallback").textContent ?? "";
    expect(note).toContain("请求的形态是「语音」");
    expect(note).toContain("语音会话尚未接入本版本");
    // 「已退化但照常计入排程」这半句必须在：少了它用户会以为这次没练成。
    expect(note).toContain("照常计入排程");
    expect(recorded.sessions[0]?.mode).toBe("voice");
  });

  it("换一首之后上一局的结果不留在屏上", async () => {
    const recorded = recorder();
    await start(recorded);
    fireEvent.change(screen.getByTestId("recite-answer"), { target: { value: "床前明月光" } });
    fireEvent.click(screen.getByTestId("submit-answer"));
    await waitFor(() => {
      expect(screen.getByTestId("score-facts")).toBeTruthy();
    });

    fireEvent.change(screen.getByTestId("recite-poem-id"), { target: { value: "fixture-other" } });
    await waitFor(() => {
      expect(screen.queryByTestId("score-facts")).toBeNull();
    });
    expect(screen.queryByTestId("session-prompt")).toBeNull();
  });
});
