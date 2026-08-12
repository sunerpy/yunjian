/**
 * 自绘标题栏的 IPC 层：把 `@tauri-apps/api/window` 收敛成一个**可能为 `null`** 的窄接口。
 *
 * # 为什么必须包 try/catch 而不是直接 import 调用
 *
 * `getCurrentWindow()` 不是「在浏览器里返回一个哑对象」，而是**直接抛异常**：它的实现
 * （`@tauri-apps/api/window.js`）第一行就读
 * `window.__TAURI_INTERNALS__.metadata.currentWindow.label`，纯浏览器里
 * `__TAURI_INTERNALS__` 是 `undefined`，于是抛 `TypeError`。
 *
 * 而「纯浏览器」不是边缘情况，它是三个日常场景：`vite dev` 开在标签页里调样式、
 * Vitest 的 jsdom、Playwright 直接访问 dev server。这三处只要抛一次异常，
 * 组件树就整棵起不来——一条自绘标题栏把整个应用变成白屏。
 *
 * 所以本模块的契约是：**拿不到窗口就返回 `null`，由呈现层渲染一条无按钮的标题栏**。
 * 刻意不返回「方法全是空实现」的假窗口：那会让 UI 画出三个点了没反应的按钮，
 * 比不画更糟——它看起来是好的。
 *
 * # `onResized` 的回调刻意不带参数
 *
 * Tauri 的 `onResized` 会带一个 `PhysicalSize`。这里把它抹掉是有意的：最大化状态
 * **不能**从尺寸推断（无边框窗口最大化后的尺寸与「手动拖到占满工作区」无法区分，
 * 多显示器下更是如此）。唯一可靠的判据是再问一次 `isMaximized()`。
 * 不把 payload 交出去，调用方就无法开始依赖它。
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

/** 取消订阅函数。Tauri 把它标成 `() => void`，但实现是 `async`，见 `safeUnsubscribe`。 */
export type UnsubscribeFn = () => void;

/**
 * 自绘标题栏真正用到的窗口能力，一条不多。
 *
 * 这个集合与 `crates/yunjian-app/capabilities/default.json` 里补的权限**一一对应**：
 * 每多一个方法就要多授一条权限，而权限缺失是**静默失败**（IPC promise 被拒，UI 无反应）。
 * 把接口收窄到这五项，是让「该授哪些权限」可以被读出来而不是靠记。
 */
export interface WindowControls {
  /** 当前是否最大化。`core:window:default` 已含 `allow-is-maximized`，无需额外授权。 */
  isMaximized(): Promise<boolean>;
  /** 订阅尺寸变化。回调不带 payload，见模块文档。 */
  onResized(handler: () => void): Promise<UnsubscribeFn>;
  /** 开始拖动窗口。**仅用于触摸与手写笔**，鼠标拖动由 Tauri 注入脚本负责。 */
  startDragging(): Promise<void>;
  /** 最大化 / 还原。注意它与注入脚本双击用的 `internal_toggle_maximize` 是两条命令。 */
  toggleMaximize(): Promise<void>;
  minimize(): Promise<void>;
  /**
   * 关闭窗口。
   *
   * 刻意用 `close()` 而不是自定义一条 `quit_app` 命令：`close()` 会走
   * `tauri://close-requested`，将来若要做托盘常驻或「确认退出」拦截，那条既有路径仍然
   * 生效。自定义命令等于开一条绕过拦截的第二出口。
   */
  close(): Promise<void>;
}

/**
 * 取当前窗口的控制句柄；不在 Tauri 宿主里时返回 `null`。
 *
 * 不缓存结果：句柄本身极轻（只包一个 label），而缓存会让「先在浏览器里渲染、后被注入」
 * 这种顺序永久停在降级态。
 */
export function createWindowControls(): WindowControls | null {
  let appWindow: ReturnType<typeof getCurrentWindow>;
  try {
    appWindow = getCurrentWindow();
  } catch {
    // 非 Tauri 宿主。刻意不打日志：dev server 与每一次 Vitest 都会走到这里，
    // 打出来就是一条永远存在的噪音，反而盖掉真实告警。
    return null;
  }

  return {
    isMaximized: () => appWindow.isMaximized(),
    // 丢掉 `PhysicalSize` 参数：见模块文档。
    onResized: (handler: () => void) => appWindow.onResized(() => handler()),
    startDragging: () => appWindow.startDragging(),
    toggleMaximize: () => appWindow.toggleMaximize(),
    minimize: () => appWindow.minimize(),
    close: () => appWindow.close(),
  };
}

/**
 * 取消订阅，且**两种失败形态都吞掉**。
 *
 * # 为什么取消订阅会失败
 *
 * `@tauri-apps/api` 的 `_unlisten` 读的是 `window.__TAURI_EVENT_PLUGIN_INTERNALS__`——
 * 一个与 `__TAURI_INTERNALS__` **不同**的全局，由 Rust 侧的事件插件注入。一个只 mock 了
 * `invoke` 的半宿主（测试替身、预览构建）能让订阅成功而让取消订阅失败。
 *
 * # 为什么必须同时接住同步抛出与被拒的 promise
 *
 * `UnlistenFn` 的类型是 `() => void`，实现却是 `async`。于是失败有两种到达方式：
 * 调用当场抛，或返回一个没人 await 的被拒 promise（后者在 Node 里是
 * `unhandledRejection`，在浏览器里是一条控制台错误）。
 *
 * # 为什么不能让它抛出去
 *
 * 这个函数只在 `useEffect` 的清理函数里被调用。**清理函数里抛出的异常会被 React 当成
 * 渲染期错误，整棵树卸载失败**——于是「取消订阅出错」这件本身零代价的事（监听器随
 * WebView 一起消亡）会把整个界面搞掉。丢掉这个错误的代价是零，让它逃出去的代价是白屏。
 */
export async function safeUnsubscribe(unsubscribe: UnsubscribeFn | null): Promise<void> {
  if (!unsubscribe) {
    return;
  }
  try {
    // 实现是 async，所以返回值可能是一个 promise；`await` 同时覆盖同步返回与异步返回。
    await (unsubscribe() as unknown as Promise<void> | void);
  } catch {
    // 见函数文档：这里必须什么都不做。
  }
}
