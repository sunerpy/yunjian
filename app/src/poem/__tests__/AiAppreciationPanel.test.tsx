/**
 * AI 赏析面板：标注义务与视觉区分。
 *
 * 这一组是本 todo 的核心验收。每条断言都对着一条**会被后来的重构悄悄抹掉**的东西：
 * 少两个字的标签、被删掉的未审校提示、被「统一样式」合并掉的容器类。
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AppreciationState } from "../../contracts/ai";
import { AI_PANEL_LABEL, AI_UNREVIEWED_BADGE, AI_UNREVIEWED_DISCLOSURE } from "../../contracts/ai";
import AiAppreciationPanel from "../AiAppreciationPanel";

const READY: AppreciationState = {
  kind: "ready",
  view: {
    text: "首句写景，次句转入乡思。",
    model: "qwen2.5-7b-instruct",
    template_version: "1.2.0",
    source: "generated",
  },
};

describe("标签", () => {
  it("是「AI 赏析」而不是「赏析」", () => {
    // 少掉「AI」两个字，这块内容就变成了看起来像考据成果的无出处文字，
    // 而整个产品的许可立场正是靠这个标签成立的。
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("ai-panel-label").textContent).toBe("AI 赏析");
  });

  it("常量本身就是「AI 赏析」，防止有人只改常量不改组件", () => {
    expect(AI_PANEL_LABEL).toBe("AI 赏析");
  });

  it("面板的无障碍名也是「AI 赏析」，不看屏幕的用户同样拿到这条标注", () => {
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByLabelText("AI 赏析").getAttribute("data-testid")).toBe("ai-panel");
  });
});

describe("未审校提示不可被删", () => {
  it("短标签渲染出来", () => {
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("ai-unreviewed-badge").textContent).toBe(AI_UNREVIEWED_BADGE);
  });

  it("短标签里含「未经人工审校」，用词与 docs/AI.zh.md 一致", () => {
    // 文档写的是「未经人工审校」（docs/AI.zh.md:272），
    // 既有常量 AI_UNREVIEWED_DISCLOSURE 也是这个用词。界面上不能出现第二种说法。
    expect(AI_UNREVIEWED_BADGE).toContain("未经人工审校");
  });

  it("完整披露逐字等于 Rust 侧的 AI_UNREVIEWED_DISCLOSURE", () => {
    // 逐字取自 `crates/yunjian-mcp/src/schema.rs:24-25`。同一件事在 MCP 与 GUI 上
    // 若有两种措辞，用户会以为是两种不同的限制。
    expect(AI_UNREVIEWED_DISCLOSURE).toBe(
      "本结果包含 AI 生成内容，未经人工审校，可能存在事实、典故或格律错误，请独立核验。",
    );
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("ai-disclosure").textContent).toBe(AI_UNREVIEWED_DISCLOSURE);
  });

  it.each([
    ["absent", { kind: "absent" } as AppreciationState],
    [
      "configuration_required",
      { kind: "configuration_required", settings_path: "设置" } as AppreciationState,
    ],
    ["failed", { kind: "failed", message: "网络不可用" } as AppreciationState],
  ])("没有正文时（%s）披露与标签依然在", (_name, state) => {
    // 面板本身的性质不因内容缺失而改变。空面板不带标注，
    // 就会在「稍后重试」之后变成一块无标注的文字容器。
    render(<AiAppreciationPanel state={state} />);
    expect(screen.getByTestId("ai-panel-label").textContent).toBe("AI 赏析");
    expect(screen.getByTestId("ai-disclosure")).toBeDefined();
    expect(screen.getByTestId("ai-unreviewed-badge")).toBeDefined();
  });
});

describe("模型名与溯源", () => {
  it("模型名显示出来", () => {
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("ai-model").textContent).toBe("qwen2.5-7b-instruct");
  });

  it("模板版本显示出来", () => {
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("ai-template-version").textContent).toBe("1.2.0");
  });

  it("来源三态各自有自己的中文说明", () => {
    for (const [source, label] of [
      ["shipped", "随包预生成"],
      ["cache", "本机缓存"],
      ["generated", "本次生成"],
    ] as const) {
      const { unmount } = render(
        <AiAppreciationPanel state={{ kind: "ready", view: { ...READY.view, source } as never }} />,
      );
      expect(screen.getByTestId("ai-source").textContent).toBe(label);
      unmount();
    }
  });

  it("拿不到来源时少一行，而不是编一个「本次生成」出来", () => {
    // `CacheSource` 在 Rust 侧没有 Serialize，且公开的 appreciate() 把它丢掉了
    // （crates/yunjian-ai/src/cache.rs:363-369）。所以「拿不到」是真实且常见的情形。
    render(
      <AiAppreciationPanel
        state={{
          kind: "ready",
          view: { text: "文本", model: "m", template_version: "1.0.0" },
        }}
      />,
    );
    expect(screen.queryByTestId("ai-source")).toBeNull();
  });
});

describe("状态", () => {
  it("需要配置时给出产品内路径，而不是一句「出错了」", () => {
    render(
      <AiAppreciationPanel
        state={{ kind: "configuration_required", settings_path: "云笺 → 设置 → AI 服务商与密钥" }}
      />,
    );
    expect(screen.getByTestId("ai-configuration-required").textContent).toContain("设置");
  });

  it("失败态用 alert 角色，屏幕阅读器会读出来", () => {
    render(<AiAppreciationPanel state={{ kind: "failed", message: "网络不可用" }} />);
    expect(screen.getByRole("alert").textContent).toBe("网络不可用");
  });
});

describe("分界", () => {
  it("面板前有一条分界与一句说明", () => {
    render(<AiAppreciationPanel state={READY} />);
    expect(screen.getByTestId("provenance-divider")).toBeDefined();
    expect(screen.getByTestId("ai-boundary-caption").textContent).toContain("不属于考据材料");
  });
});

describe("快照", () => {
  it("面板结构与类名整体钉住", () => {
    // 快照的价值在于「容器类被改动」这件事会直接显示成 diff。
    // 单独断言 className 只能钉住我想到的那几个类。
    const { container } = render(<AiAppreciationPanel state={READY} />);
    expect(container.innerHTML).toMatchSnapshot();
  });
});
