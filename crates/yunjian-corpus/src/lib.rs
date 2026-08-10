//! 云笺语料 crate。
//!
//! 负责公共领域语料的入库、清洗、稳定标识铸造，以及 SQLite FTS5 检索
//! 索引的构建。检索是一个 SQLite 文件，而非独立搜索引擎。
//!
//! 入库器只产出候选记录，身份铸造统一由 [`model::rebuild_corpus`] 负责。

pub mod commentary;
pub mod ingest;
pub mod model;
pub mod rhyme;
