/**
 * 自绘标题栏的平台探测层。
 *
 * # 为什么是 userAgent 而不是 `@tauri-apps/plugin-os`
 *
 * `plugin-os` 的 `platform()` 要付四样代价：一个 npm 包、一个 Cargo crate、一句
 * `.plugin(tauri_plugin_os::init())`、一条 capability 条目——只为回答一个 WebView 本来就
 * **同步**知道的问题。更要紧的是它在纯浏览器里根本没被注入，于是 Vitest 与 Playwright 里
 * 调它直接抛异常，而这两处正是本模块最需要能跑的地方。
 *
 * # 判定顺序不是风格，是正确性
 *
 * 顺序写错会得到一个**只在某一个平台上错、且不报错**的结果：
 *
 * 1. **移动端必须最先筛掉。** Android 的 UA 里含 `Linux`，iOS 的 UA 里含 `Mac OS X`。
 *    先判桌面平台会把手机认成 Linux 或 macOS，于是给手机渲染一条自绘标题栏。
 * 2. **Windows 在 macOS 之前。** 两者的判据不重叠，这条只是让 `Windows NT` 这个最明确的
 *    标记优先命中。
 * 3. **绝不能靠 `Safari` 判 macOS。** Linux 上 Tauri 的 WebView 是 WebKitGTK，它自报
 *    `AppleWebKit/605.1.15 (KHTML, like Gecko) Version/… Safari/605.1.15`——与 macOS 的
 *    Safari 几乎一致。真正能区分的是平台段：`Macintosh` 对 `X11; Linux`。
 *    按 `Safari` 判会把 Linux 认成 macOS，症状是 Linux 窗口左侧空出一块 80px 的红绿灯
 *    让位区、里面什么都没有，而且窗口控制按钮整条消失（macOS 分支刻意不渲染它们）。
 *
 * 本模块**没有副作用、不碰 DOM、不发 IPC**：`detectPlatform` 是纯函数，测试直接喂字符串。
 * 读取 `navigator` 的那一处单独隔在 `currentPlatform` 里，且它自己也带兜底。
 */

/** 自绘标题栏需要区分的平台。`unknown` 覆盖移动端、无 UA 的宿主与一切没识别出的情况。 */
export type ChromePlatform = "macos" | "windows" | "linux" | "unknown";

/**
 * 从一个 userAgent 字符串判定平台。纯函数。
 *
 * 判定顺序见模块文档；改动顺序前先读那段，每一条都对应一个真实的错判。
 */
export function detectPlatform(userAgent: string): ChromePlatform {
  const ua = userAgent;

  // 移动端先筛。这两条必须在桌面判据之前，否则 Android（UA 含 `Linux`）与
  // iOS（UA 含 `Mac OS X`）会被认成桌面平台。
  if (/Android/i.test(ua)) {
    return "unknown";
  }
  if (/iPhone|iPad|iPod/i.test(ua)) {
    return "unknown";
  }

  if (/Windows NT|Win64|WOW64/i.test(ua)) {
    return "windows";
  }
  // 只认平台段里的 `Macintosh` / `Mac OS X`。不认 `Safari`——见模块文档第 3 条。
  if (/Macintosh|Mac OS X/i.test(ua)) {
    return "macos";
  }
  if (/X11|Linux/i.test(ua)) {
    return "linux";
  }
  return "unknown";
}

/**
 * 读取当前宿主的平台。
 *
 * `navigator` 在 Node 侧（比如 SSR 或某些测试运行器）可能不存在，所以这里不假定它有。
 * 探测不出来时返回 `unknown`，调用方对 `unknown` 的处理与 macOS 之外的桌面一致
 * ——**除了**不做 macOS 专属的红绿灯让位。
 */
export function currentPlatform(): ChromePlatform {
  if (typeof navigator === "undefined" || typeof navigator.userAgent !== "string") {
    return "unknown";
  }
  return detectPlatform(navigator.userAgent);
}

/**
 * 该平台是否由系统自己画窗口按钮。
 *
 * macOS 上 `tauri.macos.conf.json` 保留了 `decorations: true` + `titleBarStyle: "Overlay"`，
 * 红绿灯由系统绘制。此时再画一组自绘按钮就是两套控件并存。
 */
export function hasNativeWindowButtons(platform: ChromePlatform): boolean {
  return platform === "macos";
}
