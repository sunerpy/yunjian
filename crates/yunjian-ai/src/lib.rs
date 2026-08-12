//! 云笺 AI crate。
//!
//! 负责 AI 赏析生成、提示词模板版本管理与多供应商适配。所有生成内容
//! 均标注为 AI 产出，不进入语料表。
//!
//! 供应商边界只公开本 crate 与 `yunjian-core` 持有的领域类型，不向调用方泄漏具体
//! 模型 SDK。长任务统一复用 `yunjian-core::operation`。
//!
//! 凭据存储见 [`keystore`]：它只暴露 [`StorageReport`] 这类非机密描述，密钥本身一律
//! 包在 [`secrecy::SecretString`] 里，不进配置文件、不进日志、不进 `Debug` 输出。
//!
//! 实际调用模型的实现见 [`genai_provider`]：它把钥匙串里的密钥以 resolver 注入
//! `genai`，`AuthData::FromEnv` 在本 crate 中不出现，密钥不进进程环境。

#![warn(missing_docs)]

pub mod cache;
pub mod genai_provider;
pub mod keystore;
pub mod provider;
mod stream;

pub use cache::{
    APPRECIATION_DATABASE_FILE, AppreciationCache, CacheCounts, CacheHit, CacheSource,
    CachedAppreciationProvider, DEFAULT_APPRECIATION_CACHE_CAPACITY, PurgeScope,
    ShippedAppreciation,
};
pub use genai_provider::{GenAiProvider, GenAiProviderConfig, ProviderKind};
pub use keystore::{
    Backend, KeyStore, KeyStoreConfig, Lookup, OsKeychain, Persistence, Protection, StorageReport,
    default_plaintext_path, install_default_store,
};
pub use provider::{
    APPRECIATION_TEMPLATE, APPRECIATION_TEMPLATE_FILE, APPRECIATION_TEMPLATE_VERSION, AiProvider,
    Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
    AppreciationStreamItem, GeneratedPoem, NullProvider, PoemGenerationProvider,
    PoemGenerationRequest, PromptTemplate, ProviderId, TokenUsage,
};
pub use stream::AppreciationCacheWriter;
