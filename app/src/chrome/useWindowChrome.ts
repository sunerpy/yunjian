/**
 * 自绘标题栏的状态层：持有最大化状态与窗口动作，且**在非 Tauri 宿主下也能安全渲染**。
 *
 * 呈现（`TitleBar.tsx`）与 IPC（`windowControls.ts`）之间只隔这一层，因为只有这一层有副作用
 * ——订阅、异步初始化、卸载清理。把它单独拿出来的收益是：呈现层变成纯函数，
 * 而这里的三处时序缺陷可以在无 GUI 的 jsdom 里被逐条钉住。
 *
 * # 最大化状态为什么要「初始查询 + 订阅」两条腿
 *
 * 两条都不可省，各自覆盖对方漏掉的情形：
 *
 * - **只订阅不初始查询**：窗口被系统或窗口状态插件**以最大化状态恢复**时不会发出任何
 *   resize 事件，于是按钮上的图标一直是「最大化」而窗口其实已经最大化，直到用户手动
 *   拖一下窗口才纠正。
 * - **只初始查询不订阅**：双击标题栏（Tauri 注入脚本干的）、`Win`+方向键、系统窗口菜单
 *   都会改变最大化状态而**不经过我们的按钮**，图标随即与真实状态脱节。
 *
 * # `disposed` 守卫为什么是必需的而不是防御性代码
 *
 * `main.tsx` 刻意开着 `StrictMode`，它在开发模式下把 effect 跑成
 * 「挂载 → effect → 清理 → effect」。于是第一次 effect 里那两个 `await`
 * （`isMaximized()` 与 `onResized()`）会在**它自己已经被清理之后**才落地。
 * 没有守卫就会：往已卸载的实例写状态、以及把第二次订阅之外的一个订阅漏掉不取消。
 * 后者是真实泄漏——每次热更新多留一个监听器。
 *
 * 守卫覆盖三个落点，缺一个就漏一种：
 * 1. 初始 `isMaximized()` resolve 之后写状态之前；
 * 2. `onResized()` resolve 之后**存句柄之前**（此刻若已 disposed，必须当场取消它，
 *    否则这个订阅永远没人取消）；
 * 3. resize 回调触发时（回调可能在清理之后才被投递）。
 */

import { useCallback, useEffect, useState } from "react";
import { type ChromePlatform, currentPlatform, hasNativeWindowButtons } from "./platform";
import {
  type UnsubscribeFn,
  type WindowControls,
  createWindowControls,
  safeUnsubscribe,
} from "./windowControls";

/**
 * 可注入的依赖。默认取真实实现，测试传替身。
 *
 * 刻意用参数注入而不是模块级 mock：后者要动全局，于是「测替身」与「测真实降级路径」
 * 两类用例会互相污染，而降级路径恰恰要靠真实的 `createWindowControls` 抛异常来验证。
 */
export interface WindowChromeDeps {
  createControls: () => WindowControls | null;
  getPlatform: () => ChromePlatform;
}

/** 呈现层需要的全部东西。它不直接接触 `windowControls`，也不自己发 IPC。 */
export interface WindowChrome {
  platform: ChromePlatform;
  /** `null` 表示不在 Tauri 宿主里。呈现层据此渲染**无按钮**的标题栏。 */
  controls: WindowControls | null;
  isMaximized: boolean;
  /** 是否渲染自绘的三个窗口按钮。macOS 由系统画红绿灯，非 Tauri 宿主没有窗口可控。 */
  showWindowButtons: boolean;
  minimize: () => void;
  toggleMaximize: () => void;
  close: () => void;
  /**
   * 触摸 / 手写笔的拖动入口。
   *
   * **鼠标必须走 Tauri 的注入脚本，不能走这里。** 注入脚本 `drag.js` 做了两件本模块
   * 无法复制的事：一是按平台区分决策时机（Windows/Linux 在 `mousedown`，macOS 在
   * `mouseup` 且要求指针没移动，以匹配「移开即取消」的原生语义），二是双击时改发
   * `internal_toggle_maximize`。它只监听鼠标事件，看不到触摸与手写笔——这个缺口才是
   * 本函数存在的唯一理由。
   *
   * 传 `pointerType === "mouse"` 时**什么都不做**：那会与注入脚本同时发起拖动。
   */
  dragForNonMousePointer: (pointerType: string) => void;
}

/**
 * 让被拒的 IPC 至少留下一条痕迹。
 *
 * 权限缺失时 Tauri 是**拒 promise**而不是报错弹窗，所以「按钮点了没反应」在默认情况下
 * 连一行日志都没有。这里把它打到控制台：不能修好权限，但能让排查从「玄学」变成一条可读信息。
 * 同时它也消掉 unhandled rejection。
 */
function reportIpcFailure(action: string, error: unknown): void {
  // eslint-disable-next-line no-console -- 这是给开发者看的唯一诊断出口，不是产品输出
  console.error(`窗口操作 ${action} 失败（多为 capabilities 缺少对应权限）：`, error);
}

const defaultDeps: WindowChromeDeps = {
  createControls: createWindowControls,
  getPlatform: currentPlatform,
};

export function useWindowChrome(deps: Partial<WindowChromeDeps> = {}): WindowChrome {
  const { createControls, getPlatform } = { ...defaultDeps, ...deps };

  // 两者都只在挂载时求一次：平台不会变，窗口句柄换了也没有意义（label 是固定的）。
  const [platform] = useState<ChromePlatform>(() => getPlatform());
  const [controls] = useState<WindowControls | null>(() => createControls());
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!controls) {
      // 非 Tauri 宿主：没有窗口可查也没有事件可订阅，保持默认状态。
      return;
    }

    let disposed = false;
    let unsubscribe: UnsubscribeFn | null = null;

    const refresh = (): void => {
      // 守卫落点 3 的前半：回调可能在清理之后才被投递（事件已在队列里）。
      // 这里就返回，连那次 IPC 都不发——发出去也只会得到一个没人要的结果。
      if (disposed) {
        return;
      }
      controls
        .isMaximized()
        .then((value) => {
          // 守卫落点 3：回调可能在清理之后才被投递。
          if (!disposed) {
            setIsMaximized(value);
          }
        })
        .catch((error: unknown) => {
          reportIpcFailure("isMaximized", error);
        });
    };

    void (async () => {
      try {
        const initial = await controls.isMaximized();
        // 守卫落点 1。
        if (disposed) {
          return;
        }
        setIsMaximized(initial);

        const off = await controls.onResized(refresh);
        // 守卫落点 2：已经清理过了，这个订阅必须当场取消，否则它永远没人管。
        if (disposed) {
          await safeUnsubscribe(off);
          return;
        }
        unsubscribe = off;
      } catch (error) {
        reportIpcFailure("最大化状态订阅", error);
      }
    })();

    return () => {
      disposed = true;
      const pending = unsubscribe;
      unsubscribe = null;
      // 清理函数不能是 async，也不能抛：`safeUnsubscribe` 两种失败形态都吞。
      void safeUnsubscribe(pending);
    };
  }, [controls]);

  const minimize = useCallback(() => {
    controls?.minimize().catch((error: unknown) => {
      reportIpcFailure("minimize", error);
    });
  }, [controls]);

  const toggleMaximize = useCallback(() => {
    // 用 `toggleMaximize`（对应 `core:window:allow-toggle-maximize`），
    // **不是** `core:default` 里那条 `allow-internal-toggle-maximize`——后者是注入脚本
    // 双击时用的另一条命令。两条各自需要各自的权限，缺哪条都是点了没反应。
    controls?.toggleMaximize().catch((error: unknown) => {
      reportIpcFailure("toggleMaximize", error);
    });
  }, [controls]);

  const close = useCallback(() => {
    controls?.close().catch((error: unknown) => {
      reportIpcFailure("close", error);
    });
  }, [controls]);

  const dragForNonMousePointer = useCallback(
    (pointerType: string) => {
      if (pointerType === "mouse") {
        // 见 `WindowChrome.dragForNonMousePointer` 的文档：鼠标归注入脚本。
        return;
      }
      controls?.startDragging().catch((error: unknown) => {
        reportIpcFailure("startDragging", error);
      });
    },
    [controls],
  );

  return {
    platform,
    controls,
    isMaximized,
    showWindowButtons: controls !== null && !hasNativeWindowButtons(platform),
    minimize,
    toggleMaximize,
    close,
    dragForNonMousePointer,
  };
}
