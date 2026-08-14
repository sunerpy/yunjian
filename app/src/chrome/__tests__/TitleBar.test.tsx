/**
 * 呈现层：拖动区属性、双击最大化的缺席、指针类型分流、降级形态。
 *
 * 「双击最大化不存在」这条是行为断言而不是源码扫描：源码扫描只能证明**这个写法**没出现，
 * 行为断言能证明**这个效果**没出现，包括通过别的写法混进来的。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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

  it("三个按钮都带 `title`，且与无障碍名一致", () => {
    // 窗口按钮是纯图标，鼠标用户唯一的辨认途径是原生 tooltip。
    // 「与无障碍名一致」是必需的一半：两处不同会让读屏用户与鼠标用户听到 / 看到两种说法。
    render(<TitleBar title="云笺" chrome={chromeFixture()} />);
    for (const name of ["最小化", "最大化", "关闭"]) {
      expect(screen.getByLabelText(name).getAttribute("title"), `${name} 缺 title`).toBe(name);
    }
    render(<TitleBar title="云笺" chrome={chromeFixture({ isMaximized: true })} />);
    expect(screen.getByLabelText("向下还原").getAttribute("title")).toBe("向下还原");
  });
});

describe("悬停与按下必须各有可见反馈", () => {
  /**
   * 断言写在 CSS 层而不是 DOM 层，与 `theme.test.ts`、`opMark.test.ts` 同一条理由：
   * jsdom 不做样式计算，所以样式层的事实只能靠读源码。
   *
   * 防的缺陷是「移上去 / 按下去毫无变化」——用户会以为应用卡住了。
   * 只有 `:hover` 是不够的：按下与悬停长得一样，那么「点到了没有」这一瞬就没有信号。
   */
  const css = readFileSync(resolve(process.cwd(), "src/chrome/titlebar.css"), "utf8");
  /**
   * 去掉块注释之后的样式表。
   *
   * 必需的一步：下面按 `;` 切声明，而一条规则里的解释性注释会与紧跟它的那条声明黏成
   * 同一个片段，于是那条声明被整条丢掉。第一版就是这么把 `cursor: default` 漏掉的（实测）。
   */
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, "");

  /** 取一条选择器的规则体里的声明集合。 */
  function declarationsOf(selector: string): Set<string> {
    const pattern = new RegExp(`(^|\\n)${selector.replace(/[.:\-]/g, "\\$&")}\\s*\\{([^}]*)\\}`);
    const match = pattern.exec(stripped);
    expect(match, `titlebar.css 里没有 ${selector} 这条规则`).not.toBeNull();
    return new Set(
      (match?.[2] ?? "")
        .split(";")
        .map((declaration) => declaration.replace(/\s+/g, " ").trim())
        .filter((declaration) => declaration !== "" && !declaration.startsWith("/*")),
    );
  }

  it("静息、悬停、按下三态的声明两两不同", () => {
    const rest = declarationsOf(".titlebar__button");
    const hover = declarationsOf(".titlebar__button:hover");
    const active = declarationsOf(".titlebar__button:active");
    for (const [name, set] of [
      ["hover", hover],
      ["active", active],
    ] as const) {
      expect(set.size, `${name} 是条空规则，等于没有反馈`).toBeGreaterThan(0);
    }
    // 悬停与按下都必须改背景色——那是这三个无边框图标钮唯一的面积信号。
    for (const [name, set] of [
      ["hover", hover],
      ["active", active],
    ] as const) {
      expect(
        [...set].some((declaration) => declaration.startsWith("background:")),
        `${name} 没有改背景色`,
      ).toBe(true);
    }
    // 两两不同：把 `:active` 写成与 `:hover` 相同会让这条红。
    const sameHoverActive =
      hover.size === active.size && [...hover].every((declaration) => active.has(declaration));
    expect(sameHoverActive, "按下态与悬停态完全相同，按下那一瞬没有信号").toBe(false);
    const restBackground = [...rest].find((declaration) => declaration.startsWith("background:"));
    const hoverBackground = [...hover].find((declaration) => declaration.startsWith("background:"));
    expect(restBackground).not.toBe(hoverBackground);
  });

  it("关闭键另有自己的悬停与按下态", () => {
    // 关闭键悬停是红的；若它没有自己的按下态，就会沿用中性的那一档，
    // 于是从悬停（红）切到按下（灰）看起来像是指针移开了。
    const hover = declarationsOf(".titlebar__button--close:hover");
    const active = declarationsOf(".titlebar__button--close:active");
    expect(hover.size).toBeGreaterThan(0);
    expect(active.size).toBeGreaterThan(0);
    const same = hover.size === active.size && [...hover].every((entry) => active.has(entry));
    expect(same, "关闭键的按下态与悬停态完全相同").toBe(false);
  });

  it("拖动区显式声明 `cursor`，不让它落成 I 形光标", () => {
    // 不写的话指针移到标题文字上会变成 I 形，暗示「这里能选中」，
    // 而整条标题栏刻意关掉了选中。
    expect([...declarationsOf(".titlebar__drag")]).toContain("cursor: default");
  });

  it("**按钮的光标刻意是 `default` 而不是手型，且这条例外写明了理由**", () => {
    // 这是本项目「按钮一律手型光标」的唯一例外：各平台的窗口按钮都保持箭头。
    // 断言里连带要求源码写着理由——否则下一个人会把它「修」成 pointer。
    expect([...declarationsOf(".titlebar__button")]).toContain("cursor: default");
    expect(css).toContain("唯一例外");
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
