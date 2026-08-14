/**
 * 设置弹窗：焦点陷阱、Esc、遮罩关闭、焦点归还，以及「关着的时候不挂载内容」。
 *
 * # 为什么这四条行为要自己实现、也要自己测
 *
 * jsdom 30 **没有实现 `HTMLDialogElement` 的任何方法**：`showModal`、`show`、`close`
 * 三个都是 `undefined`（本机实测，只有 `open` 属性的反射是好的）。
 * 于是原生 dialog 提供的 Esc、焦点陷阱、遮罩点击在测试里**一条都观测不到**。
 *
 * 这不是「jsdom 不够好所以只能凑合」，而是一个真实的取舍：这四条正是这个弹窗能不能用的
 * 全部内容，把它们交给原生就等于交给一份在 CI 里完全不可验证的实现。所以壳子用原生元素
 * （为了顶层与 `::backdrop`，那两样自己搭不出来），行为全部显式实现并逐条钉住。
 *
 * 代价是真实浏览器里焦点陷阱有两份（原生一份、我们一份）。它们不冲突：原生把焦点困在弹窗内，
 * 我们这一份只决定困住之后的下一站是谁。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createSampleSettingsPorts } from "../../data/sampleSettingsPorts";
import SettingsDialog from "../SettingsDialog";

function renderDialog(open: boolean, onClose: () => void = vi.fn()) {
  return render(
    <SettingsDialog open={open} onClose={onClose} ports={createSampleSettingsPorts()} />,
  );
}

describe("开合", () => {
  it("**关着的时候一块面板都不挂载**", () => {
    // 四块面板的 effect 一挂载就发查询（语料库、语音模型、缓存统计），
    // 真实路径上要开 SQLite。关着也挂上意味着那三条查询在应用启动时就跑，
    // 而用户还没打开过设置。
    renderDialog(false);
    expect(screen.getByTestId("settings-dialog")).toBeTruthy();
    expect(screen.queryByTestId("settings-screen")).toBeNull();
    expect(screen.queryByTestId("key-storage-indicator")).toBeNull();
  });

  it("打开后四块面板与密钥存储指示条都在", async () => {
    renderDialog(true);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    for (const label of ["AI 服务商与密钥", "语料库", "语音模型", "赏析缓存"]) {
      expect(screen.getByLabelText(label), `缺少「${label}」面板`).toBeTruthy();
    }
    expect(screen.getByTestId("key-storage-indicator")).toBeTruthy();
  });

  it("弹窗自己带无障碍名，读屏能说出这是哪个对话框", () => {
    renderDialog(true);
    expect(screen.getByTestId("settings-dialog").getAttribute("aria-label")).toBe("设置");
  });

  it("**「设置」这个标题只出现一次，且在滚动区之外**", async () => {
    // 目视 QA 抓到的缺陷：头部条只放了个关闭按钮，而 `SettingsScreen` 自己又渲染一个
    // `<h1>设置</h1>`，于是弹窗顶部是一条空条紧跟一个重复标题，中间白空约 70px。
    // 修法是标题移到头部条、`SettingsScreen` 收到 `showTitle={false}`。
    // 这条同时拦住两种回退：把 `h1` 放回来（变成两个），或把头部条的标题删掉（变成零个）。
    renderDialog(true);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    const headings = [
      ...screen
        .getByTestId("settings-dialog")
        .querySelectorAll<HTMLElement>("h1, h2, h3, h4, h5, h6"),
    ].filter((heading) => heading.textContent === "设置");
    expect(headings, "弹窗里「设置」这个标题的数目不是 1").toHaveLength(1);
    // 且它在滚动容器**外面**：在里面的话往下滚就看不到自己在哪个弹窗里了。
    const scroller = screen.getByTestId("settings-screen").parentElement;
    expect(scroller?.contains(headings[0] ?? null)).toBe(false);
  });

  it("`open` 属性跟着 prop 走", () => {
    // jsdom 里 `showModal()` 不存在，组件退回写 `open` 属性；
    // 这条同时确认那条降级分支真的执行了——否则弹窗内容虽然渲染了，
    // 元素本身却是关着的，真实浏览器里就是一块看不见的东西。
    const view = renderDialog(false);
    expect(screen.getByTestId("settings-dialog").hasAttribute("open")).toBe(false);
    view.unmount();
    renderDialog(true);
    expect(screen.getByTestId("settings-dialog").hasAttribute("open")).toBe(true);
  });
});

describe("关闭途径", () => {
  it("关闭按钮", async () => {
    const onClose = vi.fn();
    renderDialog(true, onClose);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("settings-dialog-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Esc", () => {
    const onClose = vi.fn();
    renderDialog(true, onClose);
    fireEvent.keyDown(screen.getByTestId("settings-dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("原生 cancel 事件也走同一条关闭路径", () => {
    // 真实浏览器里 Esc 会派发 `cancel`。两条路径都收进 `onClose`，
    // 是为了让关闭只有一个出口——否则元素自己先关掉、React 的 `open` 再变 false，
    // 两者的顺序不可控。
    const onClose = vi.fn();
    renderDialog(true, onClose);
    fireEvent(screen.getByTestId("settings-dialog"), new Event("cancel", { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("点遮罩关闭；点内容不关闭", async () => {
    const onClose = vi.fn();
    renderDialog(true, onClose);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });

    // 点在内容上：事件目标不是 `<dialog>` 自己，不该关。
    // 这一半是必需的——只验「点遮罩会关」的话，一个无条件 `onClose` 的实现照样绿，
    // 而那意味着用户点面板里任何空白处弹窗就消失。
    fireEvent.click(screen.getByTestId("settings-screen"));
    expect(onClose).not.toHaveBeenCalled();

    // 点在 `<dialog>` 自己身上：那就是遮罩区域（内容都在里层的 div 里）。
    fireEvent.click(screen.getByTestId("settings-dialog"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("键盘处理器挂在弹窗元素上，不是挂在 document 上", () => {
    // 挂在 document 上的实现会让 Esc 在整个应用里都触发关闭设置，
    // 而背诵界面的作答输入框需要 Esc 做别的事。
    // 验法：把事件派发到弹窗**外面**的元素上，关闭不该被触发。
    const outside = document.createElement("input");
    document.body.append(outside);
    const onClose = vi.fn();
    renderDialog(true, onClose);
    fireEvent.keyDown(outside, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    outside.remove();
  });

  it("别的按键不关闭", () => {
    const onClose = vi.fn();
    renderDialog(true, onClose);
    for (const key of ["Enter", "a", " ", "ArrowDown"]) {
      fireEvent.keyDown(screen.getByTestId("settings-dialog"), { key });
    }
    expect(onClose).not.toHaveBeenCalled();
  });
});

describe("焦点", () => {
  it("打开后焦点落进弹窗，而不是留在外面", async () => {
    // 留在外面的症状是：按 Tab 从弹窗后面的元素开始走，焦点陷阱第一圈是空的。
    renderDialog(true);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    expect(document.activeElement).toBe(screen.getByTestId("settings-dialog-close"));
  });

  it("关闭后焦点归还给打开它的那个元素", async () => {
    const trigger = document.createElement("button");
    trigger.textContent = "设置";
    document.body.append(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const { rerender } = render(
      <SettingsDialog open onClose={vi.fn()} ports={createSampleSettingsPorts()} />,
    );
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    expect(document.activeElement).not.toBe(trigger);

    rerender(<SettingsDialog open={false} onClose={vi.fn()} ports={createSampleSettingsPorts()} />);
    // 不归还的症状是焦点掉到 `<body>`，于是键盘用户按 Tab 会从页面开头重新走一遍。
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it("**Tab 在弹窗内循环，走不出去**", async () => {
    renderDialog(true);
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    const dialog = screen.getByTestId("settings-dialog");
    const focusable = [
      ...dialog.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ];
    // 设置面板里有输入框、下拉框与若干按钮，所以这个集合必须是多元的——
    // 只有一个元素时「循环」这件事无从验证。
    expect(focusable.length).toBeGreaterThan(3);
    const first = focusable[0];
    const last = focusable.at(-1);
    expect(first).toBeDefined();
    expect(last).toBeDefined();

    // 在最后一个上按 Tab -> 回到第一个。
    last?.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(first);

    // 在第一个上按 Shift+Tab -> 跳到最后一个。
    first?.focus();
    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("中间位置按 Tab 不被劫持，交回浏览器的默认顺序", () => {
    // 反向对照：无条件 `preventDefault` + `first.focus()` 会让上面那条绿，
    // 但用户在弹窗里根本无法用 Tab 逐个走——每按一次都跳回第一个。
    renderDialog(true);
    const dialog = screen.getByTestId("settings-dialog");
    const focusable = [...dialog.querySelectorAll<HTMLElement>("button, input, select")];
    const middle = focusable[1];
    expect(middle).toBeDefined();
    middle?.focus();
    const event = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    dialog.dispatchEvent(event);
    expect(event.defaultPrevented, "中间位置的 Tab 被劫持了").toBe(false);
    expect(document.activeElement).toBe(middle);
  });
});
