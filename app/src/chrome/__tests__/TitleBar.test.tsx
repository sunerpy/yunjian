/**
 * 呈现层：拖动区属性、双击最大化的缺席、指针类型分流、降级形态。
 *
 * 「双击最大化不存在」这条是行为断言而不是源码扫描：源码扫描只能证明**这个写法**没出现，
 * 行为断言能证明**这个效果**没出现，包括通过别的写法混进来的。
 */

import { StrictMode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import TitleBar from "../TitleBar";
import type { WindowChrome } from "../useWindowChrome";
import type { WindowControls } from "../windowControls";

function chromeFixture(overrides: Partial<WindowChrome> = {}): WindowChrome {
  const base: WindowChrome = {
    platform: "linux",
    controls: {} as WindowControls,
    isMaximized: false,
    showWindowButtons: true,
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
    dragForNonMousePointer: vi.fn(),
  };
  return { ...base, ...overrides };
}

function dragRegion(): HTMLElement {
  return screen.getByTestId("titlebar-drag-region");
}

describe("拖动区", () => {
  it("用 deep 而不是裸的 data-tauri-drag-region", () => {
    // 裸属性的语义是 `el === composedPath()[0]`，于是点在标题文字上不会拖动窗口。
    // 用户看到的是「标题栏有的地方能拖、有的地方不能」。
    render(<TitleBar title="云笺" chrome={chromeFixture()} />);
    expect(dragRegion().getAttribute("data-tauri-drag-region")).toBe("deep");
  });

  it("标题文字在拖动区**内部**，这才是 deep 的意义", () => {
    render(<TitleBar title="云笺" chrome={chromeFixture()} />);
    expect(dragRegion().contains(screen.getByText("云笺"))).toBe(true);
  });

  it("窗口按钮在拖动区**外部**，所以笔尖点按钮不会冒泡成拖动", () => {
    render(<TitleBar title="云笺" chrome={chromeFixture()} />);
    expect(dragRegion().contains(screen.getByLabelText("关闭"))).toBe(false);
  });
});

describe("双击最大化必须缺席", () => {
  it("在拖动区上双击不会切换最大化", () => {
    // Tauri 注入的 drag.js 已经做了这件事：
    // `const cmd = e.detail === 2 ? 'internal_toggle_maximize' : 'start_dragging'`。
    // 再挂一个 onDoubleClick 会让状态被切换两次，净效果回到原样——用户看到的是
    // 「双击标题栏没反应」，而代码里明明写了处理。这是本组件最难看出来的错。
    const chrome = chromeFixture();
    render(<TitleBar title="云笺" chrome={chrome} />);

    fireEvent.doubleClick(dragRegion());
    fireEvent.doubleClick(screen.getByText("云笺"));

    expect(chrome.toggleMaximize).not.toHaveBeenCalled();
  });

  it("源码里也没有 dblclick / onDoubleClick 的痕迹", async () => {
    // 与上一条互补：行为断言证明「效果不存在」，这条证明「写法不存在」，
    // 于是一个写对了但接了别的动作的双击处理器也会被拦下来。
    // 路径由 cwd 推出而不是 `import.meta.url`：Vite 会把后者改写成非 `file:` 的 URL，
    // `fileURLToPath` 于是抛 `The URL must be of scheme file`（已实测）。
    const { readFile } = await import("node:fs/promises");
    const { resolve } = await import("node:path");
    const source = await readFile(resolve(process.cwd(), "src/chrome/TitleBar.tsx"), "utf8");
    // 文件头的说明文字里刻意提到了这个词，所以只扫「作为 JSX 属性或事件名」的形态。
    expect(source).not.toMatch(/onDoubleClick\s*=/);
    expect(source).not.toMatch(/addEventListener\(\s*["'`]dblclick/);
  });
});

describe("指针类型分流", () => {
  it("手写笔按下时把 pointerType 交给状态层", () => {
    const chrome = chromeFixture();
    render(<TitleBar title="云笺" chrome={chrome} />);

    fireEvent.pointerDown(dragRegion(), { pointerType: "pen" });
    expect(chrome.dragForNonMousePointer).toHaveBeenCalledWith("pen");
  });

  it("鼠标按下时也交给状态层，由它决定不动作", () => {
    // 分流判断放在状态层而不是这里：呈现层只报告发生了什么，
    // 「鼠标归注入脚本」这条策略只有一处实现，不会两边各写一半。
    const chrome = chromeFixture();
    render(<TitleBar title="云笺" chrome={chrome} />);

    fireEvent.pointerDown(dragRegion(), { pointerType: "mouse" });
    expect(chrome.dragForNonMousePointer).toHaveBeenCalledWith("mouse");
  });
});

describe("窗口按钮", () => {
  it("三个按钮各自触发它自己的动作", () => {
    const chrome = chromeFixture();
    render(<TitleBar title="云笺" chrome={chrome} />);

    fireEvent.click(screen.getByLabelText("最小化"));
    fireEvent.click(screen.getByLabelText("最大化"));
    fireEvent.click(screen.getByLabelText("关闭"));

    expect(chrome.minimize).toHaveBeenCalledTimes(1);
    expect(chrome.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(chrome.close).toHaveBeenCalledTimes(1);
  });

  it("最大化之后按钮改名成还原，屏幕阅读器才不会读错", () => {
    render(<TitleBar title="云笺" chrome={chromeFixture({ isMaximized: true })} />);
    expect(screen.getByLabelText("向下还原")).toBeDefined();
    expect(screen.queryByLabelText("最大化")).toBeNull();
  });
});

describe("降级形态", () => {
  it("showWindowButtons 为 false 时渲染零个按钮，而不是三个点了没反应的按钮", () => {
    // 渲染三个死按钮比不渲染更糟：它看起来是好的。
    render(
      <TitleBar
        title="云笺"
        chrome={chromeFixture({ controls: null, showWindowButtons: false })}
      />,
    );

    expect(screen.queryAllByRole("button")).toHaveLength(0);
    // 标题栏本身仍然在，拖动区也仍然在——只是没有按钮。
    expect(dragRegion()).toBeDefined();
    expect(screen.getByText("云笺")).toBeDefined();
  });

  it("macOS 上标记平台，让样式给系统红绿灯让出左侧空间", () => {
    render(
      <TitleBar
        title="云笺"
        chrome={chromeFixture({ platform: "macos", showWindowButtons: false })}
      />,
    );
    expect(dragRegion().closest(".titlebar")?.getAttribute("data-platform")).toBe("macos");
  });
});

describe("与真实 useWindowChrome 串起来的降级路径", () => {
  it("纯浏览器里整条标题栏能渲染出来且没有按钮", async () => {
    // 这条刻意**不注入替身**，走真实的 createWindowControls —— 于是它验证的是
    // 「getCurrentWindow() 抛异常」这条真实路径，而不是「我们传了 null」。
    // 没有 try/catch 时这里不是断言失败，而是整棵树渲染失败。
    const { useWindowChrome } = await import("../useWindowChrome");

    function Harness() {
      const chrome = useWindowChrome();
      return <TitleBar title="云笺" chrome={chrome} />;
    }

    render(
      <StrictMode>
        <Harness />
      </StrictMode>,
    );

    expect(screen.getByText("云笺")).toBeDefined();
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(dragRegion().getAttribute("data-tauri-drag-region")).toBe("deep");
  });
});
