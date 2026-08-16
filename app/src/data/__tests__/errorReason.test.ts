import { describe, expect, it } from "vitest";
import { errorReason } from "../errorReason";

describe("errorReason", () => {
  it("保留 Error 的 message", () => {
    expect(errorReason(new Error("语料库尚未就位"), "操作失败")).toBe("语料库尚未就位");
  });

  it("保留 Tauri invoke reject 的字符串原因", () => {
    expect(errorReason("invalid args `onEvent`", "操作失败")).toBe("invalid args `onEvent`");
  });

  it("保留结构化后端错误的 code、message 与 hint", () => {
    expect(
      errorReason(
        {
          code: "corpus_unavailable",
          message: "语料库尚未就位",
          hint: "请先下载语料",
        },
        "操作失败",
      ),
    ).toBe("[corpus_unavailable] 语料库尚未就位；请先下载语料");
  });
});
