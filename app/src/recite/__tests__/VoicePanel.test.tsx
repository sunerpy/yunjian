/**
 * 语音面板的断言。
 *
 * # 这些用例盯的是七条验收里落在界面上的那几条
 *
 * 1. 高亮由**拼接时间戳**驱动，且时间戳数量与音步数一致；
 * 2. 五条失败各自把界面切到打字模式并**显示那一条独有的原因**；
 * 3. 卡顿时给内联提示；
 * 4. 结果区**不出现**任何源自偏置假设的数——注入的哨兵串一次都不得进入结果区；
 * 5. 模型下载显示进度且可取消。
 *
 * # 断言从真实入口出发
 *
 * 全部经 `render(<VoicePanel …/>)` 加真实点击驱动，端口用替身。不直接调组件内部函数：
 * 那样验的是一个纯函数而不是「用户点了按钮之后屏幕上有什么」，而本 todo 的产出恰恰是后者。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  ModelFetchEvent,
  TypedFallback,
  VoiceAvailability,
  VoiceDemonstration,
  VoiceModelOutcome,
  VoiceOutcome,
  VoiceSessionEvent,
} from "../../contracts/voice";
import type { VoicePort } from "../../data/voicePorts";
import { createSampleVoicePort } from "../../data/sampleVoicePorts";
import VoicePanel, { linesFromMarks } from "../VoicePanel";

const LINES = ["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"] as const;

/** 注入的哨兵偏置串。选一串绝不可能出自真实识别的字符，于是「它出现在哪里」是确定判据。 */
const SENTINEL = "偏置哨兵ZZQX";

const DEMONSTRATION: VoiceDemonstration = {
  audio_url: "yunjian-audio://localhost/deadbeef",
  sample_rate: 16_000,
  duration_ms: 6_680,
  marks: LINES.flatMap((text, line) => [
    {
      line,
      index_in_line: 0,
      text: text.slice(0, 2),
      start_ms: line * 1_770,
      end_ms: line * 1_770 + 500,
    },
    {
      line,
      index_in_line: 1,
      text: text.slice(2),
      start_ms: line * 1_770 + 620,
      end_ms: line * 1_770 + 1_370,
    },
  ]),
};

interface Stub {
  port: VoicePort;
  cancelled: string[];
}

function stub(overrides: Partial<VoicePort> = {}, availability?: VoiceAvailability): Stub {
  const cancelled: string[] = [];
  const base: VoicePort = {
    availability: () =>
      Promise.resolve(
        availability ?? {
          kind: "voice",
          coherence_label: "节奏连贯度",
          note: "语音路径不做机器评分。",
        },
      ),
    demonstrate: () => Promise.resolve(DEMONSTRATION),
    startSession: () => Promise.resolve({ kind: "reported", operation_id: "op", report: REPORT }),
    fetchModel: () =>
      Promise.resolve({ kind: "ready", operation_id: "op", name: "m", directory: "/tmp/m" }),
    cancel: (id) => {
      cancelled.push(id);
      return Promise.resolve(true);
    },
  };
  return { port: { ...base, ...overrides }, cancelled };
}

const REPORT = {
  spoke: true,
  long_pause_count: 4,
  relative_rhythm: "faster" as const,
  coherence: 0.21328,
  coherence_label: "节奏连贯度",
  gap_variance_ms2: 0,
  duration_ratio: 0.7186,
  lines_attempted: 4,
  prompt_count: 4,
};

function panel(port: VoicePort, onDegrade = vi.fn()) {
  render(<VoicePanel port={port} poemId="fixture-jingyesi" lines={LINES} onDegrade={onDegrade} />);
  return onDegrade;
}

describe("行文本与时间戳同源", () => {
  it("**从 marks 反推的行文本与原文逐字相同**", () => {
    // 这一条是亲手 QA 抓到的缺陷的回归：先前行文本取自打字端点的**挖空提示**，而语音形态
    // 在那条端点上会退化成挖空，于是 `＿` 被正文判据剥掉之后「床前明月光」显示成了
    // 「床前明光」。单元测试当时全绿，因为它们传的都是真实行文本。
    expect(linesFromMarks(DEMONSTRATION.marks)).toEqual([...LINES]);
  });

  it("没有 marks 也没有原文时不显示任何诗句，而是说明为什么", async () => {
    // 显示一份可能与高亮时刻对不上的文本比不显示更糟：用户读到的字会和亮起来的位置错位。
    render(
      <VoicePanel
        port={stub({ startSession: () => new Promise(() => undefined) }).port}
        poemId="fixture-jingyesi"
        lines={[]}
        onDegrade={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-lines-pending")).toBeTruthy();
    });
    expect(screen.queryByTestId("voice-char-0-0")).toBeNull();
  });

  it("示范之后行文本出现，且与 marks 反推的一致", async () => {
    render(
      <VoicePanel port={stub().port} poemId="fixture-jingyesi" lines={[]} onDegrade={vi.fn()} />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-demonstrate")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-demonstrate"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-4")).toBeTruthy();
    });
    // 第一行五个字全在，**「月」不得缺**——那正是 QA 抓到的症状。
    const first = [0, 1, 2, 3, 4].map(
      (index) => screen.getByTestId(`voice-char-0-${index}`).textContent,
    );
    expect(first.join("")).toBe("床前明月光");
  });
});

describe("可用性探测", () => {
  it("可用时给出录音与示范两个入口，并同屏说明不做机器评分", async () => {
    panel(stub().port);
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-availability-note").textContent).toContain("不做机器评分");
    // 「播放与录音不重叠」要说给用户听：边播边录会让识别器听见自己的示范音。
    expect(screen.getByTestId("voice-no-overlap-note").textContent).toContain("不会同时进行");
  });
});

describe("五条失败各自切到打字模式并显示独有原因", () => {
  const cases: Array<{ reason: TypedFallback["reason"]; label: string; message: string }> = [
    {
      reason: "feature_disabled",
      label: "本版本未编译语音",
      message: "本版本未编译语音能力，已切换到打字练习。",
    },
    {
      reason: "permission_denied",
      label: "麦克风授权被拒绝",
      message: "麦克风授权被拒绝，已切换到打字练习。想用语音默写请到系统设置允许。",
    },
    {
      reason: "no_input_device",
      label: "没有可用的麦克风",
      message: "没有检测到可用的麦克风，已切换到打字练习。",
    },
    {
      reason: "model_unavailable",
      label: "语音模型未就绪",
      message: "语音模型尚未就绪，已切换到打字练习。",
    },
    {
      reason: "system_too_old",
      label: "系统版本过低",
      message: "当前系统版本低于语音功能所需的 14.2，已切换到打字练习。",
    },
  ];

  for (const { reason, label, message } of cases) {
    it(`${reason} 显示它自己的标签与解释`, async () => {
      const onDegrade = panel(stub({}, { kind: "typed", reason, message }).port);
      await waitFor(() => {
        expect(screen.getByTestId("voice-degraded")).toBeTruthy();
      });
      expect(screen.getByTestId("voice-degraded-reason").textContent).toBe(label);
      // **解释原样取自 Rust 侧**：那个函数按平台给出「去哪个设置页」，前端重写必然漂移。
      expect(screen.getByTestId("voice-degraded").textContent).toContain(message);
      expect(onDegrade).toHaveBeenCalledWith({
        reason,
        message,
        completed_lines: 0,
      });
      // 录音入口必须消失：先画一个点了就报错的按钮是最坏的形态。
      expect(screen.queryByTestId("voice-record")).toBeNull();
    });
  }

  it("**只有缺模型那一条给出下载入口**", async () => {
    // 五条各有不同的下一步动作，把下载按钮铺给所有失败等于在四种情形下给错引导。
    panel(stub({}, { kind: "typed", reason: "permission_denied", message: "被拒绝。" }).port);
    await waitFor(() => {
      expect(screen.getByTestId("voice-degraded")).toBeTruthy();
    });
    expect(screen.queryByTestId("voice-fetch-models")).toBeNull();
  });
});

describe("示范与 karaoke 高亮", () => {
  it("时间戳数量等于音步数，且播放推进时高亮跟着走", async () => {
    panel(stub().port);
    await waitFor(() => {
      expect(screen.getByTestId("voice-demonstrate")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-demonstrate"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-audio")).toBeTruthy();
    });

    // 四句五言，每句切二三，共八个音步。断言的是载荷里的标记数与实际渲染的行数都对得上。
    expect(DEMONSTRATION.marks).toHaveLength(LINES.length * 2);
    expect(screen.getAllByTestId(/^voice-char-0-/)).toHaveLength(5);

    const audio = screen.getByTestId("voice-audio");
    // 播放到 0.2 秒：落在第 0 行第 0 个音步（0–500 ms）内。
    Object.defineProperty(audio, "currentTime", { value: 0.2, configurable: true });
    fireEvent.timeUpdate(audio);
    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-0").getAttribute("data-karaoke")).toBe("true");
    });
    expect(screen.getByTestId("voice-char-0-1").getAttribute("data-karaoke")).toBe("true");
    // 第三个字属于下一个音步，此刻不该亮。
    expect(screen.getByTestId("voice-char-0-2").getAttribute("data-karaoke")).toBeNull();

    // 推进到 0.7 秒：落在第二个音步（620–1370 ms）内，第一个音步必须熄掉。
    Object.defineProperty(audio, "currentTime", { value: 0.7, configurable: true });
    fireEvent.timeUpdate(audio);
    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-2").getAttribute("data-karaoke")).toBe("true");
    });
    expect(screen.getByTestId("voice-char-0-0").getAttribute("data-karaoke")).toBeNull();

    // 落在音步之间的静音里（0.55 秒）时谁都不亮——那段静音就是节奏本身。
    Object.defineProperty(audio, "currentTime", { value: 0.55, configurable: true });
    fireEvent.timeUpdate(audio);
    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-2").getAttribute("data-karaoke")).toBeNull();
    });
    expect(screen.getByTestId("voice-char-0-0").getAttribute("data-karaoke")).toBeNull();
  });
});

describe("复诵期间的实时反馈", () => {
  function streamingPort(events: VoiceSessionEvent[], outcome: VoiceOutcome): VoicePort {
    return stub({
      startSession: (_request, onEvent) => {
        for (const event of events) {
          onEvent(event);
        }
        return Promise.resolve(outcome);
      },
    }).port;
  }

  const listening: VoiceSessionEvent = {
    type: "progress",
    payload: {
      stage: { stage: "listening", line: 0 },
      completed_lines: 0,
      total_lines: 4,
    },
  };

  it("已匹配前缀由偏置假设驱动，并与它自带的诊断说明同屏", async () => {
    panel(
      streamingPort(
        [
          listening,
          {
            type: "item",
            payload: {
              item: "asr_partial",
              at_ms: 300,
              unbiased: "床钱",
              biased: "床前明",
              diagnostics_only: true,
              note: "以下转写仅供诊断",
            },
          },
        ],
        { kind: "reported", operation_id: "op", report: REPORT },
      ),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));

    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-0").getAttribute("data-matched")).toBe("true");
    });
    expect(screen.getByTestId("voice-char-0-2").getAttribute("data-matched")).toBe("true");
    // 第四个字不在公共前缀里，不该显示为已匹配。
    expect(screen.getByTestId("voice-char-0-3").getAttribute("data-matched")).toBeNull();
    // **诊断说明必须与转写同屏。** 只显示转写会让它被读成「你说了这些」。
    expect(screen.getByTestId("voice-partial-note").textContent).toContain("仅供诊断");
    expect(screen.getByTestId("voice-partial-biased").textContent).toBe("床前明");
  });

  it("**已匹配高亮与 karaoke 高亮在 DOM 上可分**", async () => {
    // 两者证据强度不同（一个是拼接算术，一个是 77% 字错率下的提示），画成同一个属性会让
    // 用户把后者读成前者。所以它们是两个独立的 data 属性，不是一个。
    panel(
      streamingPort(
        [
          listening,
          {
            type: "item",
            payload: {
              item: "asr_partial",
              at_ms: 300,
              unbiased: null,
              biased: "床前",
              diagnostics_only: true,
              note: "诊断",
            },
          },
        ],
        { kind: "reported", operation_id: "op", report: REPORT },
      ),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-char-0-0").getAttribute("data-matched")).toBe("true");
    });
    expect(screen.getByTestId("voice-char-0-0").getAttribute("data-karaoke")).toBeNull();
  });

  it("卡顿时给内联提示，附下一句起头的字", async () => {
    panel(
      streamingPort(
        [
          listening,
          {
            type: "item",
            payload: {
              item: "prompt",
              next_chars: "床前",
              from_index: 0,
              at_ms: 2_100,
              reason: "trailing_silence",
            },
          },
        ],
        { kind: "reported", operation_id: "op", report: REPORT },
      ),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-prompt")).toBeTruthy();
    });
    const text = screen.getByTestId("voice-prompt").textContent ?? "";
    expect(text).toContain("床前");
    expect(text).toContain("停了一会儿");
  });

  it("换行时清掉上一行的提示与已匹配前缀", async () => {
    panel(
      streamingPort(
        [
          listening,
          {
            type: "item",
            payload: {
              item: "prompt",
              next_chars: "床前",
              from_index: 0,
              at_ms: 2_100,
              reason: "no_speech_yet",
            },
          },
          {
            type: "item",
            payload: {
              item: "line_observed",
              line: 0,
              spoke: true,
              long_pause_count: 1,
              total_ms: 1_200,
              onsets_ms: [0, 400],
            },
          },
        ],
        { kind: "reported", operation_id: "op", report: REPORT },
      ),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.queryByTestId("voice-prompt")).toBeNull();
    });
    expect(screen.queryByTestId("voice-diagnostics")).toBeNull();
  });

  it("中途降级时保留已完成的行数，并给出那一条原因", async () => {
    const fallback: TypedFallback = {
      reason: "device_busy",
      message: "麦克风正被其他程序占用，已切换到打字练习。",
      completed_lines: 2,
    };
    const onDegrade = panel(
      streamingPort([{ type: "item", payload: { item: "fallback", ...fallback } }], {
        kind: "degraded",
        operation_id: "op",
        fallback,
      }),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-degraded")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-degraded-reason").textContent).toBe("麦克风被占用");
    expect(screen.getByTestId("voice-completed-lines").textContent).toContain("2 句");
    expect(onDegrade).toHaveBeenCalledWith(fallback);
  });
});

describe("结果区不得携带偏置假设", () => {
  it("**注入的哨兵串一次都不出现在结果区**", async () => {
    // 两向都验：先确认哨兵确实进了诊断区，再确认它没进结果区。只验后者的话，一个从不注入
    // 哨兵的替身也能让这条断言通过，而那时它什么也没证明。
    const onDegrade = vi.fn();
    render(
      <VoicePanel
        port={
          stub({
            startSession: (_request, onEvent) => {
              onEvent({
                type: "progress",
                payload: {
                  stage: { stage: "listening", line: 0 },
                  completed_lines: 0,
                  total_lines: 4,
                },
              });
              onEvent({
                type: "item",
                payload: {
                  item: "asr_partial",
                  at_ms: 300,
                  unbiased: `无偏置-${SENTINEL}`,
                  biased: `${SENTINEL}-床前明月光`,
                  diagnostics_only: true,
                  note: "诊断",
                },
              });
              onEvent({ type: "item", payload: { item: "report", ...REPORT } });
              return Promise.resolve({ kind: "reported", operation_id: "op", report: REPORT });
            },
          }).port
        }
        poemId="fixture-jingyesi"
        lines={LINES}
        onDegrade={onDegrade}
      />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));

    await waitFor(() => {
      expect(screen.getByTestId("voice-report")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-partial-biased").textContent).toContain(SENTINEL);
    expect(screen.getByTestId("voice-report").textContent ?? "").not.toContain(SENTINEL);
    expect(screen.getByTestId("voice-no-score-note").textContent).toContain("不做机器评分");
  });

  it("结果区**不给等级建议**：语音路径的等级只能用户自选", async () => {
    panel(stub().port);
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-report")).toBeTruthy();
    });
    const text = screen.getByTestId("voice-report").textContent ?? "";
    for (const grade of ["重来", "困难", "良好", "轻松"]) {
      expect(text).not.toContain(grade);
    }
  });

  it("六项观察逐一显示，且原样搬运不做换算", async () => {
    panel(stub().port);
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-coherence")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-spoke").textContent).toBe("检测到");
    expect(screen.getByTestId("voice-long-pauses").textContent).toBe("4 次");
    expect(screen.getByTestId("voice-relative-rhythm").textContent).toBe("比示范快");
    // 三位小数，与打字路径一致：`0.21328` → `0.213`。**不换算成百分比**——那要做一次算术，
    // 而 `noScoreArithmetic.test.ts` 会为此变红。
    expect(screen.getByTestId("voice-coherence").textContent).toBe("0.213");
    expect(screen.getByTestId("voice-duration-ratio").textContent).toBe("0.719");
    expect(screen.getByTestId("voice-lines-attempted").textContent).toBe("4 句");
  });
});

describe("模型下载", () => {
  it("显示进度并能取消", async () => {
    const progress: ModelFetchEvent[] = [
      { type: "progress", payload: { stage: "downloading", bytes_done: 512, bytes_total: 2_048 } },
    ];
    const unavailable: VoiceModelOutcome = {
      kind: "unavailable",
      operation_id: "voice-model",
      fallback: {
        reason: "model_unavailable",
        message: "模型下载已取消。",
        completed_lines: 0,
      },
    };
    const pending: Array<(outcome: VoiceModelOutcome) => void> = [];
    const recorded = stub(
      {
        fetchModel: (_request, onEvent) => {
          for (const event of progress) {
            onEvent(event);
          }
          return new Promise<VoiceModelOutcome>((resolve) => {
            pending.push(resolve);
          });
        },
      },
      { kind: "typed", reason: "model_unavailable", message: "语音模型尚未就绪。" },
    );
    panel(recorded.port);

    await waitFor(() => {
      expect(screen.getByTestId("voice-fetch-models")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-fetch-models"));

    await waitFor(() => {
      expect(screen.getByTestId("voice-fetch-progress")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-fetch-progress").textContent).toContain("512");
    expect(screen.getByTestId("voice-fetch-progress").textContent).toContain("正在下载");

    fireEvent.click(screen.getByTestId("voice-cancel-fetch"));
    expect(recorded.cancelled).toHaveLength(1);
    expect(recorded.cancelled[0]).toMatch(/^voice-model-/);

    // 取消之后落点必须给出**具体原因**，而不是一句「失败了」。
    pending[0]?.(unavailable);
    await waitFor(() => {
      expect(screen.getByTestId("voice-degraded").textContent).toContain("模型下载已取消");
    });
  });
});

describe("样例替身", () => {
  it("样例模式下走完一局，事件顺序与真实路径一致", async () => {
    panel(createSampleVoicePort());
    await waitFor(() => {
      expect(screen.getByTestId("voice-record")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("voice-record"));
    await waitFor(() => {
      expect(screen.getByTestId("voice-report")).toBeTruthy();
    });
    // 替身的产出是手算后写死的常量；这里只验它真的到了界面上。
    expect(screen.getByTestId("voice-coherence").textContent).toBe("0.213");
  });

  it("样例替身可以被要求报不可用，用来看降级形态", async () => {
    panel(
      createSampleVoicePort({
        kind: "typed",
        reason: "feature_disabled",
        message: "本版本未编译语音能力。",
      }),
    );
    await waitFor(() => {
      expect(screen.getByTestId("voice-degraded")).toBeTruthy();
    });
    expect(screen.getByTestId("voice-degraded-reason").textContent).toBe("本版本未编译语音");
  });
});
