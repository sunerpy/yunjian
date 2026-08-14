//! 内置字典的稳定查询契约。

use crate::{CorpusHandle, Error, Result, RhymeBook, RhymeTone, content_chars};
use pinyin::{Pinyin, ToPinyinMulti};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const POYIN_TSV: &str = include_str!("../../../../data/poyin.tsv");
const VARIANT_SQL: &str = "SELECT dst_char FROM variant_map WHERE src_char = ?1";
const REVERSE_VARIANTS_SQL: &str =
    "SELECT src_char FROM variant_map WHERE dst_char = ?1 ORDER BY src_char";
const RHYME_FACTS_SQL: &str = "SELECT rhyme_book, rhyme_group, tone, tone_raw FROM rhyme \
WHERE character = ?1 AND rhyme_book IN ('pingshui', 'cilin') \
ORDER BY rhyme_book, rhyme_group, tone, tone_raw";

/// 字典查询是一字查询还是双字整体查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryQueryKind {
    /// 单字查询。
    Character,
    /// 双字整体提交、逐字返回事实；v1 不合成词义。
    CharacterSequence,
}

/// 可验证的异体字到规范字关系。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantRelation {
    /// 异体或繁体字。
    pub variant: char,
    /// 语料检索使用的规范字。
    pub normalized: char,
}

/// 一条韵书原始事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryRhymeFact {
    /// 韵书。
    pub book: RhymeBook,
    /// 韵部。
    pub rhyme_group: String,
    /// 结构化声调。
    pub tone: RhymeTone,
    /// 韵书数据中的原始声部文本。
    pub tone_raw: String,
    /// 可复核定位符。
    pub source_locator: String,
}

/// `poyin.tsv` 的置信来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoyinConfidence {
    /// 韵部实证。
    RhymeAttested,
    /// 韵书调类分工。
    ToneSplit,
    /// 明确不覆写引擎候选。
    EngineDefault,
}

/// 当前语境命中的破读依据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoyinEvidence {
    /// 有据覆写读音；`None` 表示 `engine_default`。
    pub reading: Option<String>,
    /// 置信来源。
    pub confidence: PoyinConfidence,
    /// 完整依据原文。
    pub evidence: String,
    /// `poyin.tsv` 中的行定位符。
    pub source_locator: String,
}

/// 当前句中的拼音状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DictionaryPronunciation {
    /// `poyin.tsv` 给出的有据破读。
    Attested {
        /// 带调拼音。
        reading: String,
    },
    /// 通用拼音只有一个候选。
    General {
        /// 唯一的通用带调拼音候选。
        reading: String,
    },
    /// 通用拼音有多个候选，当前语境无法裁决。
    Uncertain {
        /// 全部通用带调拼音候选。
        candidates: Vec<String>,
    },
    /// 没有拼音数据。
    Unavailable,
}

/// 一个字的全部 v1 字典事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryCharacter {
    /// 用户提交的原字形。
    pub character: char,
    /// 经 `variant_map` 归一后的字形。
    pub normalized: char,
    /// 与该字相关的全部可验证异体关系。
    pub variants: Vec<VariantRelation>,
    /// 当前句拼音状态。
    pub pronunciation: DictionaryPronunciation,
    /// 当前语境命中的破读依据。
    pub poyin: Option<PoyinEvidence>,
    /// 两部随包韵书里的全部记录。
    pub rhymes: Vec<DictionaryRhymeFact>,
}

/// 内置字典回答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryLookup {
    /// 去掉空白与标点后的原查询。
    pub query: String,
    /// 查询形态。
    pub kind: DictionaryQueryKind,
    /// 按查询顺序排列的逐字事实。
    pub characters: Vec<DictionaryCharacter>,
}

/// 查询一字或双字的内置字典事实。
pub fn lookup_dictionary(
    handle: &CorpusHandle,
    query: &str,
    context: Option<&str>,
) -> Result<DictionaryLookup> {
    let query = content_chars(query).collect::<String>();
    let input = query.chars().collect::<Vec<_>>();
    if !(1..=2).contains(&input.len()) {
        return Err(Error::Search(format!(
            "内置字典只接受一字或双字查询，实际收到 {} 个内容字",
            input.len()
        )));
    }
    let connection = handle.connect()?;
    let mut characters = Vec::with_capacity(input.len());
    for character in input {
        characters.push(character_entry(&connection, character, context)?);
    }
    Ok(DictionaryLookup {
        query,
        kind: if characters.len() == 1 {
            DictionaryQueryKind::Character
        } else {
            DictionaryQueryKind::CharacterSequence
        },
        characters,
    })
}

fn character_entry(
    connection: &Connection,
    character: char,
    context: Option<&str>,
) -> Result<DictionaryCharacter> {
    let normalized = normalized_character(connection, character)?;
    let poyin = poyin_evidence(normalized, context);
    let pronunciation = pronunciation(normalized, poyin.as_ref());
    Ok(DictionaryCharacter {
        character,
        normalized,
        variants: variant_relations(connection, character, normalized)?,
        pronunciation,
        poyin,
        rhymes: rhyme_facts(connection, normalized)?,
    })
}

fn normalized_character(connection: &Connection, character: char) -> Result<char> {
    let mapped = connection
        .prepare_cached(VARIANT_SQL)?
        .query_row([character.to_string()], |row| row.get::<_, String>(0))
        .optional()?;
    match mapped {
        None => Ok(character),
        Some(value) => one_character(&value, "variant_map.dst_char"),
    }
}

fn variant_relations(
    connection: &Connection,
    original: char,
    normalized: char,
) -> Result<Vec<VariantRelation>> {
    let mut relations = Vec::new();
    if original != normalized {
        relations.push(VariantRelation {
            variant: original,
            normalized,
        });
    }
    let mut statement = connection.prepare_cached(REVERSE_VARIANTS_SQL)?;
    let rows = statement
        .query_map([normalized.to_string()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for value in rows {
        let variant = one_character(&value, "variant_map.src_char")?;
        if !relations.iter().any(|relation| relation.variant == variant) {
            relations.push(VariantRelation {
                variant,
                normalized,
            });
        }
    }
    Ok(relations)
}

fn rhyme_facts(connection: &Connection, character: char) -> Result<Vec<DictionaryRhymeFact>> {
    let mut statement = connection.prepare_cached(RHYME_FACTS_SQL)?;
    let rows = statement
        .query_map([character.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(book_key, rhyme_group, tone_key, tone_raw)| {
            let book = RhymeBook::from_key(&book_key)
                .ok_or_else(|| Error::Search(format!("韵书数据里出现未登记键 `{book_key}`")))?;
            let tone = RhymeTone::from_key(&tone_key)
                .ok_or_else(|| Error::Search(format!("韵书数据里出现未登记声调 `{tone_key}`")))?;
            let source_locator =
                format!("corpus.db:rhyme:{book_key}:{rhyme_group}:{tone_key}:{character}");
            Ok(DictionaryRhymeFact {
                book,
                rhyme_group,
                tone,
                tone_raw,
                source_locator,
            })
        })
        .collect()
}

fn pronunciation(character: char, poyin: Option<&PoyinEvidence>) -> DictionaryPronunciation {
    if let Some(reading) = poyin.and_then(|entry| entry.reading.clone()) {
        return DictionaryPronunciation::Attested { reading };
    }
    let mut candidates = character
        .to_pinyin_multi()
        .map(|multi| {
            multi
                .into_iter()
                .map(Pinyin::with_tone)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [] => DictionaryPronunciation::Unavailable,
        [reading] => DictionaryPronunciation::General {
            reading: reading.clone(),
        },
        _ => DictionaryPronunciation::Uncertain { candidates },
    }
}

fn poyin_evidence(character: char, context: Option<&str>) -> Option<PoyinEvidence> {
    let mut fallback = None;
    for (index, line) in POYIN_TSV.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() || line.starts_with('#') || line.starts_with("字\t") {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 5 || !fields[0].starts_with(character) {
            continue;
        }
        let row_context = fields[1];
        if row_context != "*" && !context.is_some_and(|text| text.contains(row_context)) {
            continue;
        }
        let confidence = match fields[4] {
            "rhyme_attested" => PoyinConfidence::RhymeAttested,
            "tone_split" => PoyinConfidence::ToneSplit,
            "engine_default" => PoyinConfidence::EngineDefault,
            _ => continue,
        };
        let evidence = PoyinEvidence {
            reading: (fields[2] != "-").then(|| fields[2].to_owned()),
            confidence,
            evidence: fields[3].to_owned(),
            source_locator: format!("data/poyin.tsv:{line_number}"),
        };
        if row_context == "*" {
            fallback = fallback.or(Some(evidence));
        } else {
            return Some(evidence);
        }
    }
    fallback
}

fn one_character(value: &str, field: &str) -> Result<char> {
    let mut characters = value.chars();
    let Some(character) = characters.next() else {
        return Err(Error::Corpus(format!("{field} 不能为空")));
    };
    if characters.next().is_some() {
        return Err(Error::Corpus(format!(
            "{field} 必须恰好一个字符，实际为 {value:?}"
        )));
    }
    Ok(character)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CorpusConfig, Error, SCHEMA_VERSION};
    use rusqlite::{Connection, params};
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        dir: PathBuf,
        handle: CorpusHandle,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-dictionary-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建字典 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开字典 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path) {
        let connection = Connection::open(path).expect("创建字典 fixture 数据库");
        connection
            .execute_batch(
                "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);
                 CREATE TABLE variant_map(
                     src_char TEXT PRIMARY KEY NOT NULL,
                     dst_char TEXT NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE rhyme(
                     rhyme_book TEXT NOT NULL,
                     rhyme_group TEXT NOT NULL,
                     tone TEXT NOT NULL,
                     tone_raw TEXT NOT NULL,
                     character TEXT NOT NULL,
                     PRIMARY KEY (rhyme_book, rhyme_group, tone, character)
                 ) WITHOUT ROWID;
                 CREATE TABLE corpus_meta(
                     singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
                     schema_version INTEGER NOT NULL,
                     corpus_version TEXT NOT NULL,
                     built_at TEXT NOT NULL,
                     poem_count INTEGER NOT NULL,
                     index_detail_mode TEXT NOT NULL,
                     derived_indexes TEXT NOT NULL,
                     shipped_scope TEXT NOT NULL,
                     integrity_check TEXT NOT NULL
                 );
                 CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character);",
            )
            .expect("创建字典 fixture schema");
        connection
            .execute(
                "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                params!["國", "国"],
            )
            .expect("写异体关系");
        for (book, group, tone, tone_raw, character) in [
            ("pingshui", "七阳", "level", "下平声部", "阳"),
            ("cilin", "第二部", "level", "平声", "阳"),
            ("pingshui", "六麻", "level", "下平声部", "斜"),
            ("cilin", "第三部", "level", "平声", "斜"),
            ("pingshui", "一东", "level", "上平声部", "国"),
        ] {
            connection
                .execute(
                    "INSERT INTO rhyme VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![book, group, tone, tone_raw, character],
                )
                .expect("写韵书行");
        }
        connection
            .execute(
                "INSERT INTO corpus_meta VALUES
                 (1, ?1, 'fixture-v1', '2026-08-14T00:00:00Z', 0, 'full',
                  'first_launch', '10k', 'ok')",
                [SCHEMA_VERSION],
            )
            .expect("写 corpus_meta");
        connection.close().expect("关闭字典 fixture 数据库");
    }

    #[test]
    fn one_character_returns_variants_raw_rhyme_facts_and_locators() {
        let fixture = fixture();
        let answer = lookup_dictionary(&fixture.handle, "國", None).expect("查國");
        assert_eq!(answer.kind, DictionaryQueryKind::Character);
        assert_eq!(answer.characters.len(), 1);
        let entry = &answer.characters[0];
        assert_eq!((entry.character, entry.normalized), ('國', '国'));
        assert_eq!(
            entry.variants,
            [VariantRelation {
                variant: '國',
                normalized: '国'
            }]
        );
        assert!(entry.rhymes.iter().any(|fact| {
            fact.book == RhymeBook::Pingshui
                && fact.rhyme_group == "一东"
                && fact.tone_raw == "上平声部"
                && !fact.source_locator.trim().is_empty()
        }));
    }

    #[test]
    fn two_character_query_is_one_request_but_never_synthesizes_a_definition() {
        let fixture = fixture();
        let answer =
            lookup_dictionary(&fixture.handle, "斜阳", Some("远上寒山石径斜")).expect("查双字");
        assert_eq!(answer.kind, DictionaryQueryKind::CharacterSequence);
        assert_eq!(
            answer
                .characters
                .iter()
                .map(|entry| entry.character)
                .collect::<String>(),
            "斜阳"
        );
        let json = serde_json::to_value(answer).expect("序列化回答");
        let object = json.as_object().expect("回答是对象");
        for forbidden in ["definition", "gloss", "meaning", "translation"] {
            assert!(!object.contains_key(forbidden), "v1 不得生成 {forbidden}");
        }
    }

    #[test]
    fn contextual_poyin_preserves_confidence_complete_evidence_and_location() {
        let fixture = fixture();
        let answer =
            lookup_dictionary(&fixture.handle, "斜", Some("远上寒山石径斜")).expect("查有据破读");
        let entry = &answer.characters[0];
        assert_eq!(
            entry.pronunciation,
            DictionaryPronunciation::Attested {
                reading: "xiá".to_owned()
            }
        );
        let evidence = entry.poyin.as_ref().expect("应命中 poyin.tsv");
        assert_eq!(evidence.confidence, PoyinConfidence::RhymeAttested);
        assert!(evidence.evidence.contains("《平水韵》下平声部 六麻"));
        assert!(
            evidence
                .evidence
                .contains("据 chinese_word_rhyme 锁定版转录本")
        );
        assert!(evidence.source_locator.starts_with("data/poyin.tsv:"));
    }

    #[test]
    fn engine_default_keeps_its_evidence_but_does_not_become_attested() {
        let fixture = fixture();
        let answer = lookup_dictionary(&fixture.handle, "乡", Some("低头思故乡"))
            .expect("查 engine_default");
        let entry = &answer.characters[0];
        assert!(!matches!(
            entry.pronunciation,
            DictionaryPronunciation::Attested { .. }
        ));
        let evidence = entry.poyin.as_ref().expect("处置台账仍须展示");
        assert_eq!(evidence.confidence, PoyinConfidence::EngineDefault);
        assert_eq!(evidence.reading, None);
        assert!(!evidence.evidence.trim().is_empty());
    }

    #[test]
    fn query_rejects_empty_or_more_than_two_content_characters() {
        let fixture = fixture();
        for query in ["，。", "床前明"] {
            let error =
                lookup_dictionary(&fixture.handle, query, None).expect_err("非法长度应失败");
            assert!(matches!(error, Error::Search(_)), "实际错误：{error:?}");
        }
    }

    #[test]
    fn response_shape_has_no_slot_for_modern_dictionary_content() {
        let fixture = fixture();
        let value =
            serde_json::to_value(lookup_dictionary(&fixture.handle, "阳", None).expect("查阳"))
                .expect("序列化");
        fn keys(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        out.push(key.clone());
                        keys(value, out);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        keys(value, out);
                    }
                }
                _ => {}
            }
        }
        let mut all_keys = Vec::new();
        keys(&value, &mut all_keys);
        for forbidden in ["definition", "gloss", "translation", "provider", "remote"] {
            assert!(!all_keys.iter().any(|key| key == forbidden));
        }
    }
}
