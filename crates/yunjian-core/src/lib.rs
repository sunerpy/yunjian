//! 云笺核心 crate。
//!
//! 承载配置加载、日志初始化、统一错误类型与稳定标识（`stable_id`）等
//! 与运行环境无关的领域逻辑。
//!
//! 本 crate 永远不感知 Tauri：不引入任何 `tauri::` 类型，也不假设宿主为
//! 桌面外壳，以此保留移动端的实现空间。
//!
//! 具体模块由后续任务补全。

#![warn(missing_docs)]

pub mod api;
pub mod config;
pub mod corpus;
pub mod derive;
pub mod error;
pub mod logger;
pub mod operation;
pub mod rhyme;
pub mod search;
pub mod text;

pub use api::{
    AuthorDetailRequest, AuthorSearchRequest, CharacterRhymesRequest, DynastyBrowseRequest,
    FirstLineSearchRequest, LastCharacterSearchRequest, PoemDetailRequest, RhymeCheckRequest,
    RhymeGroupSearchRequest, TagBrowseRequest, TitleSearchRequest, WorkGroupRequest, Yunjian,
};
pub use config::{Config, CorpusConfig, LoggerConfig, get_config, init_config};
pub use corpus::{
    CORPUS_ARCHIVE_NAME, CORPUS_FILE_NAME, CORPUS_MANIFEST_NAME, CorpusHandle, CorpusMeta,
    CorpusOrigin, DerivedState, MaterializationProgress, OpenCorpusError, SCHEMA_VERSION,
    SUPPORTED_SCHEMA, open_corpus,
};
pub use derive::{
    DeriveProgress, DeriveStep, DerivedBuildStats, build_derived_indexes,
    build_derived_indexes_with_progress, derived_indexes_present, split_metrical_lines,
    split_rhyme_feet, verify_derived_indexes,
};
pub use error::{AiError, Error, Result, redact_credentials};
pub use logger::{current_log_level, init_logger, init_stdio_logger, set_log_level};
pub use rhyme::{RhymeBook, RhymeConfidence, RhymeTone};
pub use search::meta::{
    Attribution, AttributionConflict, AuthorDetail, DynastyLabel, META_PAGE_LIMIT, MetaHit,
    MetaMatch, MetaPage, TITLE_SEPARATORS, author_detail, browse_by_dynasty, find_by_author,
    find_by_first_line, find_by_last_char, find_by_title, find_work_group_attributions,
};
pub use search::query::{
    NGRAM_CANDIDATES_SQL, QueryPlan, escape_like_literal, normalize_query, plan_metadata_query,
    plan_query,
};
pub use search::rhyme::{
    CHARACTER_RHYME_GROUPS_SQL, CharacterRhymes, RHYME_GROUP_POEMS_SQL, RhymeAnswer, RhymeGroupHit,
    RhymeGroupMatches, RhymeGroupRef, RhymeVerdict, ToneFilter, do_these_rhyme,
    find_by_rhyme_group, rhyme_groups_of,
};
pub use search::text::{
    HighlightRange, HighlightedSnippet, SearchPage, TEXT_SEARCH_HARD_CAP, TextSearchHit,
    TextSearchRequest,
};
pub use search::topic::{
    AuthorRecord, CommentaryCitation, CommentaryEntry, PoemDetail, PoemRecord, Provenance,
    RhymeGroupMembership, TAG_PAGE_LIMIT, TagSummary, Tone, ToneAnnotation, ToneCell, ToneLine,
    browse_by_tag, list_tags, poem_detail,
};
pub use text::{content_chars, is_punctuation};
