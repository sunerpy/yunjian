/**
 * 设置界面的传输形状。
 *
 * # 每一个标识符都是从 Rust 侧抄来的，没有一个是发明的
 *
 * 本文件的取值域全部来自源码而不是记忆，逐项出处写在各自的注释里。理由不是谨慎：
 * 本项目已经**六次**因为凭记忆写标识符而栽（`.omo/notepads/yunjian/issues.md`），
 * 而这里错一个字符串的后果尤其重——密钥存储位置的指示串错了，用户是在丢了密钥之后才发现的。
 *
 * # 密钥的形状：**类型层就没有能装它的地方**
 *
 * Rust 侧的读取结果是 `Lookup::Found { report, secret }`（`keystore.rs:212-227`），
 * 带 `secret` 字段。本文件**刻意只投影 `report`**：
 * [`KeyStatus`] 没有、也不允许有任何能承载密钥的字段。
 *
 * 这不是「记得清空输入框」那一类纪律，而是与 todo 61 处理缺出处引用同一路的做法——
 * **让错误在类型上无从表达**。界面拿不到密钥，于是「把已存的密钥回显出来」这件事
 * 不是被禁止，而是写不出来：没有可读的来源。
 * 写入方向照旧存在（[`KeyStorePort.setKey`]），因为那是用户在输入；
 * 读回方向根本没有对应的方法。
 */

/* ────────────────────────── 密钥存储 ────────────────────────── */

/**
 * 密钥实际所在的后端。八个取值。
 *
 * 逐字取自 `crates/yunjian-ai/src/keystore.rs:46-82` 的 `Backend`：serde 是
 * `rename_all = "snake_case"`，且同文件的 `as_str()` 与之逐一相等（Rust 侧有
 * `report_field_vocabularies_are_stable` 断言两者不漂移）。
 *
 * **界面文案一律不从这里推导。** 这个类型存在只为两件事：显示「后端是哪一个」这条
 * 诊断信息，以及在测试里穷举组合。为什么不能从它推导，见 [`KeyPersistence`]。
 */
export type KeyBackend =
  | "secret_service"
  | "keyutils"
  | "windows_credential"
  | "apple_keychain"
  | "android_keystore"
  | "session_memory"
  | "plaintext_file"
  | "absent";

/**
 * 密钥能活多久。四个取值。
 *
 * 逐字取自 `crates/yunjian-ai/src/keystore.rs:84-126` 的 `Persistence`。
 *
 * # 为什么界面文案必须从这里推导，而不是从后端名
 *
 * `docs/AI.zh.md:57-75`「Linux keyutils 是内存型，重启即失（必须如实呈现）」把这件事
 * 写成了产品契约，原话是：
 *
 * > **这条必须显式写出来，因为把它称作「系统钥匙串」是一句假话。**
 * > …… `keyutils` 和 Secret Service 都是「钥匙串」，但**持久性根本不同**，这正是
 * > `StorageReport.persistence` 这个字段必须存在、且界面标签必须从它推导的全部理由。
 *
 * 具体形态：`keyutils` 是一个货真价实的 keychain entry（内核提供、总是可用），
 * 但它 `纯内存，重启即失效`。按后端名把它报成「系统钥匙串（持久）」，
 * 是一句**用户只有在丢了密钥之后才会发现的假话**。
 *
 * Rust 侧的对应做法：`StorageReport::settings_summary()`（`keystore.rs:167-199`）
 * 只看 `persistence` 与 `protection`，**刻意不看 `backend` 的名字**。
 * 本文件的 [`StorageIndicator`] 与 `settings/storageIndicator.ts` 是它在前端的镜像。
 */
export type KeyPersistence = "persistent" | "login_session" | "process_only" | "none";

/**
 * 密钥受什么保护。三个取值。
 *
 * 逐字取自 `crates/yunjian-ai/src/keystore.rs:128-149` 的 `Protection`。
 */
export type KeyProtection = "os_encrypted" | "process_memory" | "plaintext";

/**
 * 一次存储操作的诚实描述。**不含任何密钥材料。**
 *
 * 字段名逐一对应 `crates/yunjian-ai/src/keystore.rs:151-165` 的 `StorageReport`。
 * 该结构体上没有 `rename_all`、没有 `tag`、没有字段级 rename，所以 JSON 属性名就是
 * `backend` / `persistence` / `protection` / `location` 四个。
 *
 * `location` 是**可显示的非机密字符串**（路径或后端名），不是枚举。
 *
 * `backend` 为 `absent` 时，`protection` 与 `location` 描述的是**被查询的那一层**
 * （「若存在会在哪、受什么保护」），此时 `persistence` 一律是 `none`。
 */
export interface StorageReport {
  backend: KeyBackend;
  persistence: KeyPersistence;
  protection: KeyProtection;
  location: string;
}

/**
 * 一次密钥查询的结果，**只投影 `Lookup` 里不含密钥的那一半**。
 *
 * `needs_reprompt` 对应 `Lookup::needs_reprompt()`（`keystore.rs:229-237`）。
 * 它的注释说明了为什么这个信号必须存在：
 *
 * > 这是 keyutils「内存型、不跨重启」这一事实在 API 上的落点：调用方不得假定持久，
 * > 每条读路径都要照这个信号走重新索要，而不是把它当异常抛出去。
 *
 * 所以「密钥不存在」在界面上是一个正常状态（提示重新输入），不是错误横幅。
 */
export interface KeyStatus {
  report: StorageReport;
  needs_reprompt: boolean;
}

/**
 * 设置界面用来指示「密钥实际存在哪里」的短标签，取值封闭。
 *
 * 前四个是任务书规定的四种指示：
 * `系统钥匙串（持久）` / `系统密钥环（重启后失效）` / `仅本次会话` / `明文配置文件`。
 *
 * 后两个不在那四种之内，各自对应一个 Rust 侧确实存在、而四种指示覆盖不到的组合：
 *
 * - `尚未存储` ← `persistence: "none"`（`backend: "absent"`）。此时没有「密钥在哪」
 *   这回事，硬塞进四种里的任何一种都是编造一个不存在的位置。
 *   对应 `settings_summary()` 的 `(None, _)` 分支。
 * - `保护方式与持久性不一致` ← `(persistent, process_memory)`。
 *   对应 `settings_summary()` 的最后一个分支，那里的处置是「按不可信处理」。
 *   这个组合在当前实现里出不来，但它在类型上可表达，因此必须有落点——
 *   落到 `default` 里冒充「系统钥匙串」正是这套推导要防的失败。
 *
 * # 一处措辞的调和
 *
 * `docs/AI.zh.md:68` 记的 `settings_summary()` 输出是一整句
 * 「内核会话密钥环（…）：**重启或注销后失效**，届时需要重新输入密钥」，
 * 而任务书要求的短指示是 `系统密钥环（重启后失效）`。两者不冲突也不能二选一：
 * **短指示做标签，文档那句做详情行**（[`STORAGE_INDICATOR_DETAIL`]）。
 * 沿用 todo 61 处理「未经校订 / 未经人工审校」冲突时的同一条办法——
 * 界面上出现两种说法比说法本身更糟。
 */
export type StorageIndicator =
  | "系统钥匙串（持久）"
  | "系统密钥环（重启后失效）"
  | "仅本次会话"
  | "明文配置文件"
  | "尚未存储"
  | "保护方式与持久性不一致";

/**
 * 每种指示各自的详情行，措辞取自 `settings_summary()` 与 `docs/AI.zh.md`，不自行发明。
 *
 * 详情行不是装饰：`系统密钥环（重启后失效）` 这个标签只说了「会失效」，
 * 没说「届时你要做什么」。文档那句把后半截说清楚了。
 */
export const STORAGE_INDICATOR_DETAIL: Record<StorageIndicator, string> = {
  "系统钥匙串（持久）": "重启后依然有效。",
  "系统密钥环（重启后失效）": "重启或注销后失效，届时需要重新输入密钥。",
  仅本次会话: "退出即失效，下次启动需要重新输入密钥。",
  明文配置文件: "未加密，任何能读到该文件的程序都能拿到密钥。",
  尚未存储: "尚未保存任何密钥，需要输入密钥。",
  保护方式与持久性不一致: "按不可信处理，请重新输入密钥。",
};

/* ────────────────────────── AI 服务商 ────────────────────────── */

/**
 * 支持的服务商标识，顺序与 `ProviderKind::ALL` 一致。
 *
 * 逐字取自 `crates/yunjian-ai/src/genai_provider.rs:81-113`：`ALL` 的注释明说这个顺序
 * 「稳定，供设置界面与表驱动测试枚举」，所以这里照抄顺序而不是重新排。
 *
 * **`ProviderKind` 在 Rust 侧没有 serde derive**，跨 IPC 传的是 `as_str()` 的那十个
 * 字符串；`AiConfig.provider` 本身就是 `String`（`config.rs:136-155`）。
 */
export const PROVIDER_IDS = [
  "deepseek",
  "kimi",
  "moonshot",
  "qwen",
  "zai",
  "bigmodel",
  "openrouter",
  "ollama",
  "openai",
  "anthropic",
] as const;

export type ProviderId = (typeof PROVIDER_IDS)[number];

/**
 * 「不配置任何服务商」的哨兵值。
 *
 * 逐字取自 `crates/yunjian-core/src/config.rs:40-41` 的 `PROVIDER_NONE`。
 * 它不是「空字符串」也不是 `null`：`AiConfig::default()` 就用这个值，
 * 语义是「只使用随包预生成的赏析」。
 */
export const PROVIDER_NONE = "none";

/** 服务商的中文显示名。只影响显示，标识符仍是 [`PROVIDER_IDS`] 里的那十个。 */
export const PROVIDER_LABEL: Record<ProviderId, string> = {
  deepseek: "深度求索 DeepSeek",
  kimi: "月之暗面 Kimi",
  moonshot: "月之暗面 Moonshot",
  qwen: "阿里云百炼 / 通义千问",
  zai: "智谱 Z.ai 国际站",
  bigmodel: "智谱开放平台 BigModel",
  openrouter: "OpenRouter 聚合网关",
  ollama: "本地 Ollama",
  openai: "OpenAI",
  anthropic: "Anthropic",
};

/**
 * 每个服务商留空时实际使用的 base URL 与模型。
 *
 * 逐字取自 `genai_provider.rs:149-166` 的 `default_base_url()` 与
 * `genai_provider.rs:177-190` 的 `default_model()`。前者的注释写明它「供设置界面显示
 * 『留空即使用』的那个值」——所以这份表的用途就是那个 placeholder，
 * 而不是让界面替用户把值填进配置。
 *
 * 注意 `kimi` 与 `moonshot` 共用同一组默认值：Rust 侧那两个变体落在同一个 match 臂上。
 */
export const PROVIDER_DEFAULTS: Record<ProviderId, { base_url: string; model: string }> = {
  deepseek: { base_url: "https://api.deepseek.com/v1/", model: "deepseek-chat" },
  kimi: { base_url: "https://api.moonshot.cn/v1/", model: "moonshot-v1-8k" },
  moonshot: { base_url: "https://api.moonshot.cn/v1/", model: "moonshot-v1-8k" },
  qwen: { base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1/", model: "qwen-plus" },
  zai: { base_url: "https://api.z.ai/api/paas/v4/", model: "glm-4-flash" },
  bigmodel: { base_url: "https://open.bigmodel.cn/api/paas/v4/", model: "glm-4-flash" },
  openrouter: { base_url: "https://openrouter.ai/api/v1/", model: "openai/gpt-4o-mini" },
  ollama: { base_url: "http://localhost:11434/", model: "qwen2.5:7b" },
  openai: { base_url: "https://api.openai.com/v1/", model: "gpt-4o-mini" },
  anthropic: { base_url: "https://api.anthropic.com/v1/", model: "claude-3-5-haiku-latest" },
};

/**
 * `[ai]` 里界面可编辑的那几项。
 *
 * 字段名与 `crates/yunjian-core/src/config.rs:136-155` 的 `AiConfig` 逐一对应。
 * **那个结构体里没有、并且永远不会有承载 API Key 的字段**，本类型同理——
 * 顶层配置开了 `deny_unknown_fields`，粘一个 `api_key` 进去会直接报错。
 */
export interface AiSettings {
  provider: string;
  model: string | null;
  endpoint: string | null;
  temperature: number;
  prompt_template_version: string;
}

/* ────────────────────────── 语料 ────────────────────────── */

/**
 * 语料库自述的身份与形态。
 *
 * 字段名逐一对应 `crates/yunjian-core/src/corpus.rs:178-198` 的 `CorpusMeta`，
 * 由 `Corpus::meta()`（`corpus.rs:408-412`）取得。
 *
 * **记录数的字段名是 `poem_count` 而不是 `record_count`。** 猜错这一个词，
 * 界面会显示 `undefined` 首——而那正是本项目栽过六次的那类错。
 */
export interface CorpusMeta {
  schema_version: number;
  corpus_version: string;
  built_at: string;
  poem_count: number;
  index_detail_mode: string;
  derived_indexes: string;
  shipped_scope: string;
}

/**
 * 语料状态的两态。
 *
 * 「没有语料库」是首次启动的正常处境（README 说明随包语料要下载 211 MiB），
 * 不是错误。把它做成错误态会让首启用户看到一条红色报错。
 */
export type CorpusStatus = { kind: "ready"; meta: CorpusMeta } | { kind: "absent" };

/* ────────────────────────── 语音模型 ────────────────────────── */

/** 模型用途。取自 `crates/yunjian-voice/src/models.rs:93-104` 的 `ModelKind`。 */
export type ModelKind = "asr" | "tts";

/** 是否进产品路径。取自 `crates/yunjian-voice/src/models.rs:113-124` 的 `ModelRole`。 */
export type ModelRole = "production" | "smoke";

/**
 * 一个模型此刻的本地状态，对应 `yunjian models list` 的行。
 *
 * 字段名逐一对应 `crates/yunjian-voice/src/models.rs:654-675` 的 `ModelStatus`。
 *
 * # 下载状态不是一个枚举
 *
 * Rust 侧**没有** `DownloadState` 这种类型。下载进展由两个布尔表达：
 * `unpacked`（解包后的模型目录是否就位）与 `archived`（已校验归档是否还在本地）。
 * 界面把这两位合成一句人话（见 `settings/modelState.ts`），
 * 但**不发明一个后端给不出的枚举**。
 *
 * `refused` 是许可门禁的判定：通过为 `null`，被拒时是给用户看的原因。
 * `statuses()` 的注释说明了为什么被拒的模型也要列出来：
 * 「藏起来只会让人以为清单没被改过」。
 */
export interface ModelStatus {
  name: string;
  kind: ModelKind;
  role: ModelRole;
  /** SPDX 许可。**每一行都必须显示它**——只放行 MIT 与 Apache-2.0 是产品的许可立场。 */
  license: string;
  /** **归档**（压缩包）的字节数，不是解包后的体积。 */
  size_bytes: number;
  unpacked: boolean;
  archived: boolean;
  refused: string | null;
  attribution: string;
}

/* ────────────────────────── 缓存 ────────────────────────── */

/**
 * 两张赏析表的行数。
 *
 * 字段名对应 `crates/yunjian-ai/src/cache.rs:47-54` 的 `CacheCounts`，
 * 由 `AppreciationCache::counts()`（`cache.rs:454-463`）返回。
 */
export interface CacheCounts {
  /** 随包预生成行数。**清理动作永不触及这一层。** */
  shipped: number;
  /** 用户自费生成行数。 */
  local: number;
}

/**
 * 缓存面板要显示的全部内容。
 *
 * # `database_bytes` 为什么是可选的
 *
 * Rust 的缓存模块**只有行数没有字节数**：`counts()` 返回 `CacheCounts`，
 * `cache.rs` 里没有任何报告磁盘体积的函数。缓存库是一个磁盘文件，
 * 因此体积是 IPC 层 `stat` 一下就能得到的东西，**但那是 todo 64 的实现选择**，
 * 不是 `yunjian-ai` 已经提供的能力。
 *
 * 所以这里照 todo 61 处理 `AppreciationView.source` 的同一条办法：
 * 标成可选，拿不到时显示「未知」，**而不是编一个数字出来**。
 */
export interface CacheStatus {
  counts: CacheCounts;
  database_bytes?: number;
}

/**
 * 缓存清理范围。
 *
 * # 这个形状是前端提出的，不是从 serde 抄来的
 *
 * `crates/yunjian-ai/src/cache.rs:101-110` 的 `PurgeScope` 有三个变体
 * （`Template(String)` / `Poem(String)` / `All`），但它**没有 `Serialize`/`Deserialize`
 * derive，也没有任何 serde 属性**——也就是说线上形态目前不存在，
 * 猜一个 tagged-union 出来当成「Rust 那边就是这样」是错的。
 *
 * 因此这里显式声明：**下面这个判别联合是本 todo 向 todo 64 提出的线上契约**，
 * 由 todo 64 在命令层做映射。沿用 `data/ports.ts` 已有的做法——界面先把它需要的形状
 * 说清楚，IPC 层去实现它。字段名照 Rust 变体的语义取（`template_version` 与
 * `poem_id`），而不是照 tuple 下标。
 */
export type PurgeScope =
  | { kind: "template"; template_version: string }
  | { kind: "poem"; poem_id: string }
  | { kind: "all" };
