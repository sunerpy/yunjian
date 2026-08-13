/**
 * 集评条目的出处校验。
 *
 * # 这是一条硬规则在 UI 层的落实，不是防御性编程
 *
 * 出处标准是本项目的硬错误：每条集评必须带能复核的卷次/版本定位符，
 * **空而存在的引用与缺失的引用同罪**。构建期由
 * `crates/yunjian-corpus/src/commentary.rs:289-302` 把关（`value.trim().is_empty()` 即拒收，
 * 注释写明「只有非空的 work 会让无法定位的引用溜过去」），
 * 运行期由 `crates/yunjian-core/src/search/topic.rs:760-781` 再拦一次
 * （返回 `Error::CommentaryCitationMissing`，而不是构造一条无出处的 `CommentaryEntry`）。
 *
 * 于是有人会问：既然两道都拦了，前端为什么还要校验一遍？
 * 因为**类型上不可能不等于运行时不可能**。到了前端这一侧，数据是 IPC 送来的 JSON，
 * 任何形状都能过来；而这一层一旦漏了，产品就会把一段无从复核的文言排成
 * 「看起来像考据成果」的样子——那正是整个语料立场要避免的事。
 *
 * # 不复刻构建期的定位符正则
 *
 * 构建期还要求 `source_note` 含卷次定位符（形如「卷一」「第十二则」）与版本说明（形如「据…本」），
 * 见 `commentary.rs:349-366`。**前端刻意不重复那两条判据**：那是构建期的判定，
 * 在这里再写一遍正则等于把一份判据分叉成两份，将来必然一处改一处不改。
 * 前端只判「在不在、空不空」，这是它能独立负责的部分。
 */

import type { CommentaryEntry } from "../contracts/core";

/**
 * 缺失字段的标识符。
 *
 * 取值逐字对齐 core 的 `missing_field`（`topic.rs:762-781`），
 * 这样前端报出的字段名与后端错误里的字段名是同一套词，排查时不必做心算翻译。
 */
export type MissingCommentaryField =
  | "citation"
  | "citation_work"
  | "citation_author"
  | "citation_dynasty"
  | "citation_source_note"
  | "citation_work_completed_by"
  | "text";

/** 缺失字段的中文说明，用于错误态文案。 */
const FIELD_LABEL: Record<MissingCommentaryField, string> = {
  citation: "整个出处对象",
  citation_work: "所引著作",
  citation_author: "评者",
  citation_dynasty: "评者朝代",
  citation_source_note: "卷次与版本定位符",
  citation_work_completed_by: "成书年上界",
  text: "评语正文",
};

/** 校验结果。`valid` 分支保留原条目，`invalid` 分支只带缺失字段，**不带正文**。 */
export type CommentaryCheck =
  | { kind: "valid"; entry: CommentaryEntry }
  | { kind: "invalid"; id: string; missing: MissingCommentaryField[] };

function blank(value: unknown): boolean {
  return typeof value !== "string" || value.trim() === "";
}

/**
 * 校验一条集评。
 *
 * 返回 `invalid` 时**刻意不回传 `entry.text`**：调用方拿不到正文，就不可能不小心把它渲染出来。
 * 「不渲染没有出处的集评」这条要求因此由数据形状保证，而不是由调用方的自觉保证。
 */
export function checkCommentary(entry: CommentaryEntry): CommentaryCheck {
  const missing: MissingCommentaryField[] = [];
  const id = typeof entry.id === "string" && entry.id.trim() !== "" ? entry.id : "（无编号）";

  if (blank(entry.text)) {
    missing.push("text");
  }

  const citation = entry.citation as CommentaryEntry["citation"] | null | undefined;
  if (citation === null || citation === undefined || typeof citation !== "object") {
    missing.push("citation");
    return { kind: "invalid", id, missing };
  }

  if (blank(citation.work)) {
    missing.push("citation_work");
  }
  if (blank(citation.author)) {
    missing.push("citation_author");
  }
  // 朝代是 `DynastyLabel`；`raw` 是上游原串，core 校验的正是它对应的那一列。
  if (citation.dynasty === null || citation.dynasty === undefined || blank(citation.dynasty.raw)) {
    missing.push("citation_dynasty");
  }
  if (blank(citation.source_note)) {
    missing.push("citation_source_note");
  }
  // 成书年是保守上界且必然早于 1912（随包 schema 有 CHECK）。
  // 0 或缺失都不是「未知」——它会让「前 1912」这条判据无从成立。
  if (
    typeof citation.work_completed_by !== "number" ||
    !Number.isFinite(citation.work_completed_by) ||
    citation.work_completed_by <= 0
  ) {
    missing.push("citation_work_completed_by");
  }

  if (missing.length > 0) {
    return { kind: "invalid", id, missing };
  }
  return { kind: "valid", entry };
}

/** 错误态文案。点名条目编号与缺失字段，因为那两项决定了排查从哪里开始。 */
export function missingCitationMessage(check: {
  id: string;
  missing: MissingCommentaryField[];
}): string {
  const fields = check.missing.map((field) => FIELD_LABEL[field]).join("、");
  return `集评 ${check.id} 缺出处（${fields}），已拒绝展示其正文。无法复核的评语不作为考据材料呈现。`;
}

/** 逐条校验。顺序与入参一致——集评的编排顺序本身就是编者的判断。 */
export function checkCommentaries(entries: readonly CommentaryEntry[]): CommentaryCheck[] {
  return entries.map(checkCommentary);
}

/**
 * 出处的内联显示串。
 *
 * 格式沿用 CLI 已有的人类可读渲染（`crates/yunjian-cli/src/output.rs:322-330`）：
 * `—— 朝代原串 评者《所引著作》（成书年上界）卷次与版本`。书名号由展示层加，
 * 因为 `citation.work` 里刻意不带（`commentary.rs:92-93`）。
 */
export function citationLine(citation: CommentaryEntry["citation"]): string {
  return `—— ${citation.dynasty.raw} ${citation.author}《${citation.work}》（${citation.work_completed_by}）${citation.source_note}`;
}
