use rusqlite::{Connection, params};
use std::collections::BTreeSet;
use yunjian_core::{Error, Result};

pub(crate) fn build_search_indexes(
    connection: &mut Connection,
    detail_mode: &str,
    ngram_aux_enabled: bool,
) -> Result<()> {
    validate_detail_mode(detail_mode)?;
    if !ngram_aux_enabled {
        return Err(corpus_error(
            "索引 verdict 禁用了 n-gram 辅助表，但 schema v1 要求启用实测选定的候选索引",
        ));
    }

    populate_ngrams(connection)?;
    connection.execute_batch(&format!(
        "CREATE VIRTUAL TABLE poem_fts USING fts5(
             body,
             content='poem',
             content_rowid='rowid',
             tokenize='trigram',
             detail={detail_mode}
         );
         INSERT INTO poem_fts(poem_fts) VALUES('rebuild');
         INSERT INTO poem_fts(poem_fts) VALUES('optimize');
         INSERT INTO poem_fts(poem_fts) VALUES('integrity-check');"
    ))?;
    verify_search_indexes(connection, detail_mode, ngram_aux_enabled)
}

pub(crate) fn verify_search_indexes(
    connection: &Connection,
    expected_detail_mode: &str,
    ngram_aux_enabled: bool,
) -> Result<()> {
    validate_detail_mode(expected_detail_mode)?;
    if !ngram_aux_enabled {
        return Err(corpus_error(
            "索引 verdict 禁用了 n-gram 辅助表，但 schema v1 要求启用实测选定的候选索引",
        ));
    }

    let fts_tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema
         WHERE type='table' AND lower(sql) LIKE '%using fts5(%'",
        [],
        |row| row.get(0),
    )?;
    if fts_tables != 1 {
        return Err(corpus_error(format!(
            "语料库必须恰有一个 FTS5 表，实际为 {fts_tables} 个"
        )));
    }

    let ddl: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='poem_fts'",
        [],
        |row| row.get(0),
    )?;
    for required in [
        "content='poem'",
        "content_rowid='rowid'",
        "tokenize='trigram'",
    ] {
        if !ddl.contains(required) {
            return Err(corpus_error(format!(
                "poem_fts DDL 缺少 `{required}`：{ddl}"
            )));
        }
    }
    if ddl.contains("remove_diacritics") {
        return Err(corpus_error(
            "poem_fts 不得启用 remove_diacritics，否则 trigram LIKE/GLOB 无法使用索引",
        ));
    }

    let actual_detail_mode = detail_mode_from_ddl(&ddl)?;
    if actual_detail_mode != expected_detail_mode {
        return Err(corpus_error(format!(
            "索引 verdict 要求 detail={expected_detail_mode}，实际建出 detail={actual_detail_mode}"
        )));
    }

    let metadata_tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='corpus_meta'",
        [],
        |row| row.get(0),
    )?;
    if metadata_tables == 1 {
        let metadata_mode: String = connection.query_row(
            "SELECT index_detail_mode FROM corpus_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if metadata_mode != actual_detail_mode {
            return Err(corpus_error(format!(
                "corpus_meta 记录 detail={metadata_mode}，实际建出 detail={actual_detail_mode}"
            )));
        }
    }

    let shadow_content_tables: i64 = connection.query_row(
        "SELECT count(*) FROM pragma_table_list WHERE name='poem_fts_content'",
        [],
        |row| row.get(0),
    )?;
    if shadow_content_tables != 0 {
        return Err(corpus_error(
            "poem_fts_content 影子内容表不应存在；poem_fts 必须使用 external-content",
        ));
    }

    let ngram_tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='ngram'",
        [],
        |row| row.get(0),
    )?;
    let ngram_indexes: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema
         WHERE type='index' AND name='ngram_gram_idx'
           AND sql LIKE '%ON ngram(gram, stable_id)%'",
        [],
        |row| row.get(0),
    )?;
    if ngram_tables != 1 || ngram_indexes != 1 {
        return Err(corpus_error(format!(
            "n-gram 候选索引不完整：table={ngram_tables}, covering_index={ngram_indexes}"
        )));
    }
    Ok(())
}

fn populate_ngrams(connection: &mut Connection) -> Result<()> {
    let poems = {
        let mut statement =
            connection.prepare("SELECT stable_id, body FROM poem ORDER BY stable_id")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM ngram", [])?;
    {
        let mut insert =
            transaction.prepare("INSERT INTO ngram(gram, stable_id) VALUES (?1, ?2)")?;
        for (stable_id, body) in poems {
            let characters = body
                .chars()
                .filter(|character| !character.is_whitespace() && !is_punctuation(*character))
                .collect::<Vec<_>>();
            let mut grams = BTreeSet::new();
            for (index, character) in characters.iter().enumerate() {
                grams.insert(character.to_string());
                if let Some(next) = characters.get(index + 1) {
                    let mut bigram = String::with_capacity(8);
                    bigram.push(*character);
                    bigram.push(*next);
                    grams.insert(bigram);
                }
            }
            for gram in grams {
                insert.execute(params![gram, stable_id])?;
            }
        }
    }
    transaction.commit()?;
    Ok(())
}

fn validate_detail_mode(detail_mode: &str) -> Result<()> {
    if matches!(detail_mode, "none" | "column" | "full") {
        return Ok(());
    }
    Err(corpus_error(format!(
        "索引 verdict chosen_mode 非法：{detail_mode}"
    )))
}

fn detail_mode_from_ddl(ddl: &str) -> Result<&'static str> {
    ["none", "column", "full"]
        .into_iter()
        .find(|mode| ddl.contains(&format!("detail={mode}")))
        .ok_or_else(|| corpus_error(format!("poem_fts DDL 缺少可识别的 detail 模式：{ddl}")))
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

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

#[cfg(test)]
mod tests;
