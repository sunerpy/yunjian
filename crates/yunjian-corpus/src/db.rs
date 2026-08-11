use crate::commentary::CommentaryRecord;
use crate::form;
use crate::fts;
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
use std::path::{Path, PathBuf};
use yunjian_core::{Error, Result};

/// 兼容范围、只读打开与其错误类型都住在 `yunjian-core`，这里只重导出。
///
/// 为什么不留在本 crate：构建期（这里）与运行期（`yunjian_core::corpus`）必须共用**同一个**
/// 兼容范围，而依赖方向是 corpus -> core，两份常量迟早漂移。运行期还要在落地前用它
/// 预判归档能不能读，那条路径根本到不了本 crate。
pub use yunjian_core::corpus::{OpenCorpusError, SCHEMA_VERSION, SUPPORTED_SCHEMA, open_corpus};

pub const SCHEMA_SQL: &str = include_str!("../schema.sql");
/// 审计库 schema。与 [`SCHEMA_SQL`] 同样是签入的唯一事实来源，本文件内零 DDL 拼接。
pub const AUDIT_SCHEMA_SQL: &str = include_str!("../schema-audit.sql");

/// 随包工件里**永远不该出现**的表。
///
/// 五张都是可再生的：前两张是构建期审计台账（重放上游即可重建），后三张由
/// `poem.body` 确定性派生（首启在本机构建，见 `yunjian_core::derive`）。
/// 打包前逐张断言不存在，见 [`assert_no_diagnostic_tables`]。
pub const NON_SHIPPED_TABLES: [&str; 5] = [
    "defect",
    "disposition",
    "ngram",
    "poem_fts",
    "poem_last_char",
];

/// `corpus_meta.derived_indexes` 的取值。构建器永远写 `first_launch`——三张派生结构
/// 不随包是已实测定案，不是可配置项，所以它是常量而不是参数。
const DERIVED_INDEXES: &str = "first_launch";

/// 随包默认集的范围。语料库自己记住它是哪一档，因为打包要断言「这个库就是被实测
/// 选定的那一档」，而报告只能证明报告自己。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShippedScope {
    Sample10k,
    TangSong,
    Full,
}

impl ShippedScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sample10k => "10k",
            Self::TangSong => "tang-song",
            Self::Full => "full",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "10k" => Ok(Self::Sample10k),
            "tang-song" => Ok(Self::TangSong),
            "full" => Ok(Self::Full),
            other => Err(corpus_error(format!(
                "未知随包范围 `{other}`；可选 10k | tang-song | full"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoemTagRow {
    pub poem_id: String,
    pub tag: String,
}

#[derive(Debug)]
pub struct CorpusDbInput {
    pub corpus_version: String,
    pub shipped_scope: ShippedScope,
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

/// 一次构建过程中只能在构建期观测到的量。
///
/// 为什么要单独回传：`VACUUM` 发生在 [`write_database`] 内部、临时文件重命名之前，
/// 所以「VACUUM 前的文件字节」在构建结束后**已经不存在了**——拿最终产物再 VACUUM
/// 一次只会得到同一个数。todo 20 的预算结论需要这个差值来说明紧凑化省了多少，
/// 于是由构建方在恰当的时刻量一次并回传，而不是让调用方去估。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    /// 全部数据、索引与 FTS 写完、`VACUUM` 之前的文件字节。
    pub bytes_before_vacuum: u64,
    /// `VACUUM` 之后的文件字节，即随包产物的真实大小。
    pub bytes_after_vacuum: u64,
    /// 同一次构建产出的审计库字节。不随包，但要在报告里说明拆出去了多少。
    pub audit_bytes: u64,
}

/// 由随包库路径机械推出审计库路径：`corpus.db` -> `corpus-audit.db`。
///
/// 刻意不让调用方各传一个路径：能被任意指定的第二路径会让「跨文件校验的是这一对吗」
/// 无法回答——拿旧审计库配新语料库同样能让守恒等式凑巧成立。
#[must_use]
pub fn audit_path(corpus_path: impl AsRef<Path>) -> PathBuf {
    let path = corpus_path.as_ref();
    let stem = path
        .file_stem()
        .map_or_else(|| "corpus".into(), |stem| stem.to_string_lossy());
    let extension = path
        .extension()
        .map_or_else(|| "db".into(), |extension| extension.to_string_lossy());
    path.with_file_name(format!("{stem}-audit.{extension}"))
}

pub fn build_database(path: impl AsRef<Path>, input: &CorpusDbInput) -> Result<()> {
    build_database_with_stats(path, input).map(|_| ())
}

/// 与 [`build_database`] 完全相同的构建路径，额外回传 [`BuildStats`]。
///
/// 一次调用产出**两个文件**：`path` 是随包语料库，[`audit_path`] 推出的那个是审计库。
/// 顺序是「写随包库 -> 写审计库 -> 跨文件守恒校验 -> 两个临时文件一起就位」：校验
/// 放在就位**之前**，所以未通过守恒的一对永远不会被发布出去。
pub fn build_database_with_stats(
    path: impl AsRef<Path>,
    input: &CorpusDbInput,
) -> Result<BuildStats> {
    let path = path.as_ref();
    let audit = audit_path(path);
    let metadata = validate_input(input)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let corpus_temporary = temporary_path(path);
    let audit_temporary = temporary_path(&audit);
    for temporary in [&corpus_temporary, &audit_temporary] {
        if temporary.exists() {
            std::fs::remove_file(temporary)?;
        }
    }

    let discard = || {
        let _ = std::fs::remove_file(&corpus_temporary);
        let _ = std::fs::remove_file(&audit_temporary);
    };
    let mut stats = match write_database(&corpus_temporary, input, &metadata) {
        Ok(stats) => stats,
        Err(error) => {
            discard();
            return Err(error);
        }
    };
    stats.audit_bytes = match write_audit_database(&audit_temporary, input, &metadata) {
        Ok(bytes) => bytes,
        Err(error) => {
            discard();
            return Err(error);
        }
    };
    if let Err(error) = verify_conservation_across_files(&corpus_temporary, &audit_temporary) {
        discard();
        return Err(error);
    }

    for (temporary, destination) in [(&corpus_temporary, path), (&audit_temporary, &audit)] {
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }
        std::fs::rename(temporary, destination)?;
    }
    Ok(stats)
}

/// 断言随包工件不含任何构建期诊断表或首启派生表。
///
/// 打包前的中止断言之一。看的是 `sqlite_schema` 而不是元数据列：元数据可以写错，
/// 表在不在是唯一无法自我声明的事实。
pub fn assert_no_diagnostic_tables(connection: &Connection) -> Result<()> {
    let mut present = Vec::new();
    for table in NON_SHIPPED_TABLES {
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
            params![table],
            |row| row.get(0),
        )?;
        if count != 0 {
            present.push(table);
        }
    }
    if present.is_empty() {
        return Ok(());
    }
    Err(corpus_error(format!(
        "随包工件含不该随包的表 {}；`defect`/`disposition` 属审计库，`ngram` 首启本机构建",
        present.join("、")
    )))
}

/// 跨两个文件校验处置守恒。
///
/// 三条等式与拆库前**逐字相同**，只是左端在审计库、右端在随包库的 `corpus_meta`：
///
/// | 等式 | 左端 | 右端 |
/// |---|---|---|
/// | `count(disposition)` == `input_row_count` | 审计库 | 语料库 |
/// | `count(disposition WHERE 'shipped')` == `poem_count` | 审计库 | 语料库 |
/// | `count(defect)` == `finding_count` | 审计库 | 语料库 |
///
/// 外加一条拆库**新引入**风险的对策：两个文件必须自称属于同一次构建（`schema_version`
/// / `corpus_version` / `source_manifest_sha256` 三元组相等）。否则拿一份旧审计库配
/// 一份新语料库，上面三条也可能凑巧成立。
pub fn verify_conservation_across_files(
    corpus_path: impl AsRef<Path>,
    audit_path: impl AsRef<Path>,
) -> Result<()> {
    let corpus = Connection::open_with_flags(
        corpus_path.as_ref(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let audit = Connection::open_with_flags(
        audit_path.as_ref(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;

    let (schema_version, corpus_version, manifest, poem_count, finding_count, input_row_count): (
        u32,
        String,
        String,
        i64,
        i64,
        i64,
    ) = corpus.query_row(
        "SELECT schema_version, corpus_version, source_manifest_sha256, \
                poem_count, finding_count, input_row_count FROM corpus_meta",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    let (
        audit_schema_version,
        audit_corpus_version,
        audit_manifest,
        audit_poem_count,
        audit_finding_count,
        audit_input_row_count,
    ): (u32, String, String, i64, i64, i64) = audit.query_row(
        "SELECT schema_version, corpus_version, source_manifest_sha256, \
                poem_count, finding_count, input_row_count FROM audit_meta",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;

    if (schema_version, &corpus_version, &manifest)
        != (audit_schema_version, &audit_corpus_version, &audit_manifest)
    {
        return Err(corpus_error(format!(
            "审计库与语料库不属于同一次构建：语料库 v{schema_version}/{corpus_version}/\
             {manifest} vs 审计库 v{audit_schema_version}/{audit_corpus_version}/{audit_manifest}"
        )));
    }
    if (audit_poem_count, audit_finding_count, audit_input_row_count)
        != (poem_count, finding_count, input_row_count)
    {
        return Err(corpus_error(format!(
            "审计库记录的计数与语料库不符：审计库 poem={audit_poem_count} \
             finding={audit_finding_count} input={audit_input_row_count} vs \
             语料库 poem={poem_count} finding={finding_count} input={input_row_count}"
        )));
    }

    let disposition_count = audit.query_row("SELECT count(*) FROM disposition", [], |row| {
        row.get::<_, i64>(0)
    })?;
    let shipped_count = audit.query_row(
        "SELECT count(*) FROM disposition WHERE disposition='shipped'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let defect_count = audit.query_row("SELECT count(*) FROM defect", [], |row| {
        row.get::<_, i64>(0)
    })?;
    let poem_rows =
        corpus.query_row("SELECT count(*) FROM poem", [], |row| row.get::<_, i64>(0))?;

    if disposition_count != input_row_count
        || shipped_count != poem_count
        || defect_count != finding_count
        || poem_rows != poem_count
    {
        return Err(corpus_error(format!(
            "跨文件处置守恒失败：disposition={disposition_count}, input={input_row_count}, \
             shipped={shipped_count}, poem_meta={poem_count}, poem_rows={poem_rows}, \
             defect={defect_count}, finding={finding_count}"
        )));
    }
    Ok(())
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

fn write_database(
    path: &Path,
    input: &CorpusDbInput,
    metadata: &BuildMetadata,
) -> Result<BuildStats> {
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
        let classification = form::classify(record)?;
        let normalized = normalized_by_id
            .get(record.stable_id.as_str())
            .ok_or_else(|| corpus_error(format!("缺少归一记录：{}", record.stable_id)))?;
        let last_chars = yunjian_core::split_rhyme_feet(&normalized.body)
            .filter_map(last_character)
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
                stable_id, content_hash, source_locator, source_locator_kind, genre, form, is_yuefu, title, \
                title_raw, ci_tune, author, dynasty, dynasty_raw, body, body_original, script, \
                first_line, last_chars, line_count, char_count, provenance_source, \
                provenance_revision, provenance_kind, provenance_license, \
                provenance_license_class, work_group, edition_group\
             ) VALUES (\
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27\
             )",
            params![
                record.stable_id,
                record.content_hash,
                record.source_locator,
                source_locator_kind(record.source_locator_kind),
                genre(record.genre),
                classification.form.as_str(),
                i64::from(classification.is_yuefu),
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

    transaction.execute(
        "INSERT INTO corpus_meta(\
            singleton, schema_version, corpus_version, built_at, source_manifest_sha256, \
            poem_count, finding_count, input_row_count, index_detail_mode, \
            derived_indexes, shipped_scope, builder_sqlite_version, integrity_check\
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'ok')",
        params![
            SCHEMA_VERSION,
            input.corpus_version,
            metadata.built_at,
            metadata.source_manifest_sha256,
            input.quality.poem_count as i64,
            input.quality.findings.len() as i64,
            input.quality.input_rows as i64,
            metadata.index_detail_mode,
            DERIVED_INDEXES,
            input.shipped_scope.as_str(),
            builder_sqlite_version,
        ],
    )?;
    transaction.commit()?;

    // 刻意不在这里建任何检索结构：`ngram` / `poem_fts` / `poem_last_char` 全部由
    // 首启在本机派生（`yunjian_core::derive`）。裁决选定的形态已经写进
    // `corpus_meta.index_detail_mode`，首启照它建。
    fts::reject_disabled_ngram_aux(metadata.ngram_aux_enabled)?;
    verify_shipped_shape(&connection, input)?;
    let bytes_before_vacuum = file_bytes(path)?;
    connection.execute_batch("VACUUM")?;
    let bytes_after_vacuum = file_bytes(path)?;
    let integrity =
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
    if integrity != "ok" {
        return Err(corpus_error(format!(
            "SQLite integrity_check 失败：{integrity}"
        )));
    }
    verify_query_only(&connection)?;
    Ok(BuildStats {
        bytes_before_vacuum,
        bytes_after_vacuum,
        audit_bytes: 0,
    })
}

fn write_audit_database(
    path: &Path,
    input: &CorpusDbInput,
    metadata: &BuildMetadata,
) -> Result<u64> {
    let mut connection = Connection::open(path)?;
    connection.execute_batch(AUDIT_SCHEMA_SQL)?;
    let transaction = connection.transaction()?;

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
        "INSERT INTO audit_meta(\
            singleton, schema_version, corpus_version, source_manifest_sha256, \
            poem_count, finding_count, input_row_count\
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            SCHEMA_VERSION,
            input.corpus_version,
            metadata.source_manifest_sha256,
            input.quality.poem_count as i64,
            input.quality.findings.len() as i64,
            input.quality.input_rows as i64,
        ],
    )?;
    transaction.commit()?;

    connection.execute_batch("VACUUM")?;
    let integrity =
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
    if integrity != "ok" {
        return Err(corpus_error(format!(
            "审计库 SQLite integrity_check 失败：{integrity}"
        )));
    }
    file_bytes(path)
}

fn file_bytes(path: &Path) -> Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// 随包库自己能校验的那一半：行数与元数据一致，且不含任何不该随包的表。
///
/// 另一半（处置台账的三条等式）两端分处两个文件，只能等审计库也写完，
/// 见 [`verify_conservation_across_files`]。
fn verify_shipped_shape(connection: &Connection, input: &CorpusDbInput) -> Result<()> {
    assert_no_diagnostic_tables(connection)?;
    let (poem_count, scope, source): (i64, String, String) = connection.query_row(
        "SELECT poem_count, shipped_scope, derived_indexes FROM corpus_meta",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let poem_rows =
        connection.query_row("SELECT count(*) FROM poem", [], |row| row.get::<_, i64>(0))?;
    if poem_rows != poem_count {
        return Err(corpus_error(format!(
            "随包库守恒失败：poem 行数 {poem_rows} 与 corpus_meta.poem_count {poem_count} 不符"
        )));
    }
    if scope != input.shipped_scope.as_str() || source != DERIVED_INDEXES {
        return Err(corpus_error(format!(
            "随包库元数据与构建请求不符：scope={scope} derived_indexes={source}"
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
