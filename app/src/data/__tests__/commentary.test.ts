/**
 * 集评的出处校验：缺出处必须是**错误态**，不是空文本。
 *
 * 这一组对着本 todo 最硬的一条要求。构建期与运行期各有一道校验
 * （`crates/yunjian-corpus/src/commentary.rs:289-302` 与
 * `crates/yunjian-core/src/search/topic.rs:760-781`），但类型上不可能不等于运行时不可能：
 * 到了前端这一侧数据是 IPC 送来的 JSON，任何形状都能过来。
 */

import { describe, expect, it } from "vitest";
import type { CommentaryEntry } from "../../contracts/core";
import {
  checkCommentaries,
  checkCommentary,
  citationLine,
  missingCitationMessage,
} from "../commentary";

function entry(overrides: Partial<CommentaryEntry["citation"]> = {}): CommentaryEntry {
  return {
    id: "c-1",
    text: "语极浅而意极远。",
    citation: {
      work: "沧浪诗话",
      author: "严羽",
      dynasty: { canonical: "宋", raw: "宋" },
      work_completed_by: 1250,
      source_note: "卷一，据历代诗话本",
      ...overrides,
    },
  };
}

describe("完整的出处", () => {
  it("五个字段都在就通过", () => {
    const check = checkCommentary(entry());
    expect(check.kind).toBe("valid");
  });

  it("内联出处串的格式与 CLI 的人类可读输出一致", () => {
    // 格式取自 `crates/yunjian-cli/src/output.rs:322-330`。书名号由展示层加，
    // 因为 `citation.work` 里刻意不带（`commentary.rs:92-93`）。
    expect(citationLine(entry().citation)).toBe("—— 宋 严羽《沧浪诗话》（1250）卷一，据历代诗话本");
  });
});

describe("空而存在的引用与缺失的引用同罪", () => {
  // 这是出处标准里最容易被绕过的一种形态：字段在、值是空白。
  // 只判 `undefined` 的实现会让它整条通过。
  it.each([
    ["work", { work: "" }, "citation_work"],
    ["work 只有空格", { work: "   " }, "citation_work"],
    ["author", { author: "" }, "citation_author"],
    ["source_note", { source_note: "" }, "citation_source_note"],
    ["source_note 只有空格", { source_note: " \t " }, "citation_source_note"],
  ] as const)("%s 为空即判缺出处", (_name, overrides, field) => {
    const check = checkCommentary(entry(overrides));
    expect(check.kind).toBe("invalid");
    if (check.kind === "invalid") {
      expect(check.missing).toContain(field);
    }
  });

  it("朝代原串为空也算缺出处", () => {
    const check = checkCommentary(entry({ dynasty: { canonical: "宋", raw: "  " } }));
    expect(check.kind).toBe("invalid");
    if (check.kind === "invalid") {
      expect(check.missing).toContain("citation_dynasty");
    }
  });

  it("成书年为 0 算缺出处而不是「未知」", () => {
    // 成书年上界必须早于 1912，这是「前现代作品即公有领域」这条判据的全部依据。
    // 0 让那条判据无从成立。
    const check = checkCommentary(entry({ work_completed_by: 0 }));
    expect(check.kind).toBe("invalid");
    if (check.kind === "invalid") {
      expect(check.missing).toContain("citation_work_completed_by");
    }
  });

  it("整个 citation 缺失时不再逐字段报，直接判 citation", () => {
    const broken = { id: "c-2", text: "评语。" } as unknown as CommentaryEntry;
    const check = checkCommentary(broken);
    expect(check.kind).toBe("invalid");
    if (check.kind === "invalid") {
      expect(check.missing).toContain("citation");
    }
  });
});

describe("错误结构本身就拦住了正文泄漏", () => {
  it("invalid 分支不携带正文，所以调用方不可能不小心渲染它", () => {
    // 「不渲染没有出处的集评」这条要求由数据形状保证，而不是靠调用方的自觉。
    const check = checkCommentary(entry({ source_note: "" }));
    expect(check.kind).toBe("invalid");
    expect(JSON.stringify(check)).not.toContain("语极浅而意极远");
  });

  it("错误文案点名条目编号与缺失字段", () => {
    const check = checkCommentary(entry({ source_note: "" }));
    if (check.kind !== "invalid") {
      throw new Error("这条应当无效");
    }
    const message = missingCitationMessage(check);
    expect(message).toContain("c-1");
    expect(message).toContain("卷次与版本定位符");
    expect(message).toContain("拒绝展示");
  });
});

describe("批量校验", () => {
  it("保持入参顺序，因为集评的编排顺序本身是编者的判断", () => {
    const checks = checkCommentaries([entry(), entry({ source_note: "" }), entry({ work: "" })]);
    expect(checks.map((check) => check.kind)).toEqual(["valid", "invalid", "invalid"]);
  });
});
