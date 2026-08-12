/**
 * 平台探测的判定表。
 *
 * 每一条都对着一个真实的错判，而错判的共同特征是**只在某一个平台上出现、且不报错**。
 * UA 字符串取自各平台 Tauri 实际使用的 WebView，不是构造的短串——短串会让
 * 「Linux 的 WebKitGTK 自报 Safari」这条最要紧的陷阱被绕过去。
 */

import { describe, expect, it } from "vitest";
import { detectPlatform, hasNativeWindowButtons } from "../platform";

const WINDOWS_WEBVIEW2 =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0";
const MACOS_WKWEBVIEW =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const LINUX_WEBKITGTK =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const ANDROID_WEBVIEW =
  "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";
const IOS_WKWEBVIEW =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";

describe("detectPlatform", () => {
  it("把三个桌面 WebView 分别判到各自的平台", () => {
    expect(detectPlatform(WINDOWS_WEBVIEW2)).toBe("windows");
    expect(detectPlatform(MACOS_WKWEBVIEW)).toBe("macos");
    expect(detectPlatform(LINUX_WEBKITGTK)).toBe("linux");
  });

  it("不把 Linux 的 WebKitGTK 当成 macOS，尽管两者的 AppleWebKit 版本号完全相同", () => {
    // 这两条 UA 都含 `AppleWebKit/605.1.15` 与 `Safari/605.1.15`。任何按
    // `Safari` / `AppleWebKit` 判 macOS 的实现都会把 Linux 判成 macOS，而症状是
    // Linux 窗口左侧空出一块 80px 的红绿灯让位区、里面什么都没有，且窗口按钮整条消失。
    expect(LINUX_WEBKITGTK).toContain("AppleWebKit/605.1.15");
    expect(MACOS_WKWEBVIEW).toContain("AppleWebKit/605.1.15");
    expect(detectPlatform(LINUX_WEBKITGTK)).not.toBe("macos");
  });

  it("先筛移动端：Android 的 UA 含 Linux，iOS 的含 Mac OS X", () => {
    // 判定顺序写错的后果就在这两条断言里：桌面判据在前会把手机认成 Linux / macOS，
    // 于是给手机渲染一条自绘标题栏。
    expect(ANDROID_WEBVIEW).toContain("Linux");
    expect(IOS_WKWEBVIEW).toContain("Mac OS X");
    expect(detectPlatform(ANDROID_WEBVIEW)).toBe("unknown");
    expect(detectPlatform(IOS_WKWEBVIEW)).toBe("unknown");
  });

  it("认不出来时返回 unknown 而不是猜一个", () => {
    expect(detectPlatform("")).toBe("unknown");
    expect(detectPlatform("SomeHeadlessAgent/1.0")).toBe("unknown");
  });
});

describe("hasNativeWindowButtons", () => {
  it("只有 macOS 由系统画窗口按钮", () => {
    // macOS 上 `tauri.macos.conf.json` 保留 decorations + Overlay，红绿灯是系统画的；
    // 再画一组自绘按钮就是两套控件并存。
    expect(hasNativeWindowButtons("macos")).toBe(true);
    expect(hasNativeWindowButtons("windows")).toBe(false);
    expect(hasNativeWindowButtons("linux")).toBe(false);
    expect(hasNativeWindowButtons("unknown")).toBe(false);
  });
});
