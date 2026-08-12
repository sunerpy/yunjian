/**
 * IPC 层：非 Tauri 降级、命令名映射、取消订阅的两种失败形态。
 *
 * 这里的正反对照是刻意成对的：只断言「纯浏览器返回 null」会被一个「永远返回 null」的
 * 实现满足，那样降级永远成立而真实窗口永远拿不到——所以必须同时有一条在 mock 宿主下
 * **拿到非 null** 的断言。
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { createWindowControls, safeUnsubscribe } from "../windowControls";

afterEach(() => {
  clearMocks();
});

describe("createWindowControls 的非 Tauri 降级", () => {
  it("纯浏览器里返回 null 而不是抛异常", () => {
    // `getCurrentWindow()` 的实现第一行就读
    // `window.__TAURI_INTERNALS__.metadata.currentWindow.label`，纯浏览器里那是 undefined，
    // 于是抛 TypeError。不接住它就等于让 Vite dev、Vitest、Playwright 三处全部白屏。
    expect((globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__).toBeUndefined();
    expect(createWindowControls()).toBeNull();
  });

  it("确实会抛：直接调 getCurrentWindow 在同一环境下失败", async () => {
    // 这条是上一条的正向对照。没有它，「返回 null」可能只是因为我们从没真正调用过它。
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    expect(() => getCurrentWindow()).toThrow();
  });

  it("mock 宿主里返回非 null，证明降级不是无条件的", () => {
    mockWindows("main");
    mockIPC(() => null);
    expect(createWindowControls()).not.toBeNull();
  });
});

describe("WindowControls 的命令名映射", () => {
  it("toggleMaximize 走 toggle_maximize，而不是 core:default 里那条 internal_toggle_maximize", async () => {
    // 这是本 todo 最容易静默失败的一处：`core:default` 已经授了
    // `allow-internal-toggle-maximize`（注入脚本双击用它），于是很容易以为按钮也够用了。
    // 按钮走的是**另一条命令** `plugin:window|toggle_maximize`，需要
    // `core:window:allow-toggle-maximize`。缺它时 IPC promise 被拒，按钮点了没反应且不报错。
    mockWindows("main");
    const invoked: string[] = [];
    mockIPC((cmd) => {
      invoked.push(cmd);
      return null;
    });

    const controls = createWindowControls();
    expect(controls).not.toBeNull();
    await controls?.toggleMaximize();

    expect(invoked).toEqual(["plugin:window|toggle_maximize"]);
    expect(invoked).not.toContain("plugin:window|internal_toggle_maximize");
  });

  it("四个动作各自映射到它自己那条命令", async () => {
    mockWindows("main");
    const invoked: string[] = [];
    mockIPC((cmd) => {
      invoked.push(cmd);
      return cmd === "plugin:window|is_maximized" ? true : null;
    });

    const controls = createWindowControls();
    await controls?.minimize();
    await controls?.close();
    await controls?.startDragging();
    const maximized = await controls?.isMaximized();

    // 这四条命令名与 `capabilities/default.json` 里补的四条权限一一对应，
    // 由 `capabilities.test.ts` 那侧钉住另一半。
    expect(invoked).toEqual([
      "plugin:window|minimize",
      "plugin:window|close",
      "plugin:window|start_dragging",
      "plugin:window|is_maximized",
    ]);
    expect(maximized).toBe(true);
  });
});

describe("safeUnsubscribe 的两种失败形态", () => {
  it("吞掉同步抛出", async () => {
    const throwing = () => {
      throw new Error("__TAURI_EVENT_PLUGIN_INTERNALS__ is undefined");
    };
    await expect(safeUnsubscribe(throwing)).resolves.toBeUndefined();
  });

  it("吞掉被拒的 promise", async () => {
    // `UnlistenFn` 的类型是 `() => void`，实现却是 async。于是失败还有第二种到达方式：
    // 一个没人 await 的被拒 promise。只 try/catch 接不住它。
    const rejecting = (() => Promise.reject(new Error("unlisten 失败"))) as unknown as () => void;
    await expect(safeUnsubscribe(rejecting)).resolves.toBeUndefined();
  });

  it("null 是合法输入（订阅还没建立就卸载了）", async () => {
    await expect(safeUnsubscribe(null)).resolves.toBeUndefined();
  });

  it("正常情况下确实调用了传进来的函数", async () => {
    const unsubscribe = vi.fn();
    await safeUnsubscribe(unsubscribe);
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });
});
