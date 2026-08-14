import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createSamplePorts } from "../../data/samplePorts";
import type { DictionaryPort } from "../../data/ports";
import DictionaryPanel from "../DictionaryPanel";

describe("内置字典面板", () => {
  it("初始为空态，提交双字后按顺序展示逐字事实与三层来源", async () => {
    render(<DictionaryPanel port={createSamplePorts().dictionary} />);
    expect(screen.getByTestId("dictionary-empty")).toBeTruthy();

    fireEvent.click(screen.getByTestId("dictionary-submit"));

    await waitFor(() => {
      expect(screen.getByTestId("dictionary-results")).toBeTruthy();
    });
    const characters = screen
      .getByTestId("dictionary-results")
      .querySelectorAll<HTMLElement>("[data-testid^='dictionary-character-']");
    expect([...characters].map((entry) => entry.dataset.testid)).toEqual([
      "dictionary-character-斜",
      "dictionary-character-阳",
    ]);
    expect(screen.getByText(/双字请求 · 逐字事实/)).toBeTruthy();
    expect(screen.getByText(/韵部实证/)).toBeTruthy();
    expect(screen.getAllByText(/不能单独推出当前拼音/)).toHaveLength(2);
    expect(document.querySelectorAll("[data-source-layer='rhyme']")).toHaveLength(2);
    expect(document.querySelectorAll("[data-source-layer='public-lexicon']")).toHaveLength(2);
    expect(document.querySelectorAll("[data-source-layer='ai']")).toHaveLength(2);
    expect(screen.queryByText(/词义：/)).toBeNull();
  });

  it("把一至二字与原句作为同一个请求提交", async () => {
    const lookupDictionary = vi.fn(createSamplePorts().dictionary.lookupDictionary);
    render(<DictionaryPanel port={{ lookupDictionary }} />);
    fireEvent.change(screen.getByTestId("dictionary-query"), { target: { value: "斜" } });
    fireEvent.change(screen.getByTestId("dictionary-context"), {
      target: { value: "远上寒山石径斜" },
    });
    fireEvent.click(screen.getByTestId("dictionary-submit"));

    await waitFor(() => {
      expect(lookupDictionary).toHaveBeenCalledWith({
        query: "斜",
        context: "远上寒山石径斜",
      });
    });
  });

  it("查询失败时显示错误且不保留旧结果", async () => {
    const port: DictionaryPort = {
      lookupDictionary: () => Promise.reject(new Error("字典暂不可用")),
    };
    render(<DictionaryPanel port={port} />);
    fireEvent.click(screen.getByTestId("dictionary-submit"));

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("字典暂不可用");
    });
    expect(screen.queryByTestId("dictionary-results")).toBeNull();
  });
});
