import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "../App";
import { MOBILE_FEATURE_GROUPS, MOBILE_ROUTES } from "../mobileRoutes";

describe("移动端共享 UI 路由契约", () => {
  it("六个桌面功能组都有且只有一条移动可达路径", () => {
    expect(MOBILE_FEATURE_GROUPS).toEqual([
      "search",
      "detail",
      "ai",
      "recitation",
      "voice",
      "settings-keystore",
    ]);
    expect(MOBILE_ROUTES.map(({ group }) => group)).toEqual(MOBILE_FEATURE_GROUPS);
    expect(new Set(MOBILE_ROUTES.map(({ route }) => route)).size).toBe(MOBILE_ROUTES.length);
  });

  it("每条路径都钉住真实入口和最终功能表面", () => {
    for (const route of MOBILE_ROUTES) {
      expect(route.entryTestId, `${route.group} 缺少真实入口`).not.toBe("");
      expect(route.surfaceTestId, `${route.group} 缺少最终功能表面`).not.toBe("");
    }
  });

  it("嵌套能力明确挂在用户实际进入的父屏上", () => {
    expect(MOBILE_ROUTES.find(({ group }) => group === "detail")?.parent).toBe("search");
    expect(MOBILE_ROUTES.find(({ group }) => group === "ai")?.parent).toBe("detail");
    expect(MOBILE_ROUTES.find(({ group }) => group === "voice")?.parent).toBe("recitation");
    expect(MOBILE_ROUTES.find(({ group }) => group === "settings-keystore")?.parent).toBe("search");
  });

  it("从共享外壳逐组走到六个真实功能表面", async () => {
    render(<App />);
    await waitFor(() => {
      expect(screen.getByRole("option", { name: /思乡/ })).toBeTruthy();
    });
    expect(screen.getByTestId("search-input")).toBeTruthy();

    fireEvent.change(screen.getByTestId("search-input"), { target: { value: "明月" } });
    fireEvent.click(screen.getByTestId("search-submit"));
    const result = await screen.findAllByTestId("result-row");
    const open = result[0]?.querySelector("button");
    expect(open).toBeTruthy();
    fireEvent.click(open as HTMLButtonElement);
    expect(await screen.findByTestId("poem-detail")).toBeTruthy();
    expect(await screen.findByTestId("ai-panel")).toBeTruthy();

    fireEvent.click(screen.getByTestId("nav-recite"));
    expect(await screen.findByTestId("recite-screen")).toBeTruthy();
    fireEvent.click(screen.getByTestId("mode-voice"));
    fireEvent.click(screen.getByTestId("start-session"));
    expect(await screen.findByTestId("voice-availability-note")).toBeTruthy();

    fireEvent.click(screen.getByTestId("nav-settings"));
    expect(await screen.findByTestId("key-storage-indicator")).toBeTruthy();
  });
});
