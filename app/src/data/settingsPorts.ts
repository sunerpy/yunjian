/**
 * 设置界面的数据访问端口。
 *
 * # 与 `data/ports.ts` 同一条理由
 *
 * 命令本体属 todo 64，而设置界面（本 todo）排在它之前落地。所以界面先把它需要的形状
 * 说清楚，todo 64 去实现，测试用替身实现，两侧对着同一份签名。
 *
 * 每个方法都对应一个已经存在的 Rust API，**没有一个是新发明的**：
 *
 * | 端口方法 | Rust API | 出处 |
 * | --- | --- | --- |
 * | `keyStatus` | `KeyStore::get(account)` → `Lookup` | `keystore.rs:449` |
 * | `setKey` | `KeyStore::set(account, secret)` | `keystore.rs:498` |
 * | `deleteKey` | `KeyStore::delete(account)` | `keystore.rs:525` |
 * | `readAiSettings` / `writeAiSettings` | `AiConfig` | `config.rs:136-155` |
 * | `corpusStatus` | `Corpus::meta()` | `corpus.rs:408-412` |
 * | `listModels` | `ModelCache::statuses()` | `models.rs:686-703` |
 * | `cacheStatus` | `AppreciationCache::counts()` | `cache.rs:454-463` |
 * | `purgeCache` | `AppreciationCache::purge(scope)` | `cache.rs:432-448` |
 *
 * # 读回密钥的方法**不存在**，这是刻意的
 *
 * 注意 [`KeyStorePort`] 的三个方法里：
 *
 * - `setKey` 收密钥、返回 [`StorageReport`]；
 * - `deleteKey` 返回 [`StorageReport`]；
 * - `keyStatus` 返回 [`KeyStatus`]，其中只有 `report` 与 `needs_reprompt`。
 *
 * **没有任何一个方法能把已保存的密钥交给界面。** Rust 侧的 `Lookup::Found` 确实带
 * `secret`，本端口刻意不投影它。于是「保存后把密钥回显出来」不是被规范禁止，
 * 而是**没有数据来源可用**——这条防护在类型层，不在纪律层。
 *
 * 这与 todo 61 让缺出处的引用「在类型上不携带正文」是同一路：
 * 让错误的输出无从表达，而不是指望后来者记得清空某个变量。
 */

import type {
  AiSettings,
  CacheStatus,
  CorpusProgressEvent,
  CorpusStatus,
  KeyStatus,
  ModelStatus,
  PurgeScope,
  StorageReport,
} from "../contracts/settings";

/**
 * 密钥存储端口。
 *
 * `provider` 就是钥匙串里的 account 名——`ProviderKind::as_str()` 的注释明说
 * 「同时用作钥匙串里的 account 名」（`genai_provider.rs:98-99`）。
 */
export interface KeyStorePort {
  /** 查询某服务商的密钥状态。「不存在」是正常返回，`needs_reprompt` 为 `true`。 */
  keyStatus(provider: string): Promise<KeyStatus>;
  /** 写入密钥，返回**它实际落在哪一层**的报告。界面据这份报告显示指示串。 */
  setKey(provider: string, secret: string): Promise<StorageReport>;
  /** 删除密钥。幂等：本来就不存在时返回 `backend: "absent"` 的报告而非报错。 */
  deleteKey(provider: string): Promise<StorageReport>;
}

/** `[ai]` 配置端口。密钥不在其中，见 `contracts/settings.ts` 的 `AiSettings`。 */
export interface AiSettingsPort {
  readAiSettings(): Promise<AiSettings>;
  writeAiSettings(settings: AiSettings): Promise<void>;
}

/**
 * 语料端口。`fetchCorpus` 是「取用/更新」动作，完成后返回新的状态。
 *
 * # `onEvent` 是必需参数，不是可选的
 *
 * 首启物化在唐宋规模上实测 571.8 s（`crates/yunjian-core/src/derive.rs` 的量），
 * 其中 487.5 s 在 n-gram 那一步。一个九分钟没有任何反馈的按钮与「按下去坏了」不可区分。
 * Rust 侧 `fetch_corpus` 本来就在逐步上报（`ipc.rs` 的 `Channel<Event<CorpusProgress, Value>>`），
 * 所以这里把进度汇做成**签名的一部分**：调用方拿不到一个「不接收进度」的重载，
 * 于是「把进度丢掉」这件事在类型层就写不出来。
 *
 * 这与 [`VoicePort.startSession`] 的做法一致（`data/voicePorts.ts`），也与本文件顶上
 * 「读回密钥的方法不存在」同一路——**让错误无从表达**，而不是在注释里嘱咐。
 */
export interface CorpusPort {
  corpusStatus(): Promise<CorpusStatus>;
  fetchCorpus(onEvent: (event: CorpusProgressEvent) => void): Promise<CorpusStatus>;
}

/** 模型端口。 */
export interface ModelPort {
  listModels(): Promise<ModelStatus[]>;
}

/** 缓存端口。`purgeCache` 返回删除行数，与 `AppreciationCache::purge` 一致。 */
export interface CachePort {
  cacheStatus(): Promise<CacheStatus>;
  purgeCache(scope: PurgeScope): Promise<number>;
}

/** 设置界面需要的全部端口。 */
export interface SettingsPorts {
  keyStore: KeyStorePort;
  aiSettings: AiSettingsPort;
  corpus: CorpusPort;
  models: ModelPort;
  cache: CachePort;
}
