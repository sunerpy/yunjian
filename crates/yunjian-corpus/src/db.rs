use crate::commentary::CommentaryRecord;
use crate::fts::build_search_indexes;
use crate::model::{
    CanonicalRecord, Genre, LicenseClass, ProvenanceKind, Script, SourceLocatorKind,
};
use crate::normalize::{NormalizedRecord, VariantRow};
use crate::quality::{Disposition, QualityReport};
use crate::rhyme::RhymeEntry;
pub use crate::rhyme_foot::PoemRhymeGroupRow;
use rusqlite::{Connection, OpenFlags, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use yunjian_core::{Error, Result};

pub const SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_SCHEMA: RangeInclusive<u32> = 1..=1;
pub const SCHEMA_SQL: &str = include_str!("../schema.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoemTagRow {
    pub poem_id: String,
    pub tag: String,
}

#[derive(Debug)]
pub struct CorpusDbInput {
    pub corpus_version: String,
    pub source_manifest: Vec<u8>,
    pub index_verdict: Vec<u8>,
    pub records: Vec<CanonicalRecord>,
    pub normalized_records: Vec<NormalizedRecord>,
    pub commentaries: Vec<CommentaryRecord>,
    pub rhymes: Vec<RhymeEntry>,
    pub poem_rhyme_groups: Vec<PoemRhymeGroupRow>,
    pub variants: Vec<VariantRow>,
    pub tags: Vec<PoemTagRow>,
    pub quality: QualityReport,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenCorpusError {
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("语料库元数据错误：{0}")]
    InvalidMetadata(String),
    #[error(
        "语料库 schema 版本 {corpus_schema_version} 与应用 {app_version} 不兼容；应用支持 {supported_min}..={supported_max}。请运行 `yunjian corpus fetch` 获取兼容语料库"
    )]
    IncompatibleSchema {
        corpus_schema_version: u32,
        app_version: &'static str,
        supported_min: u32,
        supported_max: u32,
    },
}

#[derive(Deserialize)]
struct SourceManifest {
    #[serde(default)]
    source: Vec<SourceEntry>,
}

#[derive(Deserialize)]
struct SourceEntry {
    retrieved_at: String,
}

#[derive(Deserialize)]
struct IndexVerdict {
    schema_version: u32,
    chosen_mode: String,
    ngram_aux_enabled: bool,
    environment: IndexEnvironment,
}

#[derive(Deserialize)]
struct IndexEnvironment {
    page_size: i64,
}

struct BuildMetadata {
    built_at: String,
    source_manifest_sha256: String,
    index_detail_mode: String,
    ngram_aux_enabled: bool,
}

pub fn build_database(path: impl AsRef<Path>, input: &CorpusDbInput) -> Result<()> {
    let path = path.as_ref();
    let metadata = validate_input(input)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(path);
    if temporary.exists() {
        std::fs::remove_file(&temporary)?;
    }
    let result = write_database(&temporary, input, &metadata);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&temporary, path)?;
    Ok(())
}

pub fn open_corpus(path: impl AsRef<Path>) -> std::result::Result<Connection, OpenCorpusError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (rows, schema_version): (i64, Option<u32>) = connection.query_row(
        "SELECT count(*), min(schema_version) FROM corpus_meta",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if rows != 1 {
        return Err(OpenCorpusError::InvalidMetadata(format!(
            "corpus_meta 必须恰有一行，实际为 {rows} 行"
        )));
    }
    let schema_version = schema_version.ok_or_else(|| {
        OpenCorpusError::InvalidMetadata("corpus_meta.schema_version 不能为空".to_owned())
    })?;
    if !SUPPORTED_SCHEMA.contains(&schema_version) {
        return Err(OpenCorpusError::IncompatibleSchema {
            corpus_schema_version: schema_version,
            app_version: env!("CARGO_PKG_VERSION"),
            supported_min: *SUPPORTED_SCHEMA.start(),
            supported_max: *SUPPORTED_SCHEMA.end(),
        });
    }
    connection.pragma_update(None, "query_only", true)?;
    Ok(connection)
}

pub fn schema_statements(schema: &str) -> Result<Vec<String>> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(schema)?;
    let mut statement = connection.prepare(
        "SELECT sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
         ORDER BY type, name",
    )?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Error::from)
}

fn validate_input(input: &CorpusDbInput) -> Result<BuildMetadata> {
    validate_semver(&input.corpus_version)?;
    input.quality.check_conservation()?;
    input.quality.check_cross_report_integrity()?;
    if input.quality.poem_count != input.records.len() {
        return Err(corpus_error(format!(
            "守恒失败：poem_count {} 与待写入 poem 行数 {} 不符",
            input.quality.poem_count,
            input.records.len()
        )));
    }

    let record_ids = input
        .records
        .iter()
        .map(|record| record.stable_id.as_str())
        .collect::<BTreeSet<_>>();
    if record_ids.len() != input.records.len() {
        return Err(corpus_error("待写入 poem 含重复 stable_id"));
    }
    let shipped_ids = input
        .quality
        .dispositions
        .iter()
        .filter(|row| row.disposition == Disposition::Shipped)
        .map(|row| {
            row.stable_id.as_deref().ok_or_else(|| {
                corpus_error(format!(
                    "shipped 处置 {} 缺少 stable_id",
                    row.source_locator
                ))
            })
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if shipped_ids != record_ids {
        return Err(corpus_error(
            "守恒失败：shipped 处置的 stable_id 集合与 poem 集合不一致",
        ));
    }

    let normalized_ids = input
        .normalized_records
        .iter()
        .map(|record| record.stable_id.as_str())
        .collect::<BTreeSet<_>>();
    if normalized_ids.len() != input.normalized_records.len() || normalized_ids != record_ids {
        return Err(corpus_error(
            "归一记录必须与 poem 按 stable_id 一一对应，且不得重复",
        ));
    }

    for record in &input.records {
        if record.provenance.license_class == LicenseClass::Restricted {
            return Err(corpus_error(format!(
                "受限记录 {} 不得写入可分发 poem 表",
                record.stable_id
            )));
        }
    }

    let manifest_text = std::str::from_utf8(&input.source_manifest)
        .map_err(|error| corpus_error(format!("source manifest 不是 UTF-8：{error}")))?;
    let manifest: SourceManifest = toml::from_str(manifest_text)
        .map_err(|error| corpus_error(format!("解析 source manifest 失败：{error}")))?;
    let latest_date = manifest
        .source
        .iter()
        .map(|source| source.retrieved_at.as_str())
        .max()
        .ok_or_else(|| corpus_error("source manifest 没有 source 条目"))?;
    validate_date(latest_date)?;

    let verdict: IndexVerdict = serde_json::from_slice(&input.index_verdict)
        .map_err(|error| corpus_error(format!("解析索引 verdict 失败：{error}")))?;
    if verdict.schema_version != 1 {
        return Err(corpus_error(format!(
            "索引 verdict schema_version {} 不受支持",
            verdict.schema_version
        )));
    }
    if !matches!(verdict.chosen_mode.as_str(), "none" | "column" | "full") {
        return Err(corpus_error(format!(
            "索引 verdict chosen_mode 非法：{}",
            verdict.chosen_mode
        )));
    }
    if !verdict.ngram_aux_enabled {
        return Err(corpus_error(
            "索引 verdict 禁用了 n-gram 辅助表，但 schema v1 要求启用实测选定的候选索引",
        ));
    }
    if verdict.environment.page_size != 4096 {
        return Err(corpus_error(format!(
            "索引 verdict page_size {} 与 schema 固定值 4096 不符",
            verdict.environment.page_size
        )));
    }

    Ok(BuildMetadata {
        built_at: format!("{latest_date}T00:00:00Z"),
        source_manifest_sha256: Sha256::digest(&input.source_manifest)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        index_detail_mode: verdict.chosen_mode,
        ngram_aux_enabled: verdict.ngram_aux_enabled,
    })
}

fn write_database(path: &Path, input: &CorpusDbInput, metadata: &BuildMetadata) -> Result<()> {
    let mut connection = Connection::open(path)?;
    connection.execute_batch(SCHEMA_SQL)?;
    let builder_sqlite_version =
        connection.query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))?;
    let transaction = connection.transaction()?;

    let normalized_by_id = input
        .normalized_records
        .iter()
        .map(|record| (record.stable_id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let authors = input
        .records
        .iter()
        .map(|record| record.author.as_str())
        .collect::<BTreeSet<_>>();
    for author in authors {
        transaction.execute("INSERT INTO author(name) VALUES (?1)", params![author])?;
    }

    let mut records = input.records.iter().collect::<Vec<_>>();
    records.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    for record in records {
        let normalized = normalized_by_id
            .get(record.stable_id.as_str())
            .ok_or_else(|| corpus_error(format!("缺少归一记录：{}", record.stable_id)))?;
        let last_chars = normalized
            .body_lines
            .iter()
            .filter_map(|line| last_character(line))
            .map(|character| character.to_string())
            .collect::<Vec<_>>();
        let last_chars_json = serde_json::to_string(&last_chars)
            .map_err(|error| corpus_error(format!("序列化 last_chars 失败：{error}")))?;
        let first_line = normalized.body_lines.first().cloned().unwrap_or_default();
        let char_count = normalized
            .body
            .chars()
            .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
            .count();
        transaction.execute(
            "INSERT INTO poem(\
                stable_id, content_hash, source_locator, source_locator_kind, genre, title, \
                title_raw, ci_tune, author, dynasty, dynasty_raw, body, body_original, script, \
                first_line, last_chars, line_count, char_count, provenance_source, \
                provenance_revision, provenance_kind, provenance_license, \
                provenance_license_class, work_group, edition_group\
             ) VALUES (\
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25\
             )",
            params![
                record.stable_id,
                record.content_hash,
                record.source_locator,
                source_locator_kind(record.source_locator_kind),
                genre(record.genre),
                record.title,
                record.title_raw,
                record.ci_tune,
                record.author,
                record.dynasty.as_key(),
                record.dynasty_raw,
                normalized.body,
                normalized.body_original,
                script(normalized.script),
                first_line,
                last_chars_json,
                normalized.body_lines.len() as i64,
                char_count as i64,
                record.provenance.source_name,
                record.provenance.source_rev,
                provenance_kind(record.provenance.kind),
                record.provenance.license,
                license_class(record.provenance.license_class),
                record.work_group,
                record.edition_group,
            ],
        )?;
        for (line_index, character) in normalized
            .body_lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| last_character(line).map(|character| (index, character)))
        {
            transaction.execute(
                "INSERT INTO poem_last_char(poem_id, line_index, ch) VALUES (?1, ?2, ?3)",
                params![record.stable_id, line_index as i64, character.to_string()],
            )?;
        }
    }

    let mut commentaries = input.commentaries.iter().collect::<Vec<_>>();
    commentaries.sort_by(|left, right| left.id.cmp(&right.id));
    for commentary in commentaries {
        transaction.execute(
            "INSERT INTO commentary(\
                id, poem_id, text, citation_work, citation_author, citation_dynasty, \
                citation_dynasty_raw, citation_work_completed_by, citation_source_note\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                commentary.id,
                commentary.poem_id,
                commentary.text,
                commentary.citation.work,
                commentary.citation.author,
                commentary.citation.dynasty.as_key(),
                commentary.citation.dynasty_raw,
                commentary.citation.work_completed_by,
                commentary.citation.source_note,
            ],
        )?;
    }

    let mut rhymes = input.rhymes.clone();
    rhymes.sort();
    rhymes.dedup();
    for rhyme in rhymes {
        transaction.execute(
            "INSERT INTO rhyme(rhyme_book, rhyme_group, tone, tone_raw, character) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rhyme.book.as_key(),
                rhyme.rhyme_group,
                rhyme.tone.as_key(),
                rhyme.tone_raw,
                rhyme.character,
            ],
        )?;
    }

    let mut poem_rhyme_groups = input.poem_rhyme_groups.iter().collect::<Vec<_>>();
    poem_rhyme_groups.sort_by(|left, right| {
        (&left.poem_id, left.rhyme_book, &left.rhyme_group, left.tone).cmp(&(
            &right.poem_id,
            right.rhyme_book,
            &right.rhyme_group,
            right.tone,
        ))
    });
    for row in poem_rhyme_groups {
        transaction.execute(
            "INSERT INTO poem_rhyme_group(\
                poem_id, rhyme_book, rhyme_group, tone, confidence\
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                row.poem_id,
                row.rhyme_book.as_key(),
                row.rhyme_group,
                row.tone.as_key(),
                row.confidence.as_str(),
            ],
        )?;
    }

    let mut variants = input.variants.clone();
    variants.sort();
    variants.dedup();
    for row in variants {
        transaction.execute(
            "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
            params![row.src_char.to_string(), row.dst_char.to_string()],
        )?;
    }

    let tag_names = input
        .tags
        .iter()
        .map(|row| row.tag.as_str())
        .collect::<BTreeSet<_>>();
    for tag in tag_names {
        transaction.execute("INSERT INTO tag(name) VALUES (?1)", params![tag])?;
    }
    let mut tags = input.tags.iter().collect::<Vec<_>>();
    tags.sort_by(|left, right| (&left.poem_id, &left.tag).cmp(&(&right.poem_id, &right.tag)));
    for row in tags {
        transaction.execute(
            "INSERT INTO poem_tag(poem_id, tag) VALUES (?1, ?2)",
            params![row.poem_id, row.tag],
        )?;
    }

    let mut findings = input.quality.findings.iter().collect::<Vec<_>>();
    findings.sort();
    for (index, finding) in findings.into_iter().enumerate() {
        transaction.execute(
            "INSERT INTO defect(id, stable_id, work_group, reason_code, detail, source) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                (index + 1) as i64,
                finding.stable_id,
                finding.work_group,
                finding.reason_code.as_str(),
                finding.detail,
                finding.source,
            ],
        )?;
    }

    let mut dispositions = input.quality.dispositions.iter().collect::<Vec<_>>();
    dispositions.sort();
    for row in dispositions {
        transaction.execute(
            "INSERT INTO disposition(source_locator, stable_id, disposition) \
             VALUES (?1, ?2, ?3)",
            params![
                row.source_locator,
                row.stable_id,
                disposition(row.disposition),
            ],
        )?;
    }

    transaction.execute(
        "INSERT INTO corpus_meta(\
            singleton, schema_version, corpus_version, built_at, source_manifest_sha256, \
            poem_count, finding_count, input_row_count, index_detail_mode, \
            builder_sqlite_version, integrity_check\
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ok')",
        params![
            SCHEMA_VERSION,
            input.corpus_version,
            metadata.built_at,
            metadata.source_manifest_sha256,
            input.quality.poem_count as i64,
            input.quality.findings.len() as i64,
            input.quality.input_rows as i64,
            metadata.index_detail_mode,
            builder_sqlite_version,
        ],
    )?;
    transaction.commit()?;

    build_search_indexes(
        &mut connection,
        &metadata.index_detail_mode,
        metadata.ngram_aux_enabled,
    )?;
    verify_database_conservation(&connection)?;
    connection.execute_batch("VACUUM")?;
    let integrity =
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
    if integrity != "ok" {
        return Err(corpus_error(format!(
            "SQLite integrity_check 失败：{integrity}"
        )));
    }
    verify_query_only(&connection)?;
    Ok(())
}

fn verify_database_conservation(connection: &Connection) -> Result<()> {
    let (poem_count, input_row_count): (i64, i64) = connection.query_row(
        "SELECT poem_count, input_row_count FROM corpus_meta",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let disposition_count =
        connection.query_row("SELECT count(*) FROM disposition", [], |row| {
            row.get::<_, i64>(0)
        })?;
    let shipped_count = connection.query_row(
        "SELECT count(*) FROM disposition WHERE disposition='shipped'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if disposition_count != input_row_count || shipped_count != poem_count {
        return Err(corpus_error(format!(
            "数据库处置守恒失败：disposition={disposition_count}, input={input_row_count}, shipped={shipped_count}, poem={poem_count}"
        )));
    }
    Ok(())
}

fn verify_query_only(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "query_only", true)?;
    if connection
        .execute("INSERT INTO tag(name) VALUES ('query-only-probe')", [])
        .is_ok()
    {
        return Err(corpus_error("PRAGMA query_only 未阻止 INSERT"));
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "corpus.db".into(), |name| name.to_string_lossy());
    path.with_file_name(format!("{name}.tmp"))
}

fn validate_semver(version: &str) -> Result<()> {
    let core = version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core);
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u64>().is_err())
    {
        return Err(corpus_error(format!(
            "corpus_version 必须是 semver，收到 `{version}`"
        )));
    }
    Ok(())
}

fn validate_date(date: &str) -> Result<()> {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(corpus_error(format!(
            "source manifest retrieved_at 必须是 YYYY-MM-DD，收到 `{date}`"
        )));
    }
    Ok(())
}

fn last_character(line: &str) -> Option<char> {
    line.chars()
        .rev()
        .find(|character| !character.is_whitespace() && !is_punctuation(*character))
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '《'
                | '》'
                | '〈'
                | '〉'
                | '【'
                | '】'
                | '〔'
                | '〕'
                | '—'
                | '…'
                | '·'
        )
}

const fn source_locator_kind(kind: SourceLocatorKind) -> &'static str {
    match kind {
        SourceLocatorKind::Native => "native",
        SourceLocatorKind::Positional => "positional",
    }
}

const fn genre(value: Genre) -> &'static str {
    match value {
        Genre::Shi => "shi",
        Genre::Ci => "ci",
        Genre::Qu => "qu",
        Genre::Fu => "fu",
        Genre::Wen => "wen",
    }
}

const fn script(value: Script) -> &'static str {
    match value {
        Script::Simplified => "simplified",
        Script::Traditional => "traditional",
        Script::Mixed => "mixed",
    }
}

const fn provenance_kind(value: ProvenanceKind) -> &'static str {
    match value {
        ProvenanceKind::Original => "原文",
        ProvenanceKind::PublicDomainCommentary => "集评-PD",
        ProvenanceKind::Ai => "AI",
    }
}

const fn license_class(value: LicenseClass) -> &'static str {
    match value {
        LicenseClass::PublicDomain => "public_domain",
        LicenseClass::Permissive => "permissive",
        LicenseClass::Restricted => "restricted",
    }
}

const fn disposition(value: Disposition) -> &'static str {
    value.as_str()
}

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

#[cfg(test)]
mod tests;
