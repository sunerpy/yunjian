/**
 * 全工作区长任务事件协议的线上形状。
 *
 * 逐字对应 `crates/yunjian-core/src/operation.rs` 的 `Event<P, I>`，它带
 * `#[serde(tag = "type", content = "payload", rename_all = "snake_case")]`——**邻接标签**，
 * 于是载荷在 `payload` 键下而不是与 `type` 平级。这一点与 `contracts/voice.ts` 里那些
 * **内部标签**的枚举正好相反，混起来会让判别式读到 `undefined` 而落到默认分支。
 *
 * 语料派生、AI 流式、流式识别、跟读会话四条长任务共用这一份语义，因此取消、背压与资源
 * 释放的行为在四处完全一致，界面不需要为每一条各写一套。
 */

/**
 * 一条长任务事件。
 *
 * 五个取值分工明确：
 *
 * - `progress` **可合并**。生产者比消费者快时中间快照会被覆盖掉，这是协议允许的行为——
 *   所以界面可以照它渲染进度，但**不能**指望收到某一个特定的中间值。
 * - `item` **不可丢弃**。增量结果走这一条。
 * - `done` / `cancelled` / `failed` 是三个终止取值，收到任一条之后流就结束了。
 */
export type Event<P, I> =
  | { type: "progress"; payload: P }
  | { type: "item"; payload: I }
  | { type: "done" }
  | { type: "cancelled" }
  | { type: "failed"; payload: { message: string } };

/** 该事件是否结束事件流。 */
export function isTerminal<P, I>(event: Event<P, I>): boolean {
  return event.type === "done" || event.type === "cancelled" || event.type === "failed";
}
