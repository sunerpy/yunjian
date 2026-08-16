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

    await ports?.corpus.fetchCorpus();

    expect(invoke).toHaveBeenCalledWith(
      SETTINGS_IPC_COMMANDS.fetchCorpus,
      expect.objectContaining({ onEvent: expect.any(Channel) }),
    );
  });
});
