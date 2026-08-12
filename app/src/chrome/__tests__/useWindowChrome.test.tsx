/**
 * 状态层的时序：初始查询、resize 重查、StrictMode 双调用下的三处 `disposed` 守卫。
 *
 * 这些是本 todo 唯一**能在无 GUI 环境里真正验证**的部分，也是最容易写错的部分——
 * 时序缺陷在真机上表现为「图标偶尔不对」「热更新几次之后监听器变多」，
 * 而这两种症状都不会报错。
 *
 * 替身刻意**每次都返回同一个对象**：`useState(() => createControls())` 在 StrictMode 下
 * 初始化器会被调两次，工厂每次造新对象会让 spy 计数散到两个实例上，于是断言测的
 * 不是真实行为。真实的 `createWindowControls` 也是等价的（同一个窗口 label）。
 */

import { StrictMode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { UnsubscribeFn, WindowControls } from "../windowControls";
import { useWindowChrome } from "../useWindowChrome";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

interface FakeHost {
  controls: WindowControls;
  createControls: () => WindowControls | null;
  isMaximized: ReturnType<typeof vi.fn>;
  onResized: ReturnType<typeof vi.fn>;
  unsubscribe: ReturnType<typeof vi.fn>;
  startDragging: ReturnType<typeof vi.fn>;
  toggleMaximize: ReturnType<typeof vi.fn>;
  /** 手动投递一次 resize 事件。刻意带 payload，以证明状态层不去读它。 */
  emitResize: () => void;
}

function fakeHost(
  options: {
    maximized?: boolean | Promise<boolean>;
    onResizedResult?: Promise<UnsubscribeFn>;
  } = {},
): FakeHost {
  const unsubscribe = vi.fn();
  const handlers: Array<(...args: unknown[]) => void> = [];

  const isMaximized = vi.fn(() =>
    options.maximized instanceof Promise
      ? options.maximized
      : Promise.resolve(options.maximized ?? false),
  );
  const onResized = vi.fn((handler: () => void) => {
    handlers.push(handler as (...args: unknown[]) => void);
    return options.onResizedResult ?? Promise.resolve(unsubscribe as UnsubscribeFn);
  });
  const startDragging = vi.fn(() => Promise.resolve());
  const toggleMaximize = vi.fn(() => Promise.resolve());

  const controls: WindowControls = {
    isMaximized: isMaximized as unknown as WindowControls["isMaximized"],
    onResized: onResized as unknown as WindowControls["onResized"],
    startDragging: startDragging as unknown as WindowControls["startDragging"],
    toggleMaximize: toggleMaximize as unknown as WindowControls["toggleMaximize"],
    minimize: vi.fn(() => Promise.resolve()) as unknown as WindowControls["minimize"],
    close: vi.fn(() => Promise.resolve()) as unknown as WindowControls["close"],
  };

  return {
    controls,
    createControls: () => controls,
    isMaximized,
    onResized,
    unsubscribe,
    startDragging,
    toggleMaximize,
    emitResize: () => {
      for (const handler of handlers) {
        handler({ payload: { width: 1920, height: 1080 } });
      }
    },
  };
}

describe("最大化状态的两条腿", () => {
  it("初始值来自 isMaximized()，而不是先假定 false 再等事件", async () => {
    // 窗口被系统或窗口状态插件**以最大化状态恢复**时不会发出任何 resize 事件。
    // 少了这次初始查询，按钮图标会一直错到用户手动拖一下窗口为止。
    const host = fakeHost({ maximized: true });
    const { result } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );

    await waitFor(() => {
      expect(result.current.isMaximized).toBe(true);
    });
    expect(host.isMaximized).toHaveBeenCalled();
  });

  it("resize 事件触发重新查询，而不是从事件 payload 推断", async () => {
    // 无边框窗口最大化后的尺寸与「手动拖到占满工作区」无法区分，多显示器下更是如此。
    // 唯一可靠的判据是再问一次 isMaximized()。替身刻意把 payload 传进回调，
    // 而状态层的回调签名根本收不到它。
    let maximized = false;
    const host = fakeHost();
    host.isMaximized.mockImplementation(() => Promise.resolve(maximized));

    const { result } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );
    await waitFor(() => {
      expect(host.onResized).toHaveBeenCalled();
    });
    expect(result.current.isMaximized).toBe(false);

    maximized = true;
    await act(async () => {
      host.emitResize();
    });

    expect(result.current.isMaximized).toBe(true);
  });
});

describe("StrictMode 下的 disposed 守卫", () => {
  it("订阅数与取消订阅数持平，没有留下孤儿监听器", async () => {
    // StrictMode 把 effect 跑成「挂载 → effect → 清理 → effect」。
    // 第一次 effect 的 `await isMaximized()` 会在它自己被清理之后才落地；
    // 守卫落点 1 让它当场返回，于是它**不会**再去建第二个订阅。
    // 少了那个守卫，这里会变成 2 次订阅、1 次取消——每次热更新泄漏一个监听器。
    const host = fakeHost();
    const { unmount } = renderHook(
      () => useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
      { wrapper: StrictMode },
    );

    await waitFor(() => {
      expect(host.onResized).toHaveBeenCalled();
    });
    unmount();
    await act(async () => {
      await Promise.resolve();
    });

    expect(host.unsubscribe.mock.calls.length).toBe(host.onResized.mock.calls.length);
  });

  it("初始查询在卸载之后才落地时，不写状态也不建订阅", async () => {
    // 守卫落点 1。没有它就是「往已卸载的实例写状态」加「建一个永远没人取消的订阅」。
    const pending = deferred<boolean>();
    const host = fakeHost({ maximized: pending.promise });
    const { unmount } = renderHook(
      () => useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
      { wrapper: StrictMode },
    );

    unmount();
    await act(async () => {
      pending.resolve(true);
      await Promise.resolve();
    });

    expect(host.onResized).not.toHaveBeenCalled();
  });

  it("订阅在卸载之后才建立时，那个订阅仍然被取消", async () => {
    // 守卫落点 2。`unsubscribe` 变量此刻还是 null，清理函数取消不到它；
    // 必须由 async 链自己在发现 disposed 之后当场取消，否则这个订阅永远没人管。
    const pendingSubscription = deferred<UnsubscribeFn>();
    const host = fakeHost({ onResizedResult: pendingSubscription.promise });
    const { unmount } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );

    await waitFor(() => {
      expect(host.onResized).toHaveBeenCalled();
    });
    unmount();
    await act(async () => {
      pendingSubscription.resolve(host.unsubscribe as unknown as UnsubscribeFn);
      await Promise.resolve();
    });

    expect(host.unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("卸载之后投递的 resize 回调不再查询窗口状态", async () => {
    // 守卫落点 3。回调可能在清理之后才被投递（事件已在队列里）。
    // 「卸载后没有状态写入」在 React 18 里无法直接观察（它不再警告），
    // 但**卸载后不再发起那次查询**是可观察的，且它正是那次写入的前置动作。
    const host = fakeHost();
    const { unmount } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );
    await waitFor(() => {
      expect(host.onResized).toHaveBeenCalled();
    });

    unmount();
    const callsAtUnmount = host.isMaximized.mock.calls.length;
    await act(async () => {
      host.emitResize();
      await Promise.resolve();
    });

    expect(host.isMaximized.mock.calls.length).toBe(callsAtUnmount);
  });
});

describe("非 Tauri 宿主", () => {
  it("controls 为 null 时不订阅、不显示按钮，也不抛异常", async () => {
    const { result } = renderHook(
      () => useWindowChrome({ createControls: () => null, getPlatform: () => "linux" }),
      { wrapper: StrictMode },
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.controls).toBeNull();
    expect(result.current.showWindowButtons).toBe(false);
    expect(result.current.isMaximized).toBe(false);
  });

  it("动作是安全的空操作，不会因为没有窗口而抛", () => {
    const { result } = renderHook(() =>
      useWindowChrome({ createControls: () => null, getPlatform: () => "linux" }),
    );

    expect(() => {
      result.current.minimize();
      result.current.toggleMaximize();
      result.current.close();
      result.current.dragForNonMousePointer("pen");
    }).not.toThrow();
  });
});

describe("按钮的显示条件", () => {
  it("macOS 不显示自绘按钮，因为红绿灯由系统绘制", () => {
    const host = fakeHost();
    const { result } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "macos" }),
    );
    expect(result.current.showWindowButtons).toBe(false);
  });

  it("Windows 与 Linux 显示自绘按钮", () => {
    for (const platform of ["windows", "linux"] as const) {
      const host = fakeHost();
      const { result } = renderHook(() =>
        useWindowChrome({ createControls: host.createControls, getPlatform: () => platform }),
      );
      expect(result.current.showWindowButtons).toBe(true);
    }
  });
});

describe("dragForNonMousePointer", () => {
  it("手写笔与触摸调 startDragging()", () => {
    const host = fakeHost();
    const { result } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );

    act(() => {
      result.current.dragForNonMousePointer("pen");
    });
    expect(host.startDragging).toHaveBeenCalledTimes(1);

    act(() => {
      result.current.dragForNonMousePointer("touch");
    });
    expect(host.startDragging).toHaveBeenCalledTimes(2);
  });

  it("鼠标不调 startDragging()，那条路归 Tauri 注入的脚本", () => {
    // 注入脚本 `drag.js` 做了两件这里无法复制的事：按平台区分决策时机
    // （Windows/Linux 在 mousedown，macOS 在 mouseup 且要求指针没移动），
    // 以及双击时改发 internal_toggle_maximize。对鼠标也调一次就是两处同时发起拖动。
    const host = fakeHost();
    const { result } = renderHook(() =>
      useWindowChrome({ createControls: host.createControls, getPlatform: () => "linux" }),
    );

    act(() => {
      result.current.dragForNonMousePointer("mouse");
    });
    expect(host.startDragging).not.toHaveBeenCalled();
  });
});
