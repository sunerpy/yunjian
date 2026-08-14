/**
 * 侧栏：折叠形态、选中态与无障碍语义。
 *
 * # 三条断言各自对着一个「写错了不报错」的缺陷
 *
 * 1. **折叠时标签必须仍在 DOM 里。** 用 `{collapsed ? null : <span>…</span>}` 折叠会让
 *    三个导航项变成无名图标钮，读屏用户听到的是「按钮」「按钮」「按钮」。
 *    正确做法是 `sr-only`：移出视觉流、留在无障碍树里。
 *    这条缺陷在截图上**看不出来**（视觉结果一样），所以只能靠断言。
 * 2. **设置按钮不能带 `aria-current="page"`。** 它是弹窗触发器而不是一屏；写成页面语义会让
 *    弹窗打开时同时存在两个「当前页」。
 * 3. **两种形态下几何量之外的东西一律不变。** 折叠只该改宽度与标签的可见性；
 *    若顺手把 testid 或无障碍名也改了，`App.test.tsx` 那一组从 `<App />` 出发的断言
 *    会在折叠态下失效——而默认是展开态，于是那个缺陷只在用户折叠之后出现。
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Sidebar, { type ShellSection } from "../Sidebar";

interface Overrides {
  section?: ShellSection;
  collapsed?: boolean;
  settingsOpen?: boolean;
  onToggleCollapsed?: () => void;
  onSelect?: (section: ShellSection) => void;
  onOpenSettings?: () => void;
}

function renderSidebar(overrides: Overrides = {}) {
  const props = {
    section: overrides.section ?? ("search" as ShellSection),
    collapsed: overrides.collapsed ?? false,
    settingsOpen: overrides.settingsOpen ?? false,
    onToggleCollapsed: overrides.onToggleCollapsed ?? vi.fn(),
    onSelect: overrides.onSelect ?? vi.fn(),
    onOpenSettings: overrides.onOpenSettings ?? vi.fn(),
  };
  return { ...render(<Sidebar {...props} />), props };
}

describe("导航入口", () => {
  it("三个入口都在，testid 与改造前逐字相同", () => {
    // 这四个 testid 是 `App.test.tsx` 那一组端到端断言的抓手，
    // 而那一组存在的理由是一次真实缺口（设置全部实现完但页面上没有入口能到它）。
    // 换掉任何一个等于把那道防线拆了。
    renderSidebar();
    expect(screen.getByTestId("app-nav")).toBeTruthy();
    for (const [testId, label] of [
      ["nav-search", "检索"],
      ["nav-recite", "背诵"],
      ["nav-settings", "设置"],
    ] as const) {
      const entry = screen.getByTestId(testId) as HTMLButtonElement;
      expect(entry.disabled, `${testId} 被禁用了，等于到不了`).toBe(false);
      expect(entry.textContent).toBe(label);
    }
  });

  it("点检索与背诵各自回报自己那一支", () => {
    const onSelect = vi.fn();
    renderSidebar({ onSelect });
    fireEvent.click(screen.getByTestId("nav-recite"));
    expect(onSelect).toHaveBeenLastCalledWith("recite");
    fireEvent.click(screen.getByTestId("nav-search"));
    expect(onSelect).toHaveBeenLastCalledWith("search");
  });

  it("点设置走的是「打开弹窗」而不是「切屏」", () => {
    const onSelect = vi.fn();
    const onOpenSettings = vi.fn();
    renderSidebar({ onSelect, onOpenSettings });
    fireEvent.click(screen.getByTestId("nav-settings"));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    // 关键在这一句：设置不得经由切屏的回调。走那条路会把底下那一屏卸载掉，
    // 于是「关掉弹窗回到原处」就实现不了。
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("选中态", () => {
  it("检索被选中时只有它是当前页", () => {
    renderSidebar({ section: "search" });
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-recite").getAttribute("aria-current")).toBeNull();
  });

  it("背诵被选中时只有它是当前页", () => {
    renderSidebar({ section: "recite" });
    expect(screen.getByTestId("nav-recite").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBeNull();
  });

  it("整份渲染里当前页有且只有一个", () => {
    // 这一条比上面两条更强：它拦得住「选中态判定写成了常量」这类改法——
    // 把 `section === "search"` 换成 `true` 会让上面第一条照样绿，这一条会红。
    for (const section of ["search", "recite"] as const) {
      const view = renderSidebar({ section });
      expect(view.container.querySelectorAll('[aria-current="page"]')).toHaveLength(1);
      view.unmount();
    }
  });

  it("**视觉选中与语义选中必须是同一项**", () => {
    // 这一条补的是一个真实的空洞：上面几条只看 `aria-current`，而那个属性与
    // className 是**两处独立写的**。把 `itemClass(section === "search")` 改成
    // `itemClass(true)` 之后，检索项会永远画成选中样式，而 `aria-current` 照旧正确
    // ——注入实测：40 条断言全绿。用户看到的是「高亮在检索、内容却是背诵」。
    //
    // 判据取 className 里那个只出现在选中分支的记号（左侧标尺的强调色）。
    // 断言的是**两个集合相等**，所以多标一个与少标一个都会红。
    const ACTIVE_MARK = "border-[var(--color-accent)]";
    for (const section of ["search", "recite"] as const) {
      const view = renderSidebar({ section });
      const items = ["nav-search", "nav-recite", "nav-settings"].map((testId) =>
        screen.getByTestId(testId),
      );
      const visually = items
        .filter((item) => item.className.includes(ACTIVE_MARK))
        .map((item) => item.dataset.testid);
      const semantically = items
        .filter((item) => item.getAttribute("aria-current") === "page")
        .map((item) => item.dataset.testid);
      expect(visually, `section=${section} 时视觉选中与语义选中不是同一项`).toEqual(semantically);
      expect(visually).toHaveLength(1);
      view.unmount();
    }
  });

  it("设置弹窗开着时，视觉选中跟着设置项走", () => {
    // 设置项没有 `aria-current`（它是弹窗触发器），所以它的视觉选中要与 `aria-expanded`
    // 对齐；而此时检索项**仍然**是当前页，于是这一刻有两项带选中样式——那是对的，
    // 一项表示「当前在哪屏」，一项表示「弹窗开着」。
    const ACTIVE_MARK = "border-[var(--color-accent)]";
    renderSidebar({ section: "search", settingsOpen: true });
    expect(screen.getByTestId("nav-settings").className).toContain(ACTIVE_MARK);
    expect(screen.getByTestId("nav-search").className).toContain(ACTIVE_MARK);
    expect(screen.getByTestId("nav-recite").className).not.toContain(ACTIVE_MARK);
  });

  it("**设置按钮用弹窗语义，永不带 `aria-current`**", () => {
    for (const settingsOpen of [false, true]) {
      const view = renderSidebar({ settingsOpen });
      const entry = screen.getByTestId("nav-settings");
      expect(entry.getAttribute("aria-haspopup")).toBe("dialog");
      expect(entry.getAttribute("aria-expanded")).toBe(String(settingsOpen));
      expect(entry.getAttribute("aria-current"), "设置是弹窗触发器，不是一屏").toBeNull();
      // 弹窗开着时底下那一屏仍然是当前页，所以当前页的数目不因弹窗而变。
      expect(view.container.querySelectorAll('[aria-current="page"]')).toHaveLength(1);
      view.unmount();
    }
  });
});

describe("折叠", () => {
  it("折叠开关回报自己的展开状态，且有无障碍名", () => {
    const onToggleCollapsed = vi.fn();
    const view = renderSidebar({ collapsed: false, onToggleCollapsed });
    const toggle = screen.getByTestId("sidebar-toggle");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(toggle.getAttribute("aria-label")).toBe("折叠侧栏");
    fireEvent.click(toggle);
    expect(onToggleCollapsed).toHaveBeenCalledTimes(1);
    view.unmount();

    renderSidebar({ collapsed: true });
    const collapsedToggle = screen.getByTestId("sidebar-toggle");
    expect(collapsedToggle.getAttribute("aria-expanded")).toBe("false");
    expect(collapsedToggle.getAttribute("aria-label")).toBe("展开侧栏");
  });

  it("折叠状态标在容器上，供样式与目视 QA 定位", () => {
    const view = renderSidebar({ collapsed: false });
    expect(screen.getByTestId("app-nav").dataset.collapsed).toBe("false");
    view.unmount();
    renderSidebar({ collapsed: true });
    expect(screen.getByTestId("app-nav").dataset.collapsed).toBe("true");
  });

  it("**折叠后标签仍在 DOM 里，只是视觉隐藏**", () => {
    // 见文件头第 1 条：换成条件渲染会让三个入口变成无名图标钮，而截图上看不出区别。
    renderSidebar({ collapsed: true });
    for (const [testId, label] of [
      ["nav-search", "检索"],
      ["nav-recite", "背诵"],
      ["nav-settings", "设置"],
    ] as const) {
      const entry = screen.getByTestId(testId);
      expect(entry.textContent, `${testId} 折叠后没了无障碍名`).toBe(label);
      // `sr-only` 是移出视觉流的手段。类名在这里是被断言的实现细节，
      // 因为「视觉隐藏但仍可读」在 jsdom 里没有别的可观测信号（jsdom 不算样式）。
      const span = entry.querySelector("span");
      expect(span?.className).toContain("sr-only");
    }
  });

  it("展开时标签不带 `sr-only`", () => {
    // 反向对照：少了它，「折叠时是 sr-only」可以靠「永远是 sr-only」满足，
    // 而那意味着展开态下标签也看不见。
    renderSidebar({ collapsed: false });
    for (const testId of ["nav-search", "nav-recite", "nav-settings"]) {
      expect(screen.getByTestId(testId).querySelector("span")?.className).not.toContain("sr-only");
    }
  });

  it("折叠后靠 `title` 给出悬停提示", () => {
    // 折叠态下文字看不见，鼠标用户唯一的辨认途径就是原生 tooltip。
    renderSidebar({ collapsed: true });
    expect(screen.getByTestId("nav-search").getAttribute("title")).toBe("检索");
    expect(screen.getByTestId("nav-recite").getAttribute("title")).toBe("背诵");
    expect(screen.getByTestId("nav-settings").getAttribute("title")).toBe("设置");
  });

  it("展开后不带 `title`：文字已经在那里，再弹一个 tooltip 是重复", () => {
    renderSidebar({ collapsed: false });
    expect(screen.getByTestId("nav-search").getAttribute("title")).toBeNull();
  });
});
