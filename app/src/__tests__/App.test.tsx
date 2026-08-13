/**
 * 外壳导航：从初始视图能真的走到设置页。
 *
 * # 这一组存在的理由是一次真实的交付缺口
 *
 * 设置界面（todo 62）的 14 个文件全部实现完、37 条断言全绿，**但页面上没有任何入口能到它**——
 * `SettingsScreen` 没有挂进 `App.tsx`，所以浏览器里的 a11y 快照只有顶栏加检索。
 * 组件级测试全绿而功能不可达，是因为那些测试都直接 `render(<SettingsScreen />)`，
 * 绕过了「用户怎么到这里」这一段。
 *
 * 所以这一组刻意**从 `<App />` 开始渲染**，只用用户看得见的东西导航（点按钮），
 * 不直接挂子屏。这是唯一能拦住「组件做好了但接不上」的测法。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "../App";

/**
 * 渲染 `<App />` 并等到检索屏挂载时那次 `listTags` 落地。
 *
 * 不等的话，纯同步的测试会在那个 promise resolve 之前结束，React 抛一条
 * 「update ... was not wrapped in act(...)」。它不影响断言，但会淹没将来真正的 act 警告。
 */
async function renderApp(): Promise<void> {
  render(<App />);
  await waitFor(() => {
    expect(screen.getByRole("option", { name: /思乡/ })).toBeTruthy();
  });
}

describe("外壳导航", () => {
  it("初始视图是检索，且导航条上有设置入口", async () => {
    await renderApp();
    expect(screen.getByTestId("app-nav")).toBeTruthy();
    // 入口必须可见且可点——一个存在但 disabled 的按钮同样到不了设置页。
    const entry = screen.getByTestId("nav-settings") as HTMLButtonElement;
    expect(entry.disabled).toBe(false);
    expect(entry.textContent).toBe("设置");
    // 初始态：检索屏在，设置屏不在。
    expect(screen.queryByTestId("settings-screen")).toBeNull();
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBe("page");
    expect(entry.getAttribute("aria-current")).toBeNull();
  });

  it("点设置入口后设置页出现，且带上密钥存储指示条", async () => {
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));

    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    // 标志性元素：不只是容器在，本 todo 的核心那一块也真的渲染出来了。
    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator")).toBeTruthy();
    });
    // 首启是空柜，所以指示条说「尚未存储」——这是真实的首次运行态，不是缺陷。
    expect(screen.getByTestId("key-storage-indicator").textContent).toContain("尚未存储");
    // 选中态跟着换。
    expect(screen.getByTestId("nav-settings").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBeNull();
  });

  it("走完导航再保存密钥，诚实链在端到端路径上依然成立", async () => {
    // 这一条是整条链的端到端确认：不直接挂 `SettingsScreen`，而是从 `<App />` 出发，
    // 点导航、输密钥、按保存，最后看界面说了什么。
    // 样例宿主默认降级到 keyutils，所以正确答案是「重启后失效」而不是「系统钥匙串」。
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));
    const input = (await screen.findByTestId("api-key-input")) as HTMLInputElement;

    fireEvent.change(input, { target: { value: "sk-NAV-E2E-KEY" } });
    fireEvent.click(screen.getByTestId("save-key"));

    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain(
        "系统密钥环（重启后失效）",
      );
    });
    const indicator = screen.getByTestId("key-storage-indicator").textContent ?? "";
    expect(indicator).not.toContain("持久");
    expect(indicator).not.toContain("系统钥匙串");
    // 顺带确认「保存后不回显」在真实路径上也成立。
    expect((screen.getByTestId("api-key-input") as HTMLInputElement).value).toBe("");
    expect(document.body.outerHTML).not.toContain("sk-NAV-E2E-KEY");
  });

  it("设置页上四块面板都在", async () => {
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    for (const label of ["AI 服务商与密钥", "语料库", "语音模型", "赏析缓存"]) {
      expect(screen.getByLabelText(label), `缺少「${label}」面板`).toBeTruthy();
    }
  });

  it("能从设置页回到检索", async () => {
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("nav-search"));
    await waitFor(() => {
      expect(screen.queryByTestId("settings-screen")).toBeNull();
    });
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBe("page");
  });

  it("导航条在两个视图下都在，不会切走就消失", async () => {
    await renderApp();
    expect(screen.getByTestId("app-nav")).toBeTruthy();
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    expect(screen.getByTestId("app-nav")).toBeTruthy();
  });
});
