/**
 * AI 赏析的传输形状与**面向用户的固定文案**。
 *
 * # 文案不是设计选择，是标注义务
 *
 * `docs/AI.zh.md:268-275` 把三条义务写成了产品契约，本文件把其中与界面有关的两条固化成常量：
 *
 * > 1. **视觉分区。** 界面里 AI 赏析与有出处的集评**在视觉上分开呈现**，不渲染在同一个视觉
 * >    层级里。把没有出处的生成文字排成和带卷次页码的引文一样的样子，本身就是一种误导。
 * > 2. **附「未经人工审校」的说明。**
 *
 * 以及准确性披露（`docs/AI.zh.md:282-285`）：
 *
 * > **准确性免责，说清楚它的形状：** AI 赏析可能编造典故、错置作者、误解句意……
 * > 它没有出处可核，因此**不能当作事实引用**。
 *
 * # 一处措辞冲突，已按文档取值
 *
 * 任务书写的提示语是「AI 生成，未经校订」，而 `docs/AI.zh.md:272` 与
 * `crates/yunjian-mcp/src/schema.rs:24-25` 的既有常量写的是「未经人工审校」。
 * **取文档与代码里已存在的那一个**——界面上出现两种说法比说法本身更糟，而且
 * 「照 docs/AI.zh.md 写，不要自己发明文案」是更强的一条要求。
 * 短标签因此是 `AI 生成，未经人工审校`：句式来自任务书，用词来自文档。
 */

/**
 * AI 赏析面板的标签。
 *
 * **必须是「AI 赏析」而不是「赏析」。** 少掉那两个字母，这块内容就变成了看起来像考据成果的
 * 无出处文字，而整个产品的许可立场（只分发公有领域原文 + 有出处的前人集评 + 明确标注的 AI 输出）
 * 就是靠这个标签成立的。`docs/AI.zh.md:275` 记的 MCP 工具注解同样是 `title = "AI 赏析"`。
 */
export const AI_PANEL_LABEL = "AI 赏析";

/**
 * 未经审校的短提示，与标签同排显示。
 *
 * 用词取自 `docs/AI.zh.md:272` 的「未经人工审校」。
 */
export const AI_UNREVIEWED_BADGE = "AI 生成，未经人工审校";

/**
 * 完整准确性披露，逐字取自 `crates/yunjian-mcp/src/schema.rs:24-25` 的
 * `AI_UNREVIEWED_DISCLOSURE`。
 *
 * 与短提示并存不是重复：短提示是标签旁的一瞥，这一条是把「错在哪些方面」说清楚。
 * `docs/AI.zh.md` 要求的是后者——"说清楚它的形状"。
 */
export const AI_UNREVIEWED_DISCLOSURE =
  "本结果包含 AI 生成内容，未经人工审校，可能存在事实、典故或格律错误，请独立核验。";

/**
 * 分界说明，渲染在考据材料与 AI 面板之间。
 *
 * 「不与原文或集评交错排布」这条要求在 DOM 上由顺序与容器隔离保证，
 * 但顺序对用户是不可见的；这句话是把那条保证说出来。
 */
export const AI_BOUNDARY_CAPTION = "以下内容由 AI 生成，不属于考据材料。";

/**
 * 结果来源的三态。
 *
 * 取值逐字取自 `crates/yunjian-mcp/src/lib.rs:513-517` 的映射，**不是** Rust 变体名的
 * snake_case：
 *
 * ```rust
 * CacheSource::Shipped   => "shipped",
 * CacheSource::Local     => "cache",      // 注意：变体叫 Local，线上串叫 cache
 * CacheSource::Generated => "generated",
 * ```
 *
 * 猜成 `"local"` 会让来源标注静默失效（落到 `default` 分支），这正是本项目栽过六次的那类错。
 */
export type AppreciationSource = "shipped" | "cache" | "generated";

/** 来源的中文说明。三态各自一句，不合并。 */
export const APPRECIATION_SOURCE_LABEL: Record<AppreciationSource, string> = {
  shipped: "随包预生成",
  cache: "本机缓存",
  generated: "本次生成",
};

/**
 * AI 赏析面板要渲染的内容。
 *
 * `text` / `model` / `template_version` 对应 `crates/yunjian-ai/src/provider.rs:247-264` 的
 * `Appreciation`。
 *
 * **`source` 刻意是可选的。** `CacheSource` 在 Rust 侧既没有 `Serialize`，公开的
 * `AppreciationProvider::appreciate` 也把它丢掉了（`crates/yunjian-ai/src/cache.rs:363-369`
 * 只返回 `hit.appreciation`）；只有 MCP 的 `AppreciatePoemOutput` 才把它映射成串。
 * 因此桌面端能不能拿到来源，取决于 todo 64 的 IPC 命令返回 `CacheHit` 还是 `Appreciation`。
 * 拿不到时面板少一行，而不是编一个「本次生成」出来。
 */
export interface AppreciationView {
  text: string;
  /** 实际使用的模型标识，必须显示——它是「这段话是谁写的」的唯一线索。 */
  model: string;
  template_version: string;
  source?: AppreciationSource;
}

/** AI 面板的四种状态。空态与错误态刻意与「有内容」区分，避免空白被读成「没有争议」。 */
export type AppreciationState =
  | { kind: "ready"; view: AppreciationView }
  | { kind: "absent" }
  | { kind: "configuration_required"; settings_path: string }
  | { kind: "failed"; message: string };
