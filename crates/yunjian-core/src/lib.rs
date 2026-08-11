//! 云笺核心 crate。
//!
//! 承载配置加载、日志初始化、统一错误类型与稳定标识（`stable_id`）等
//! 与运行环境无关的领域逻辑。
//!
//! 本 crate 永远不感知 Tauri：不引入任何 `tauri::` 类型，也不假设宿主为
//! 桌面外壳，以此保留移动端的实现空间。
//!
//! 具体模块由后续任务补全。

pub mod config;
pub mod derive;
pub mod error;
pub mod logger;
pub mod rhyme;
pub mod text;

pub use config::{Config, LoggerConfig, get_config, init_config};
pub use derive::{
    DerivedBuildStats, build_derived_indexes, derived_indexes_present, verify_derived_indexes,
};
pub use error::{AiError, Error, Result, redact_credentials};
pub use logger::{current_log_level, init_logger, set_log_level};
pub use rhyme::{RhymeBook, RhymeTone};
pub use text::{content_chars, is_punctuation};
