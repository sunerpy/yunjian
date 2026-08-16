import { Channel, invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTauriSettingsPorts, SETTINGS_IPC_COMMANDS } from "../sampleSettingsPorts";
import { createTauriPorts, IPC_COMMANDS } from "../tauriPorts";

vi.mock("@tauri-apps/api/core", () => {
  class TestChannel<T = unknown> {
    onmessage: (message: T) => void = () => undefined;
  }

  return {
    Channel: TestChannel,
    invoke: vi.fn(() => Promise.resolve({ kind: "absent" })),
  };
});

/**
 * 从某次 `invoke` 调用里取出交出去的那个 Channel。
 *
 * 断言要问的是「Rust 会往里发的那个对象」，所以必须取**实参本身**，
 * 而不是另建一个同类对象来比对——后者恒真。
 */
function handedOverChannel(command: string): Channel<unknown> {
  const call = vi
    .mocked(invoke)
    .mock.calls.find(([name]) => name === command)
    ?.at(1) as Record<string, unknown> | undefined;
  const channel = call?.onEvent;
  expect(channel, `${command} 没有把 onEvent 交给 Tauri`).toBeInstanceOf(Channel);
  return channel as Channel<unknown>;
}

describe("Tauri 长任务 Channel 契约", () => {
  beforeEach(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    vi.mocked(invoke).mockClear();
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  });

  it("appreciate_poem 把必需的 onEvent Channel 交给 Tauri", async () => {
    const ports = createTauriPorts();
    expect(ports).not.toBeNull();

    await ports?.appreciation.appreciate({ poem_id: "p-1" });

    expect(invoke).toHaveBeenCalledWith(
      IPC_COMMANDS.appreciate,
      expect.objectContaining({ onEvent: expect.any(Channel) }),
    );
  });

  it("fetch_corpus 把必需的 onEvent Channel 交给 Tauri", async () => {
    const ports = createTauriSettingsPorts();
    expect(ports).not.toBeNull();

    await ports?.corpus.fetchCorpus(() => undefined);

    expect(invoke).toHaveBeenCalledWith(
      SETTINGS_IPC_COMMANDS.fetchCorpus,
      expect.objectContaining({ onEvent: expect.any(Channel) }),
    );
  });

  /**
   * 这一条与上一条不是重复。
   *
   * 上一条只问「Channel 在不在实参里」，而 PR #105 补齐之后 `fetch_corpus`
   * 恰恰是**建了却不订阅**：`new Channel()` 之后没给 `onmessage` 赋值，注释还自陈
   * 「设置面板暂不渲染进度」。于是命令跑得通、Rust 一路发进度，事件全被丢在地上，
   * 首启物化 474,043 首诗全程无反馈——而上一条断言照绿。
   *
   * 所以这里往交出去的那个 Channel 里**真发一条事件**，要求它落到调用方给的回调上。
   * 「建了通道 ≠ 读了通道」这件事只能这样问出来。
   */
  it("fetch_corpus 交出去的 Channel 真被订阅：Rust 发的进度落到调用方", async () => {
    const ports = createTauriSettingsPorts();
    expect(ports).not.toBeNull();
    const received: unknown[] = [];

    await ports?.corpus.fetchCorpus((event) => {
      received.push(event);
    });

    const channel = handedOverChannel(SETTINGS_IPC_COMMANDS.fetchCorpus);
    const sent = {
      type: "progress",
      payload: { stage: "deriving", step: "构建候选索引", done: 1, total: 4 },
    };
    channel.onmessage(sent);

    expect(received, "Rust 往 Channel 里发的进度没有落到调用方——通道建了但没订阅").toEqual([sent]);
  });
});
