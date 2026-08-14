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
    // 设置改成弹窗之后，它的状态是「展开 / 收起」而不是「是不是当前页」。
    // 原先这里验 `aria-current` 为 null；现在验 `aria-expanded` 为 "false"，
    // 两者验的是同一件事——此刻还没进设置——但后者同时钉住了「弹窗语义没被写成页面语义」。
    expect(entry.getAttribute("aria-current")).toBeNull();
    expect(entry.getAttribute("aria-expanded")).toBe("false");
    expect(entry.getAttribute("aria-haspopup")).toBe("dialog");
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
    // 原先这里验「设置成了当前页、检索不再是当前页」。设置改成弹窗之后那句话本身不再成立：
    // 弹窗打开时底下那一屏**仍然是当前页**，给设置也写上 `aria-current="page"` 会让读屏
    // 用户同时听到两个「当前页」。所以改成验：设置按钮报告自己展开了，
    // 而检索**仍然**是当前页——这一条比原来那条更强，它顺带钉住了「底下那一屏没有被切走」，
    // 而那正是「关掉弹窗要回到原处」的前提。
    expect(screen.getByTestId("nav-settings").getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("nav-settings").getAttribute("aria-current")).toBeNull();
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBe("page");
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

  it("关掉设置弹窗后回到原来那一屏", async () => {
    // 原先这一条叫「能从设置页回到检索」，做法是点检索那个导航项。设置改成弹窗之后
    // 那个做法验不到要验的东西了：检索本来就没离开过，点它是个空操作。
    // 现在改成点弹窗自己的关闭按钮，并确认关掉之后检索屏还在、设置屏没了、
    // 设置按钮回报收起。这一条与原条目的意图相同（用户能离开设置回到内容），
    // 但走的是弹窗真正的退出路径。
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("settings-dialog-close"));
    await waitFor(() => {
      expect(screen.queryByTestId("settings-screen")).toBeNull();
    });
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-settings").getAttribute("aria-expanded")).toBe("false");
    // 检索屏没有被卸载重挂：标签选项还在，说明它一直挂着。
    expect(screen.getByRole("option", { name: /思乡/ })).toBeTruthy();
  });

  it("**按 Esc 也能关掉设置弹窗**", async () => {
    // jsdom 没有实现 `HTMLDialogElement` 的方法（`showModal`/`close` 都是 undefined，
    // 已实测），所以原生的 Esc 行为在这里根本不存在。这条断言验的是
    // `SettingsDialog` 自己实现的那一份——也正因为原生那份在测试里不可见，
    // 自己实现的这一份才是唯一能被钉住的。
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    fireEvent.keyDown(screen.getByTestId("settings-dialog"), { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByTestId("settings-screen")).toBeNull();
    });
  });

  it("**`?sample-key-tier=plaintext` 走完真实导航后必须显示「明文配置文件」**", async () => {
    // 这一条盯的是一个只有真浏览器发现的缺陷：档位参数解析正确、报告构造正确、
    // 指示串推导正确，但面板首屏查的是 `keyStatus(aiSettings.provider)`，
    // 而预置密钥当时写在另一个 account 名下——于是页面显示「尚未存储」。
    // 档位断言全绿是因为它们硬写了 provider，绕过了这一段。
    // 因此这里从 `<App />` 出发，只看用户看得到的东西。
    window.history.replaceState({}, "", "/index.html?sample-key-tier=plaintext");
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-settings"));

    await waitFor(() => {
      expect(screen.getByTestId("key-storage-indicator").textContent).toContain("明文配置文件");
    });
    // 有密钥，所以告警用陈述语气。
    const warning = screen.getByTestId("plaintext-warning");
    expect(warning.getAttribute("data-mood")).toBe("actual");
    expect(warning.textContent).toContain("密钥以明文保存在");
    // 且整屏不出现「尚未保存任何密钥」——那正是缺陷时的实际文案。
    expect(screen.getByTestId("settings-screen").textContent).not.toContain("尚未保存任何密钥");
    window.history.replaceState({}, "", "/index.html");
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

describe("外壳导航到内置字典", () => {
  it("从侧栏进入字典并完成双字逐字查询", async () => {
    await renderApp();
    const entry = screen.getByTestId("nav-dictionary") as HTMLButtonElement;
    expect(entry.disabled).toBe(false);
    fireEvent.click(entry);

    expect(await screen.findByTestId("dictionary-panel")).toBeTruthy();
    expect(entry.getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBeNull();
    fireEvent.click(screen.getByTestId("dictionary-submit"));
    await waitFor(() => {
      expect(screen.getByTestId("dictionary-character-斜")).toBeTruthy();
      expect(screen.getByTestId("dictionary-character-阳")).toBeTruthy();
    });
  });
});

/**
 * 背诵界面（todo 63）的入口。
 *
 * 与上面那一组同一条理由：`recite/__tests__/` 下的面板级断言全部直接挂子组件，
 * 绕过了「用户怎么走到这里」。这一组从 `<App />` 出发，只点用户看得见的按钮。
 */
describe("外壳导航到背诵界面", () => {
  it("导航条上有背诵入口且可点", async () => {
    await renderApp();
    const entry = screen.getByTestId("nav-recite") as HTMLButtonElement;
    expect(entry.disabled).toBe(false);
    expect(entry.textContent).toBe("背诵");
    expect(screen.queryByTestId("recite-screen")).toBeNull();
    expect(entry.getAttribute("aria-current")).toBeNull();
  });

  it("**点背诵入口后背诵屏出现，四种形态与复习队列都在**", async () => {
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-recite"));

    await waitFor(() => {
      expect(screen.getByTestId("recite-screen")).toBeTruthy();
    });
    // 四种形态一个不漏，包括此刻走不通的语音——它必须能被请求到并得到明确回答。
    for (const mode of ["cloze", "first-char", "masked", "voice"]) {
      expect(screen.getByTestId(`mode-${mode}`), `缺形态 ${mode}`).toBeTruthy();
    }
    // 复习队列由 FSRS 排程驱动，一进屏就该拉到样例排程。
    await waitFor(() => {
      expect(screen.getByTestId("review-queue")).toBeTruthy();
    });
    expect(screen.getByTestId("nav-recite").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBeNull();
  });

  it("**从 `<App />` 走到详情页，拼音与平仄两个开关都在且各自独立**", async () => {
    // 与本文件存在的理由同一条：注音层的档位断言全在组件级，唯有这一条能拦住
    // 「注音层做好了但详情页没接上开关」。所以只点用户能看见的东西。
    await renderApp();
    // 检索屏初始没有结果，得先真的查一次——这也正是用户到详情页的唯一路径。
    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "明月" } });
    fireEvent.click(screen.getByTestId("search-submit"));
    const rows = await waitFor(() => {
      const found = screen.getAllByTestId("result-row");
      expect(found.length).toBeGreaterThan(0);
      return found;
    });
    const open = rows[0]?.querySelector("button");
    expect(open, "检索结果行里没有可点的打开按钮").toBeTruthy();
    fireEvent.click(open as HTMLButtonElement);

    await waitFor(() => {
      expect(screen.getByTestId("poem-detail")).toBeTruthy();
    });

    const pinyin = screen.getByTestId("pinyin-toggle") as HTMLInputElement;
    const tones = screen.getByTestId("tone-toggle") as HTMLInputElement;
    expect(pinyin.checked).toBe(false);
    expect(tones.checked).toBe(false);

    fireEvent.click(pinyin);
    await waitFor(() => {
      expect(screen.getByTestId("poem-body").dataset.pinyin).toBe("on");
    });
    // 开拼音不得把平仄一起点亮，这条在真实页面上再确认一次。
    expect((screen.getByTestId("tone-toggle") as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByTestId("tone-row")).toBeNull();

    // 样例注音里四档齐全，所以真实路径上四种形态都应该看得到。
    expect(screen.getAllByTitle("暂无注音").length).toBeGreaterThan(0);
    expect(screen.getAllByTestId("uncertain-mark").length).toBeGreaterThan(0);
    expect(screen.getAllByTitle("通用拼音，不是古典语境裁决").length).toBeGreaterThan(0);
    expect(screen.getAllByTitle(/有据破读：/).length).toBeGreaterThan(0);
    expect(screen.getByTestId("pinyin-coverage").textContent).toContain("名册之外");
  });

  it("**从 `<App />` 走完一整局：出题、作答、五类标记、发音注记、落账**", async () => {
    // 这一条是整条链的端到端确认，也是唯一能拦住「面板做好了但接不上」的测法。
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-recite"));
    await waitFor(() => {
      expect(screen.getByTestId("start-session")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("start-session"));
    await waitFor(() => {
      expect(screen.getByTestId("session-prompt")).toBeTruthy();
    });
    // 样例提示里有内核填的全角下划线，不是 CSS 画的。
    expect(screen.getByTestId("session-prompt").textContent).toContain("＿");

    fireEvent.change(screen.getByTestId("recite-answer"), {
      target: { value: "床前明月光，疑是地上霜。举头望明月，低头思故乡。" },
    });
    fireEvent.click(screen.getByTestId("submit-answer"));

    await waitFor(() => {
      expect(screen.getByTestId("op-feedback")).toBeTruthy();
    });
    // 五类差异在真实路径上也各自渲染出不同的标记。
    const kinds = [
      ...screen.getByTestId("op-feedback").querySelectorAll<HTMLElement>("[data-op]"),
    ].map((cell) => cell.dataset.op);
    for (const kind of [
      "deletion",
      "insertion",
      "re_recitation",
      "substitution",
      "near_homophone_substitution",
    ]) {
      expect(kinds, `样例载荷里缺 ${kind}`).toContain(kind);
    }
    expect(screen.getByTestId("pronunciation-boundary").textContent).toContain("不评估发音标准度");

    fireEvent.click(screen.getByTestId("grade-good"));
    fireEvent.click(screen.getByTestId("commit-grade"));
    await waitFor(() => {
      expect(screen.getByTestId("commit-facts")).toBeTruthy();
    });
    expect(screen.getByTestId("commit-grade-label").textContent).toBe("良好");
  });

  it("**能从背诵页回到检索；在背诵页开设置，关掉后仍在背诵页**", async () => {
    // 前半段与原条目相同。后半段那句断言原先是
    // `expect(screen.queryByTestId("recite-screen")).toBeNull()`——因为设置当时是第三屏，
    // 进设置就意味着离开背诵。设置改成弹窗之后那句话反过来了：**背诵屏必须还在**，
    // 否则「关掉弹窗回到原处」就实现不了（回不到一个已经被卸载的屏）。
    // 所以这里把它改成正向断言，并补上关掉之后仍在背诵页——那才是这条改动的用户可见收益。
    await renderApp();
    fireEvent.click(screen.getByTestId("nav-recite"));
    await waitFor(() => {
      expect(screen.getByTestId("recite-screen")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("nav-search"));
    await waitFor(() => {
      expect(screen.queryByTestId("recite-screen")).toBeNull();
    });
    fireEvent.click(screen.getByTestId("nav-recite"));
    await waitFor(() => {
      expect(screen.getByTestId("recite-screen")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("nav-settings"));
    await waitFor(() => {
      expect(screen.getByTestId("settings-screen")).toBeTruthy();
    });
    expect(screen.getByTestId("recite-screen")).toBeTruthy();
    expect(screen.getByTestId("nav-recite").getAttribute("aria-current")).toBe("page");

    fireEvent.click(screen.getByTestId("settings-dialog-close"));
    await waitFor(() => {
      expect(screen.queryByTestId("settings-screen")).toBeNull();
    });
    expect(screen.getByTestId("recite-screen")).toBeTruthy();
    expect(screen.getByTestId("nav-recite").getAttribute("aria-current")).toBe("page");
    expect(screen.getByTestId("nav-search").getAttribute("aria-current")).toBeNull();
  });
});
